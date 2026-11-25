//! Offline contracts for a completely synthetic, two-table directory.
mod common;

use common::{assert_tree_equal, run_powerbi, stdout_json};
use serde_json::{Value, json};
use std::{fs, path::Path};

const MODEL: &str = "examples/directory.schema.json";
const SPEC: &str = "examples/directory.dashboard.v2.json";

fn read(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn write(path: &Path, doc: &Value) {
    fs::write(path, serde_json::to_vec_pretty(doc).unwrap()).unwrap();
}

fn ok(args: &[&str]) -> Value {
    let out = run_powerbi(args);
    assert_eq!(out.code, 0, "{}", out.stderr);
    stdout_json(&out)
}

fn build(spec: &str, output: &Path) -> Value {
    ok(&[
        "report",
        "build",
        "--schema",
        MODEL,
        "--spec",
        spec,
        "--out-dir",
        output.to_str().unwrap(),
        "--json",
    ])
}

fn visual(project: &Path, page: &str, visual: &str) -> Value {
    read(project.join(format!("Directory.Report/definition/pages/ReportSection{page}/visuals/VisualContainer{visual}/visual.json")))
}

#[test]
fn searchable_directory_is_page_local_and_reuses_replay_for_relationship_and_model_sort() {
    let tmp = tempfile::tempdir().unwrap();
    let directory = tmp.path().join("directory");
    let validate = ok(&[
        "report", "spec", "validate", "--schema", MODEL, "--spec", SPEC, "--json",
    ]);
    assert_eq!(validate["ok"], true);
    assert_eq!(validate["compiled"]["counts"]["relationships"], 1);
    let explain = ok(&[
        "report", "spec", "explain", "--schema", MODEL, "--spec", SPEC, "--json",
    ]);
    let annotated = explain["plan"]["ops"].as_array().unwrap();
    let mut model_ops = vec![];
    let mut search_count = 0;
    for entry in annotated {
        match entry["op"].as_str().unwrap() {
            "addRelationship" | "setSortBy" => {
                assert_eq!(entry["stageName"], "model");
                assert!(entry["pointer"].as_str().unwrap().starts_with("/model/"));
                model_ops.push(entry["operation"].clone());
            }
            "addVisual" if entry["operation"]["searchEnabled"] == true => {
                assert_eq!(entry["operation"]["singleSelect"], false);
                assert_eq!(entry["operation"]["page"], "page:ReportSectionDirectory");
                search_count += 1;
            }
            _ => {}
        }
    }
    assert_eq!(model_ops.len(), 2);
    assert_eq!(search_count, 2);
    let preview = ok(&[
        "report",
        "build",
        "--schema",
        MODEL,
        "--spec",
        SPEC,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(preview["dryRun"], true);
    assert!(!directory.exists());
    let authored = build(SPEC, &directory);
    assert_eq!(authored["validation"]["ok"], true);
    assert_eq!(authored["compiled"]["counts"]["relationships"], 1);
    for name in ["Branch", "Company"] {
        let slicer = visual(&directory, "Directory", &format!("Slicer{name}"));
        assert_eq!(slicer["visual"]["visualType"], "slicer");
        let objects = &slicer["visual"]["objects"];
        assert_eq!(
            objects
                .pointer("/data/0/properties/mode/expr/Literal/Value")
                .unwrap(),
            "'Dropdown'"
        );
        assert_eq!(
            objects
                .pointer("/general/0/properties/selfFilterEnabled/expr/Literal/Value")
                .unwrap(),
            "true"
        );
        assert_eq!(
            slicer["visual"]["query"]["queryState"]["Values"]["projections"][0]["queryRef"],
            format!("{name}.Label")
        );
        // False uses the native default: there is no single-select restriction, saved
        // term/value, or sync group. This is an artifact test, not a live UI assertion.
        assert!(
            objects
                .pointer("/selection/0/properties/singleSelect")
                .is_none()
        );
        assert!(objects.pointer("/general/0/properties/filter").is_none());
        assert!(
            objects
                .pointer("/general/0/properties/selfFilter")
                .is_none()
        );
        assert!(slicer.pointer("/visual/syncGroup").is_none());
    }
    let overview =
        directory.join("Directory.Report/definition/pages/ReportSectionOverview/visuals");
    assert_eq!(fs::read_dir(&overview).unwrap().count(), 1);
    assert_eq!(
        visual(&directory, "Overview", "Total")["visual"]["visualType"],
        "card"
    );
    let slicers = ok(&[
        "report",
        "slicers",
        "list",
        "--project",
        directory.to_str().unwrap(),
        "--json",
    ]);
    for s in slicers["slicers"].as_array().unwrap() {
        assert_eq!(s["state"]["searchEnabled"], true);
        assert_eq!(s["state"]["hasSelectionState"], false);
        assert_eq!(s["safety"]["mayContainDataValues"], false);
    }
    let checked = ok(&[
        "validate",
        directory.to_str().unwrap(),
        "--strict",
        "--json",
    ]);
    assert_eq!(checked["ok"], true);
    let handoff = ok(&["handoff", "check", directory.to_str().unwrap(), "--json"]);
    assert_eq!(handoff["ok"], true);
    let definition = "Directory.SemanticModel/definition";
    let rel = fs::read_to_string(directory.join(definition).join("relationships.tmdl")).unwrap();
    for piece in [
        "fromColumn: 'Company'.'BranchID'",
        "toColumn: 'Branch'.'ID'",
        "crossFilteringBehavior: oneDirection",
    ] {
        assert!(rel.contains(piece), "{piece}: {rel}");
    }
    let table = fs::read_to_string(directory.join(definition).join("tables/Branch.tmdl")).unwrap();
    assert!(table.contains("sortByColumn: SortOrder"));
    let mut without_model = read(SPEC);
    without_model.as_object_mut().unwrap().remove("model");
    let base_spec = tmp.path().join("base.json");
    write(&base_spec, &without_model);
    let first = tmp.path().join("base");
    build(base_spec.to_str().unwrap(), &first);
    let ops = tmp.path().join("replay.json");
    write(
        &ops,
        &json!({"schema":"powerbi-cli.ops.v1","ops":model_ops}),
    );
    let replay = tmp.path().join("replay");
    ok(&[
        "ops",
        "apply",
        "--project",
        first.to_str().unwrap(),
        "--ops",
        ops.to_str().unwrap(),
        "--out-dir",
        replay.to_str().unwrap(),
        "--json",
    ]);
    assert_tree_equal(
        &directory.join(definition),
        &replay.join(definition),
        "declarative model extensions must emit the same complete model as primitive replay",
    );
    // The unrelated page is unchanged, not merely lacking slicer files.
    assert_tree_equal(
        &overview,
        &first.join("Directory.Report/definition/pages/ReportSectionOverview/visuals"),
        "unrelated page",
    );
}

#[test]
fn unsupported_or_malformed_intent_refuses_before_publishing_at_the_declared_spec_pointer() {
    let temp = tempfile::tempdir().unwrap();
    for (ptr, replacement, expected) in [
        (
            "/model/relationships/0/to",
            json!("Branch[Missing]"),
            "/model/relationships/0/to",
        ),
        (
            "/model/relationships/0/to",
            json!("Company[Count]"),
            "/model/relationships/0/to",
        ),
        (
            "/model/relationships/0/cardinality",
            json!("any"),
            "/model/relationships/0/cardinality",
        ),
        (
            "/model/relationships/0/isActive",
            json!("yes"),
            "/model/relationships/0/isActive",
        ),
        (
            "/model/sortBy/0/by",
            json!("Company[Label]"),
            "/model/sortBy/0/by",
        ),
        (
            "/model/sortBy/0/by",
            json!("Branch[Label]"),
            "/model/sortBy/0/by",
        ),
        (
            "/pages/0/slicers/0/searchEnabled",
            json!("true"),
            "/pages/0/slicers/0/searchEnabled",
        ),
        (
            "/pages/0/slicers/0/field",
            json!("Branch[ID]"),
            "/pages/0/slicers/0/searchEnabled",
        ),
        (
            "/pages/0/slicers/0/mode",
            json!("Between"),
            "/pages/0/slicers/0/searchEnabled",
        ),
    ] {
        let mut spec = read(SPEC);
        *spec.pointer_mut(ptr).unwrap() = replacement;
        let path = temp.path().join("invalid.json");
        write(&path, &spec);
        let dest = temp.path().join("must-not-exist");
        let out = run_powerbi(&[
            "report",
            "build",
            "--schema",
            MODEL,
            "--spec",
            path.to_str().unwrap(),
            "--out-dir",
            dest.to_str().unwrap(),
            "--json",
        ]);
        assert_ne!(out.code, 0, "{ptr}: {}", out.stdout);
        let error: Value = serde_json::from_str(&out.stderr).unwrap();
        assert_eq!(error["error"]["pointer"], expected, "{ptr}: {error}");
        assert!(!dest.exists());
    }
}

#[test]
fn explicit_search_off_in_a_rail_is_not_saved_search_text_and_respects_page_opt_out() {
    let temp = tempfile::tempdir().unwrap();
    let mut spec = read(SPEC);
    spec["pages"][0].as_object_mut().unwrap().remove("slicers");
    spec["layout"] = json!({"rail":{"side":"right","slicers":[{"field":"Branch[Label]","title":"Lookup","mode":"Dropdown","searchEnabled":false,"singleSelect":true}]}});
    spec["pages"][1]["rail"] = json!(false);
    let source = temp.path().join("rail.json");
    write(&source, &spec);
    let out = temp.path().join("rail");
    build(source.to_str().unwrap(), &out);
    let slicer = visual(&out, "Directory", "RailLookup");
    let objects = &slicer["visual"]["objects"];
    assert_eq!(
        objects
            .pointer("/general/0/properties/selfFilterEnabled/expr/Literal/Value")
            .unwrap(),
        "false"
    );
    assert_eq!(
        objects
            .pointer("/selection/0/properties/singleSelect/expr/Literal/Value")
            .unwrap(),
        "true"
    );
    assert!(
        objects
            .pointer("/general/0/properties/selfFilter")
            .is_none()
    );
    assert_eq!(
        fs::read_dir(out.join("Directory.Report/definition/pages/ReportSectionOverview/visuals"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn duplicate_relationships_and_sort_cycles_fail_validation_without_writing() {
    let temp = tempfile::tempdir().unwrap();
    let mut base = read(SPEC);
    let rel = base["model"]["relationships"][0].clone();
    base["model"]["relationships"]
        .as_array_mut()
        .unwrap()
        .push(rel);
    let mut cycle = read(SPEC);
    cycle["model"]["sortBy"]
        .as_array_mut()
        .unwrap()
        .push(json!({"column":"Branch[SortOrder]","by":"Branch[Label]"}));
    let mut mismatch = read(SPEC);
    mismatch["model"]["relationships"][0]["to"] = json!("Branch[Label]");
    let mut same = read(SPEC);
    same["model"]["relationships"][0]["to"] = json!("Company[ID]");
    let mut schema = read(MODEL);
    schema["relationships"] =
        json!([{"fromTable":"Company","fromColumn":"BranchID","toTable":"Branch","toColumn":"ID"}]);
    let schema_path = temp.path().join("manifest.json");
    write(&schema_path, &schema);
    let with_schema = schema_path.to_str().unwrap();
    let destination = temp.path().join("must-not-exist");
    for (manifest, spec, expected) in [
        (MODEL, base, "/model/relationships/1/from"),
        (MODEL, cycle, "/model/sortBy/1/by"),
        (MODEL, mismatch, "/model/relationships/0/to"),
        (MODEL, same, "/model/relationships/0/to"),
        (with_schema, read(SPEC), "/model/relationships/0/from"),
    ] {
        let path = temp.path().join("conflict.json");
        write(&path, &spec);
        let out = run_powerbi(&[
            "report",
            "build",
            "--schema",
            manifest,
            "--spec",
            path.to_str().unwrap(),
            "--out-dir",
            destination.to_str().unwrap(),
            "--json",
        ]);
        assert_ne!(out.code, 0);
        let error: Value = serde_json::from_str(&out.stderr).unwrap();
        assert_eq!(error["error"]["pointer"], expected, "{error}");
        assert!(!destination.exists());
    }
}

#[test]
fn saved_search_state_is_distinct_from_the_switch_and_not_exposed_in_safe_slicer_listing() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("project");
    build(SPEC, &out);
    let file=out.join("Directory.Report/definition/pages/ReportSectionDirectory/visuals/VisualContainerSlicerCompany/visual.json");
    let mut visual = read(&file);
    // A deliberately synthetic local term, never a generated default.
    visual["visual"]["objects"]["general"][0]["properties"]["selfFilter"] = json!({"filter":{"Version":2,"Where":[{"Condition":{"Contains":{"Right":{"Literal":{"Value":"'example-lookup'"}}}}}]}});
    write(&file, &visual);
    let listing = ok(&[
        "report",
        "slicers",
        "list",
        "--project",
        out.to_str().unwrap(),
        "--json",
    ]);
    let by_title = |title: &str| {
        listing["slicers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["title"] == title)
            .unwrap()
    };
    assert_eq!(by_title("Company")["state"]["searchEnabled"], true);
    assert_eq!(by_title("Company")["state"]["hasSelectionState"], true);
    assert_eq!(by_title("Company")["safety"]["mayContainDataValues"], true);
    assert_eq!(by_title("Branch")["safety"]["mayContainDataValues"], false);
    assert!(!listing.to_string().contains("example-lookup"));
}
