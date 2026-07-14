use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ACCEPTANCE_SENTINEL: &str = ".powerbi-acceptance-out";
const ACCEPTANCE_SENTINEL_CONTENT: &str = "powerbi-cli acceptance harness output\n";

struct RunOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

struct Harness {
    root: PathBuf,
    coverage: BTreeSet<String>,
}

impl Harness {
    fn new() -> Self {
        let root = if let Ok(out) = env::var("POWERBI_ACCEPTANCE_OUT") {
            PathBuf::from(out)
        } else {
            env::temp_dir().join(format!(
                "powerbi_cli_everything_acceptance_{}",
                std::process::id()
            ))
        };
        prepare_acceptance_root(&root)
            .unwrap_or_else(|message| panic!("unsafe acceptance output directory: {message}"));
        Self {
            root,
            coverage: BTreeSet::new(),
        }
    }

    fn ok(&mut self, path: &str, args: &[String]) -> Value {
        self.coverage.insert(path.to_string());
        let output = run_powerbi(args);
        assert_eq!(
            output.code, 0,
            "command `{path}` failed\nargs: {:?}\nstdout: {}\nstderr: {}",
            args, output.stdout, output.stderr
        );
        stdout_json(&output)
    }

    fn code(&mut self, path: &str, expected: i32, args: &[String]) -> Value {
        self.coverage.insert(path.to_string());
        let output = run_powerbi(args);
        assert_eq!(
            output.code, expected,
            "command `{path}` expected exit {expected}\nargs: {:?}\nstdout: {}\nstderr: {}",
            args, output.stdout, output.stderr
        );
        json_from_any_stream(&output)
    }
}

fn prepare_acceptance_root(path: &Path) -> Result<(), String> {
    if path.exists() {
        if !path.is_dir() {
            return Err(format!(
                "output path is not a directory: {}",
                path.display()
            ));
        }
        let mut entries = fs::read_dir(path)
            .map_err(|err| format!("read output directory {}: {err}", path.display()))?;
        let is_empty = entries
            .next()
            .transpose()
            .map_err(|err| format!("read output directory entry in {}: {err}", path.display()))?
            .is_none();
        if !is_empty {
            let sentinel = path.join(ACCEPTANCE_SENTINEL);
            let marker_matches = fs::read_to_string(&sentinel)
                .map(|content| content == ACCEPTANCE_SENTINEL_CONTENT)
                .unwrap_or(false);
            if !marker_matches {
                return Err(format!(
                    "refusing to recursively delete non-empty {} without harness marker {}",
                    path.display(),
                    sentinel.display()
                ));
            }
        }
        fs::remove_dir_all(path)
            .map_err(|err| format!("remove marked acceptance output {}: {err}", path.display()))?;
    }
    fs::create_dir_all(path)
        .map_err(|err| format!("create acceptance output {}: {err}", path.display()))?;
    fs::write(path.join(ACCEPTANCE_SENTINEL), ACCEPTANCE_SENTINEL_CONTENT).map_err(|err| {
        format!(
            "write acceptance output marker in {}: {err}",
            path.display()
        )
    })
}

#[test]
fn acceptance_root_refuses_unmarked_nonempty_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("unmarked");
    fs::create_dir_all(&root).expect("create unmarked root");
    let user_file = root.join("keep.txt");
    fs::write(&user_file, "keep me").expect("write user file");

    let error = prepare_acceptance_root(&root).expect_err("unmarked directory must be refused");
    assert!(error.contains("refusing to recursively delete"));
    assert!(
        user_file.is_file(),
        "refused cleanup must preserve user data"
    );
}

#[test]
fn acceptance_root_replaces_only_marked_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("marked");
    prepare_acceptance_root(&root).expect("create marked root");
    let stale = root.join("stale.txt");
    fs::write(&stale, "stale").expect("write stale output");

    prepare_acceptance_root(&root).expect("replace marked root");
    assert!(!stale.exists());
    assert_eq!(
        fs::read_to_string(root.join(ACCEPTANCE_SENTINEL)).expect("read marker"),
        ACCEPTANCE_SENTINEL_CONTENT
    );
}

#[test]
fn everything_acceptance_invokes_every_catalog_command() {
    let mut h = Harness::new();
    let schema = h.root.join("everything.schema.json");
    let normalized_schema = h.root.join("everything.schema.normalized.json");
    let profile = h.root.join("everything.profile.json");
    let spec = h.root.join("everything.dashboard.json");
    let planned_spec = h.root.join("everything.planned.dashboard.json");
    let project = h.root.join("EverythingAcceptance");
    let scaffold_project = h.root.join("ScaffoldSmoke");
    let package = h.root.join("source-bearing-template.pbit");
    let package_extract_dir = h.root.join("package_extract");
    let package_import_dir = h.root.join("package_import");
    let fixture_summary = h.root.join("everything.summary.json");
    let theme_bundle = h.root.join("theme.bundle.json");
    let style_before = h.root.join("style.before.json");
    let style_after = h.root.join("style.after.json");
    let visual_formatting_bundle = h.root.join("visual-formatting.bundle.json");
    let wireframe = h.root.join("wireframe.json");
    let dax_file = h.root.join("average-cost.dax");
    let desktop_screenshot = h.root.join("everything-desktop.png");

    write_json(&schema, &acceptance_schema());
    write_json(&spec, &acceptance_dashboard());
    fs::write(
        &dax_file,
        "DIVIDE(\n    [Total Cost],\n    [Total Incidents]\n)\n",
    )
    .expect("write dax file");

    h.ok("capabilities", &svec(["capabilities", "--json"]));
    h.ok("version", &svec(["version", "--json"]));
    h.ok("features list", &svec(["features", "list", "--json"]));
    h.ok("robot-docs guide", &svec(["robot-docs", "guide", "--json"]));
    h.ok("--robot-triage", &svec(["--robot-triage", "--json"]));
    h.ok("robot-triage", &svec(["robot-triage", "--json"]));
    h.ok("doctor", &svec(["doctor", "--json"]));

    h.ok(
        "schema validate",
        &svec(["schema", "validate", &p(&schema), "--json"]),
    );
    h.ok(
        "schema normalize",
        &svec([
            "schema",
            "normalize",
            &p(&schema),
            "--out",
            &p(&normalized_schema),
            "--json",
        ]),
    );
    h.ok(
        "profile infer",
        &svec([
            "profile",
            "infer",
            "--schema",
            &p(&schema),
            "--out",
            &p(&profile),
            "--json",
        ]),
    );
    h.ok(
        "profile validate",
        &svec(["profile", "validate", &p(&profile), "--json"]),
    );
    h.ok(
        "profile summarize",
        &svec(["profile", "summarize", &p(&profile), "--json"]),
    );
    h.ok(
        "report spec validate",
        &svec([
            "report",
            "spec",
            "validate",
            "--schema",
            &p(&schema),
            "--profile",
            &p(&profile),
            "--spec",
            &p(&spec),
            "--json",
        ]),
    );
    h.ok(
        "report spec fields",
        &svec([
            "report",
            "spec",
            "fields",
            "--schema",
            &p(&schema),
            "--profile",
            &p(&profile),
            "--json",
        ]),
    );
    h.ok(
        "scaffold",
        &svec([
            "scaffold",
            "--schema",
            &p(&schema),
            "--out-dir",
            &p(&scaffold_project),
            "--json",
        ]),
    );
    h.ok(
        "report plan",
        &svec([
            "report",
            "plan",
            "--schema",
            &p(&schema),
            "--profile",
            &p(&profile),
            "--objective",
            "Executive safety dashboard with trend, branch comparison, and cost portfolio views",
            "--out",
            &p(&planned_spec),
            "--json",
        ]),
    );
    h.ok(
        "report build",
        &svec([
            "report",
            "build",
            "--schema",
            &p(&schema),
            "--profile",
            &p(&profile),
            "--spec",
            &p(&spec),
            "--out-dir",
            &p(&project),
            "--json",
        ]),
    );
    let project_arg = p(&project);

    h.ok(
        "inspect",
        &svec(["inspect", "--deep", &project_arg, "--json"]),
    );
    h.ok("lint", &svec(["lint", &project_arg, "--json"]));
    h.ok(
        "validate",
        &svec(["validate", "--strict", &project_arg, "--json"]),
    );
    h.ok(
        "handoff check",
        &svec(["handoff", "check", &project_arg, "--json"]),
    );
    h.code(
        "desktop open-check",
        30,
        &svec(["desktop", "open-check", &project_arg, "--json"]),
    );
    h.code(
        "desktop screenshot",
        30,
        &svec([
            "desktop",
            "screenshot",
            &project_arg,
            "--out",
            &p(&desktop_screenshot),
            "--json",
        ]),
    );
    h.ok(
        "package export-plan",
        &svec([
            "package",
            "export-plan",
            "--project",
            &project_arg,
            "--json",
        ]),
    );

    h.ok(
        "package source-pack",
        &svec([
            "package",
            "source-pack",
            "--project",
            &project_arg,
            "--out",
            &p(&package),
            "--json",
        ]),
    );
    h.ok(
        "package inspect",
        &svec(["package", "inspect", &p(&package), "--json"]),
    );
    h.ok(
        "package extract",
        &svec([
            "package",
            "extract",
            &p(&package),
            "--out-dir",
            &p(&package_extract_dir),
            "--json",
        ]),
    );
    h.ok(
        "package import",
        &svec([
            "package",
            "import",
            &p(&package),
            "--out-dir",
            &p(&package_import_dir),
            "--json",
        ]),
    );

    h.ok(
        "source-template add",
        &svec([
            "source-template",
            "add",
            "--project",
            &project_arg,
            "--table",
            "FactIncidents",
            "--name",
            "CorpSqlIncidents",
            "--kind",
            "sql",
            "--server",
            "<server>",
            "--database",
            "<database>",
            "--schema",
            "dbo",
            "--object",
            "FactIncidents",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "source-template list",
        &svec([
            "source-template",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "source-template show",
        &svec([
            "source-template",
            "show",
            "--project",
            &project_arg,
            "--handle",
            "source-template:FactIncidents:CorpSqlIncidents",
            "--json",
        ]),
    );
    h.ok(
        "source-template apply",
        &svec([
            "source-template",
            "apply",
            "--project",
            &project_arg,
            "--handle",
            "source-template:FactIncidents:CorpSqlIncidents",
            "--server",
            "sql.example.internal",
            "--database",
            "Incidents",
            "--dry-run",
            "--json",
        ]),
    );
    h.ok(
        "handoff rebind-plan",
        &svec([
            "handoff",
            "rebind-plan",
            &project_arg,
            "--allow-unmapped",
            "--json",
        ]),
    );

    h.ok(
        "model measures list",
        &svec([
            "model",
            "measures",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "model measures show",
        &svec([
            "model",
            "measures",
            "show",
            "--project",
            &project_arg,
            "--handle",
            "measure:FactIncidents:Total Incidents",
            "--json",
        ]),
    );
    h.ok(
        "model measures add",
        &svec([
            "model",
            "measures",
            "add",
            "--project",
            &project_arg,
            "--table",
            "FactIncidents",
            "--name",
            "Average Cost",
            "--expression-file",
            &p(&dax_file),
            "--format-string",
            "$#,##0",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "model measures update",
        &svec([
            "model",
            "measures",
            "update",
            "--project",
            &project_arg,
            "--handle",
            "measure:FactIncidents:Average Cost",
            "--description",
            "Desktop acceptance DAX measure",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "model measures add",
        &svec([
            "model",
            "measures",
            "add",
            "--project",
            &project_arg,
            "--table",
            "FactIncidents",
            "--name",
            "Transient Measure",
            "--expression",
            "1",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "model measures delete",
        &svec([
            "model",
            "measures",
            "delete",
            "--project",
            &project_arg,
            "--handle",
            "measure:FactIncidents:Transient Measure",
            "--in-place",
            "--confirm",
            "measure:FactIncidents:Transient Measure",
            "--json",
        ]),
    );

    h.ok(
        "model calculated-columns list",
        &svec([
            "model",
            "calculated-columns",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "model calculated-columns add",
        &svec([
            "model",
            "calculated-columns",
            "add",
            "--project",
            &project_arg,
            "--table",
            "FactIncidents",
            "--name",
            "Cost Band",
            "--expression",
            "IF('FactIncidents'[Cost] >= 10000, \"High\", \"Standard\")",
            "--data-type",
            "string",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "model calculated-columns show",
        &svec([
            "model",
            "calculated-columns",
            "show",
            "--project",
            &project_arg,
            "--handle",
            "column:FactIncidents:Cost Band",
            "--json",
        ]),
    );
    h.ok(
        "model calculated-columns update",
        &svec([
            "model",
            "calculated-columns",
            "update",
            "--project",
            &project_arg,
            "--handle",
            "column:FactIncidents:Cost Band",
            "--description",
            "Cost severity band",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "model calculated-columns add",
        &svec([
            "model",
            "calculated-columns",
            "add",
            "--project",
            &project_arg,
            "--table",
            "FactIncidents",
            "--name",
            "Transient Column",
            "--expression",
            "1",
            "--data-type",
            "int64",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "model calculated-columns delete",
        &svec([
            "model",
            "calculated-columns",
            "delete",
            "--project",
            &project_arg,
            "--handle",
            "column:FactIncidents:Transient Column",
            "--in-place",
            "--confirm",
            "column:FactIncidents:Transient Column",
            "--json",
        ]),
    );

    let relationships = h.ok(
        "model relationships list",
        &svec([
            "model",
            "relationships",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    let first_relationship = relationships["relationships"][0]["handle"]
        .as_str()
        .expect("relationship handle")
        .to_string();
    h.ok(
        "model relationships show",
        &svec([
            "model",
            "relationships",
            "show",
            "--project",
            &project_arg,
            "--handle",
            &first_relationship,
            "--json",
        ]),
    );
    let rel_add = h.ok(
        "model relationships add",
        &svec([
            "model",
            "relationships",
            "add",
            "--project",
            &project_arg,
            "--from-table",
            "FactIncidents",
            "--from-column",
            "InjuryTypeKey",
            "--to-table",
            "DimInjuryType",
            "--to-column",
            "InjuryTypeKey",
            "--in-place",
            "--json",
        ]),
    );
    let transient_rel = rel_add["target"]["handle"]
        .as_str()
        .expect("relationship handle")
        .to_string();
    h.ok(
        "model relationships update",
        &svec([
            "model",
            "relationships",
            "update",
            "--project",
            &project_arg,
            "--handle",
            &transient_rel,
            "--inactive",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "model relationships delete",
        &svec([
            "model",
            "relationships",
            "delete",
            "--project",
            &project_arg,
            "--handle",
            &transient_rel,
            "--in-place",
            "--confirm",
            &transient_rel,
            "--json",
        ]),
    );
    h.ok(
        "model partitions list",
        &svec([
            "model",
            "partitions",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "model partitions show",
        &svec([
            "model",
            "partitions",
            "show",
            "--project",
            &project_arg,
            "--handle",
            "partition:FactIncidents:FactIncidents",
            "--json",
        ]),
    );
    h.ok(
        "model dax bridge-plan",
        &svec([
            "model",
            "dax",
            "bridge-plan",
            "--project",
            &project_arg,
            "--engine",
            "desktop",
            "--json",
        ]),
    );
    h.ok(
        "model dax dependencies",
        &svec([
            "model",
            "dax",
            "dependencies",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "model dax lint",
        &svec(["model", "dax", "lint", "--project", &project_arg, "--json"]),
    );
    install_advanced_model_fixtures(&project);
    h.ok(
        "model advanced inventory",
        &svec([
            "model",
            "advanced",
            "inventory",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "model roles list",
        &svec([
            "model",
            "roles",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "model roles show",
        &svec([
            "model",
            "roles",
            "show",
            "--project",
            &project_arg,
            "--handle",
            "role:Safety",
            "--json",
        ]),
    );
    h.ok(
        "model perspectives list",
        &svec([
            "model",
            "perspectives",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "model perspectives show",
        &svec([
            "model",
            "perspectives",
            "show",
            "--project",
            &project_arg,
            "--handle",
            "perspective:Executive",
            "--json",
        ]),
    );
    h.ok(
        "model cultures list",
        &svec([
            "model",
            "cultures",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "model cultures show",
        &svec([
            "model",
            "cultures",
            "show",
            "--project",
            &project_arg,
            "--handle",
            "culture:de-CH",
            "--json",
        ]),
    );
    h.ok(
        "model expressions list",
        &svec([
            "model",
            "expressions",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "model expressions show",
        &svec([
            "model",
            "expressions",
            "show",
            "--project",
            &project_arg,
            "--handle",
            "expression:RefreshDate",
            "--json",
        ]),
    );

    install_conditional_formatting_fixture(&project, "Total Incidents");
    install_slicer_fixture(&project, "Branch Slicer Seed");
    install_bookmark_fixtures(&project);

    let pages = h.ok(
        "report pages list",
        &svec([
            "report",
            "pages",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    let overview = page_handle(&pages, "Overview");
    let visual_catalog = page_handle(&pages, "Visual Catalog");
    let scatter_page = page_handle(&pages, "Scatter Drill");
    let drill_page = page_handle(&pages, "Drillthrough Detail");

    let bookmarks = h.ok(
        "report bookmarks list",
        &svec([
            "report",
            "bookmarks",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    let bookmark_handles = bookmarks["bookmarks"]
        .as_array()
        .expect("bookmarks")
        .iter()
        .map(|bookmark| {
            bookmark["handle"]
                .as_str()
                .expect("bookmark handle")
                .to_string()
        })
        .collect::<Vec<_>>();
    assert!(
        bookmark_handles.len() >= 2,
        "acceptance fixture should install multiple bookmarks"
    );
    let first_bookmark = bookmark_handles[0].clone();
    h.ok(
        "report bookmarks show",
        &svec([
            "report",
            "bookmarks",
            "show",
            "--project",
            &project_arg,
            "--handle",
            &first_bookmark,
            "--json",
        ]),
    );
    h.ok(
        "report bookmarks set-display-name",
        &svec([
            "report",
            "bookmarks",
            "set-display-name",
            "--project",
            &project_arg,
            "--handle",
            &first_bookmark,
            "--display-name",
            "Desktop Acceptance View",
            "--dry-run",
            "--json",
        ]),
    );
    let reversed_bookmarks = bookmark_handles
        .iter()
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join(",");
    h.ok(
        "report bookmarks reorder",
        &svec([
            "report",
            "bookmarks",
            "reorder",
            "--project",
            &project_arg,
            "--order",
            &reversed_bookmarks,
            "--dry-run",
            "--json",
        ]),
    );
    h.ok(
        "report bookmarks delete",
        &svec([
            "report",
            "bookmarks",
            "delete",
            "--project",
            &project_arg,
            "--handle",
            &first_bookmark,
            "--dry-run",
            "--json",
        ]),
    );

    h.ok(
        "report pages show",
        &svec([
            "report",
            "pages",
            "show",
            "--project",
            &project_arg,
            "--handle",
            &overview,
            "--json",
        ]),
    );
    let added_page = h.ok(
        "report pages add",
        &svec([
            "report",
            "pages",
            "add",
            "--project",
            &project_arg,
            "--display-name",
            "Scratch",
            "--width",
            "1280",
            "--height",
            "720",
            "--in-place",
            "--json",
        ]),
    );
    let scratch = added_page["target"]["handle"]
        .as_str()
        .expect("scratch handle")
        .to_string();
    h.ok(
        "report pages update",
        &svec([
            "report",
            "pages",
            "update",
            "--project",
            &project_arg,
            "--handle",
            &scratch,
            "--display-name",
            "Scratch Updated",
            "--in-place",
            "--json",
        ]),
    );
    let order = format!("{overview},{visual_catalog},{scatter_page},{drill_page},{scratch}");
    h.ok(
        "report pages reorder",
        &svec([
            "report",
            "pages",
            "reorder",
            "--project",
            &project_arg,
            "--order",
            &order,
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report pages set-active",
        &svec([
            "report",
            "pages",
            "set-active",
            "--project",
            &project_arg,
            "--handle",
            &overview,
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report pages delete-empty",
        &svec([
            "report",
            "pages",
            "delete-empty",
            "--project",
            &project_arg,
            "--handle",
            &scratch,
            "--in-place",
            "--confirm",
            &scratch,
            "--json",
        ]),
    );

    h.ok(
        "report tree",
        &svec(["report", "tree", "--project", &project_arg, "--json"]),
    );
    let tree = h.ok(
        "report find",
        &svec([
            "report",
            "find",
            "--project",
            &project_arg,
            "--kind",
            "visual",
            "--json",
        ]),
    );
    let first_visual = tree["objects"][0]["handle"]
        .as_str()
        .expect("first visual")
        .to_string();
    h.ok(
        "report cat",
        &svec([
            "report",
            "cat",
            "--project",
            &project_arg,
            "--handle",
            &first_visual,
            "--json",
        ]),
    );
    h.ok(
        "report query",
        &svec([
            "report",
            "query",
            "--project",
            &project_arg,
            "--selector",
            "kind:binding",
            "--json",
        ]),
    );
    let wireframe_json = h.ok(
        "report wireframe export",
        &svec(["report", "wireframe", "export", &project_arg, "--json"]),
    );
    write_json(&wireframe, &wireframe_json);
    h.ok(
        "report layout auto",
        &svec([
            "report",
            "layout",
            "auto",
            "--project",
            &project_arg,
            "--page",
            &visual_catalog,
            "--preset",
            "grid",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report design-plan",
        &svec(["report", "design-plan", "--project", &project_arg, "--json"]),
    );

    let visuals = h.ok(
        "report visuals list",
        &svec([
            "report",
            "visuals",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    let handles = visual_handles_by_title(&visuals);
    let total_incidents = handles["Total Incidents"].clone();
    let severe_incidents = handles["Severe Incidents"].clone();
    let line = handles["Incident Rate Drilldown"].clone();
    let table = handles["Company Detail"].clone();
    let scatter = handles["Branch Injury Cost Bubble"].clone();
    let catalog_column = handles["Stacked Column by Year"].clone();
    let slicer = slicer_handle(&h.ok(
        "report slicers list",
        &svec([
            "report",
            "slicers",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    ));

    h.ok(
        "report visuals show",
        &svec([
            "report",
            "visuals",
            "show",
            "--project",
            &project_arg,
            "--handle",
            &line,
            "--json",
        ]),
    );
    h.ok(
        "report visuals catalog",
        &svec(["report", "visuals", "catalog", "--json"]),
    );
    h.ok(
        "report visuals set-position",
        &svec([
            "report",
            "visuals",
            "set-position",
            "--project",
            &project_arg,
            "--handle",
            &scatter,
            "--x",
            "40",
            "--y",
            "92",
            "--width",
            "720",
            "--height",
            "460",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report visuals set-bindings",
        &svec([
            "report",
            "visuals",
            "set-bindings",
            "--project",
            &project_arg,
            "--handle",
            &line,
            "--binding",
            "role=Category,table=DimBranch,column=Branch",
            "--binding",
            "role=Category,table=DimCompany,column=Company",
            "--binding",
            "role=Y,table=FactIncidents,measure=Incident Rate",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report drilldown set-hierarchy",
        &svec([
            "report",
            "drilldown",
            "set-hierarchy",
            "--project",
            &project_arg,
            "--handle",
            &line,
            "--field",
            "DimBranch[Branch]",
            "--field",
            "DimCompany[Company]",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report drillthrough set",
        &svec([
            "report",
            "drillthrough",
            "set",
            "--project",
            &project_arg,
            "--page",
            &drill_page,
            "--target",
            "DimCompany[Company]",
            "--keep-visible",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report drillthrough show",
        &svec([
            "report",
            "drillthrough",
            "show",
            "--project",
            &project_arg,
            "--page",
            &drill_page,
            "--json",
        ]),
    );
    h.ok(
        "report drillthrough clear",
        &svec([
            "report",
            "drillthrough",
            "clear",
            "--project",
            &project_arg,
            "--page",
            &drill_page,
            "--in-place",
            "--confirm",
            &drill_page,
            "--json",
        ]),
    );
    h.ok(
        "report drillthrough set",
        &svec([
            "report",
            "drillthrough",
            "set",
            "--project",
            &project_arg,
            "--page",
            &drill_page,
            "--target",
            "DimCompany[Company]",
            "--keep-visible",
            "--in-place",
            "--json",
        ]),
    );

    h.ok(
        "report visuals formatting set-text",
        &svec([
            "report",
            "visuals",
            "formatting",
            "set-text",
            "--project",
            &project_arg,
            "--handle",
            &total_incidents,
            "--title",
            "Total Incidents KPI",
            "--alt-text",
            "Total incident count",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report visuals formatting set-color",
        &svec([
            "report",
            "visuals",
            "formatting",
            "set-color",
            "--project",
            &project_arg,
            "--handle",
            &total_incidents,
            "--slot",
            "title.fontColor",
            "--color",
            "#111827",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report visuals formatting list",
        &svec([
            "report",
            "visuals",
            "formatting",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "report visuals formatting show",
        &svec([
            "report",
            "visuals",
            "formatting",
            "show",
            "--project",
            &project_arg,
            "--handle",
            &total_incidents,
            "--include-raw",
            "--json",
        ]),
    );
    h.ok(
        "report visuals formatting conditional-formatting list",
        &svec([
            "report",
            "visuals",
            "formatting",
            "conditional-formatting",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "report visuals formatting conditional-formatting show",
        &svec([
            "report",
            "visuals",
            "formatting",
            "conditional-formatting",
            "show",
            "--project",
            &project_arg,
            "--handle",
            &total_incidents,
            "--include-raw",
            "--json",
        ]),
    );
    h.ok(
        "report visuals formatting extract",
        &svec([
            "report",
            "visuals",
            "formatting",
            "extract",
            "--project",
            &project_arg,
            "--handle",
            &total_incidents,
            "--out",
            &p(&visual_formatting_bundle),
            "--json",
        ]),
    );
    h.ok(
        "report visuals formatting apply",
        &svec([
            "report",
            "visuals",
            "formatting",
            "apply",
            "--project",
            &project_arg,
            "--handle",
            &severe_incidents,
            "--bundle",
            &p(&visual_formatting_bundle),
            "--allow-literal-text",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report visuals formatting set-text",
        &svec([
            "report",
            "visuals",
            "formatting",
            "set-text",
            "--project",
            &project_arg,
            "--handle",
            &severe_incidents,
            "--title",
            "Severe Incidents KPI",
            "--alt-text",
            "Severe incident count",
            "--in-place",
            "--json",
        ]),
    );

    let added_visual = h.ok(
        "report visuals add",
        &svec([
            "report",
            "visuals",
            "add",
            "--project",
            &project_arg,
            "--page",
            &visual_catalog,
            "--visual-type",
            "card",
            "--title",
            "Transient Cost Card",
            "--binding",
            "role=Values,table=FactIncidents,measure=Total Cost",
            "--in-place",
            "--json",
        ]),
    );
    let transient_visual = added_visual["target"]["handle"]
        .as_str()
        .expect("added visual")
        .to_string();
    let cloned = h.ok(
        "report visuals clone",
        &svec([
            "report",
            "visuals",
            "clone",
            "--project",
            &project_arg,
            "--handle",
            &catalog_column,
            "--title",
            "Cloned Column",
            "--x",
            "860",
            "--y",
            "392",
            "--in-place",
            "--json",
        ]),
    );
    let cloned_visual = cloned["target"]["handle"]
        .as_str()
        .expect("cloned visual")
        .to_string();
    h.ok(
        "report visuals delete",
        &svec([
            "report",
            "visuals",
            "delete",
            "--project",
            &project_arg,
            "--handle",
            &transient_visual,
            "--in-place",
            "--confirm",
            &transient_visual,
            "--json",
        ]),
    );
    h.ok(
        "report visuals delete",
        &svec([
            "report",
            "visuals",
            "delete",
            "--project",
            &project_arg,
            "--handle",
            &cloned_visual,
            "--in-place",
            "--confirm",
            &cloned_visual,
            "--json",
        ]),
    );

    h.ok(
        "report themes presets",
        &svec(["report", "themes", "presets", "list", "--json"]),
    );
    h.ok(
        "report themes apply-preset",
        &svec([
            "report",
            "themes",
            "apply-preset",
            "--project",
            &project_arg,
            "--preset",
            "risk-dashboard",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report themes show",
        &svec([
            "report",
            "themes",
            "show",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "report themes extract",
        &svec([
            "report",
            "themes",
            "extract",
            "--project",
            &project_arg,
            "--out",
            &p(&theme_bundle),
            "--json",
        ]),
    );
    h.ok(
        "report themes apply",
        &svec([
            "report",
            "themes",
            "apply",
            "--project",
            &project_arg,
            "--bundle",
            &p(&theme_bundle),
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report style inspect",
        &svec([
            "report",
            "style",
            "inspect",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "report style extract",
        &svec([
            "report",
            "style",
            "extract",
            "--project",
            &project_arg,
            "--out",
            &p(&style_before),
            "--include-literal-text",
            "--json",
        ]),
    );
    h.ok(
        "report style apply",
        &svec([
            "report",
            "style",
            "apply",
            "--project",
            &project_arg,
            "--bundle",
            &p(&style_before),
            "--allow-literal-text",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report style extract",
        &svec([
            "report",
            "style",
            "extract",
            "--project",
            &project_arg,
            "--out",
            &p(&style_after),
            "--include-literal-text",
            "--json",
        ]),
    );
    h.ok(
        "report style diff",
        &svec([
            "report",
            "style",
            "diff",
            &p(&style_before),
            &p(&style_after),
            "--json",
        ]),
    );

    h.ok(
        "report filters add",
        &svec([
            "report",
            "filters",
            "add",
            "--project",
            &project_arg,
            "--scope",
            "report",
            "--target",
            "DimBranch[Country]",
            "--value",
            "CH",
            "--in-place",
            "--json",
        ]),
    );
    let filters = h.ok(
        "report filters list",
        &svec([
            "report",
            "filters",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    let report_filter = filters["filters"][0]["handle"]
        .as_str()
        .expect("filter handle")
        .to_string();
    h.ok(
        "report filters show",
        &svec([
            "report",
            "filters",
            "show",
            "--project",
            &project_arg,
            "--handle",
            &report_filter,
            "--json",
        ]),
    );
    h.ok(
        "report filters update",
        &svec([
            "report",
            "filters",
            "update",
            "--project",
            &project_arg,
            "--handle",
            &report_filter,
            "--display-name",
            "Country",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report audit",
        &svec(["report", "audit", "--project", &project_arg, "--json"]),
    );
    h.ok(
        "report sanitize plan",
        &svec([
            "report",
            "sanitize",
            "plan",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    h.ok(
        "report sanitize apply",
        &svec([
            "report",
            "sanitize",
            "apply",
            "--project",
            &project_arg,
            "--dry-run",
            "--json",
        ]),
    );
    h.ok(
        "report filters delete",
        &svec([
            "report",
            "filters",
            "delete",
            "--project",
            &project_arg,
            "--handle",
            &report_filter,
            "--in-place",
            "--confirm",
            &report_filter,
            "--json",
        ]),
    );
    h.ok(
        "report filters add",
        &svec([
            "report",
            "filters",
            "add",
            "--project",
            &project_arg,
            "--page",
            &overview,
            "--target",
            "DimBranch[Branch]",
            "--value",
            "Construction",
            "--in-place",
            "--json",
        ]),
    );
    h.ok(
        "report filters add",
        &svec([
            "report",
            "filters",
            "add",
            "--project",
            &project_arg,
            "--visual",
            &table,
            "--target",
            "DimBranch[Branch]",
            "--value",
            "Construction",
            "--in-place",
            "--json",
        ]),
    );
    let clear_dry = h.ok(
        "report filters clear",
        &svec([
            "report",
            "filters",
            "clear",
            "--project",
            &project_arg,
            "--scope",
            "page",
            "--page",
            &overview,
            "--dry-run",
            "--json",
        ]),
    );
    let clear_token = clear_dry["confirmToken"]
        .as_str()
        .expect("filter clear token")
        .to_string();
    h.ok(
        "report filters clear",
        &svec([
            "report",
            "filters",
            "clear",
            "--project",
            &project_arg,
            "--scope",
            "page",
            "--page",
            &overview,
            "--in-place",
            "--confirm",
            &clear_token,
            "--json",
        ]),
    );

    h.ok(
        "report slicers show",
        &svec([
            "report",
            "slicers",
            "show",
            "--project",
            &project_arg,
            "--handle",
            &slicer,
            "--json",
        ]),
    );
    let slicer_dry = h.ok(
        "report slicers clear",
        &svec([
            "report",
            "slicers",
            "clear",
            "--project",
            &project_arg,
            "--handle",
            &slicer,
            "--dry-run",
            "--json",
        ]),
    );
    let slicer_token = slicer_dry["confirmToken"]
        .as_str()
        .expect("slicer token")
        .to_string();
    h.ok(
        "report slicers clear",
        &svec([
            "report",
            "slicers",
            "clear",
            "--project",
            &project_arg,
            "--handle",
            &slicer,
            "--in-place",
            "--confirm",
            &slicer_token,
            "--json",
        ]),
    );

    h.ok(
        "report interactions set",
        &svec([
            "report",
            "interactions",
            "set",
            "--project",
            &project_arg,
            "--page",
            &overview,
            "--source",
            &line,
            "--target",
            &table,
            "--type",
            "DataFilter",
            "--in-place",
            "--json",
        ]),
    );
    let interactions = h.ok(
        "report interactions list",
        &svec([
            "report",
            "interactions",
            "list",
            "--project",
            &project_arg,
            "--json",
        ]),
    );
    let interaction = interactions["interactions"][0]["handle"]
        .as_str()
        .expect("interaction")
        .to_string();
    h.ok(
        "report interactions show",
        &svec([
            "report",
            "interactions",
            "show",
            "--project",
            &project_arg,
            "--handle",
            &interaction,
            "--json",
        ]),
    );
    h.ok(
        "report interactions disable",
        &svec([
            "report",
            "interactions",
            "disable",
            "--project",
            &project_arg,
            "--page",
            &overview,
            "--source",
            &line,
            "--target",
            &table,
            "--in-place",
            "--json",
        ]),
    );

    h.ok(
        "fixture normalize",
        &svec([
            "fixture",
            "normalize",
            &project_arg,
            "--out",
            &p(&fixture_summary),
            "--json",
        ]),
    );
    h.ok(
        "fixture verify",
        &svec([
            "fixture",
            "verify",
            &project_arg,
            "--expected",
            &p(&fixture_summary),
            "--json",
        ]),
    );
    h.ok(
        "diff",
        &svec(["diff", &project_arg, &project_arg, "--json"]),
    );

    h.ok(
        "validate",
        &svec(["validate", "--strict", &project_arg, "--json"]),
    );
    h.ok(
        "handoff check",
        &svec(["handoff", "check", &project_arg, "--json"]),
    );

    assert_capability_coverage(&h.coverage);

    let proof = json!({
        "schema": "powerbi-cli.desktopAcceptanceEverything.v1",
        "projectDir": project_arg,
        "pbip": p(&project.join("EverythingAcceptance.pbip")),
        "schemaPath": p(&schema),
        "dashboardSpec": p(&spec),
        "fixtureSummary": p(&fixture_summary),
        "coverage": {
            "matchedCommands": h.coverage.len(),
            "commands": h.coverage.iter().cloned().collect::<Vec<_>>()
        },
        "desktopProof": {
            "status": "pending-computer-use",
            "required": "Open the generated PBIP in Power BI Desktop, refresh, inspect Overview/Visual Catalog/Scatter Drill/Drillthrough Detail, fail on issue banners or blank canvas, then close Desktop."
        }
    });
    write_json(&h.root.join("everything-acceptance-run.json"), &proof);
}

fn assert_capability_coverage(coverage: &BTreeSet<String>) {
    let output = run_powerbi(&svec(["capabilities", "--json"]));
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let commands = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path").to_string())
        .collect::<BTreeSet<_>>();
    let missing = commands.difference(coverage).cloned().collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "desktop acceptance harness did not invoke every advertised command: {missing:#?}"
    );
    assert_eq!(
        commands.len(),
        coverage.len(),
        "coverage set has unexpected extra command paths"
    );
}

fn run_powerbi(args: &[String]) -> RunOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args(args)
        .env_remove("POWERBI_DESKTOP_ORACLE")
        .output()
        .expect("run powerbi-cli binary");
    RunOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn stdout_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stdout.trim()).expect("stdout JSON")
}

fn json_from_any_stream(output: &RunOutput) -> Value {
    let text = if output.stdout.trim().is_empty() {
        output.stderr.trim()
    } else {
        output.stdout.trim()
    };
    serde_json::from_str(text).expect("JSON output")
}

fn svec(items: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<String> {
    items
        .into_iter()
        .map(|item| item.as_ref().to_string())
        .collect()
}

fn p(path: &Path) -> String {
    path.to_str().expect("path").to_string()
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("json parent dir");
    }
    fs::write(
        path,
        serde_json::to_string_pretty(value).expect("serialize json"),
    )
    .expect("write json");
}

fn acceptance_schema() -> Value {
    let years = 2018..=2025;
    let branches = [
        (1, "Construction", "High", 130.0),
        (2, "Logistics", "Elevated", 92.0),
        (3, "Manufacturing", "Elevated", 82.0),
        (4, "Services", "Moderate", 42.0),
    ];
    let injury_types = [
        (1, "Sprain"),
        (2, "Cut"),
        (3, "Fracture"),
        (4, "Burn"),
        (5, "Contusion"),
    ];
    let mut dates = Vec::new();
    for year in years.clone() {
        for quarter in 1..=4 {
            let month = (quarter - 1) * 3 + 1;
            dates.push(json!({
                "DateKey": year * 10000 + month * 100 + 1,
                "Date": format!("{year}-{month:02}-01"),
                "Year": year,
                "Quarter": format!("Q{quarter}"),
                "MonthNo": month,
                "MonthName": month_name(month)
            }));
        }
    }
    let companies = vec![
        (101, 1, "Bau Nord AG", "Large", 320),
        (102, 1, "Alpine Works GmbH", "Mid", 185),
        (103, 1, "TunnelTec SA", "Mid", 240),
        (104, 1, "KranPartner AG", "Small", 74),
        (201, 2, "FleetMove AG", "Large", 410),
        (202, 2, "Parcel Route SA", "Mid", 230),
        (203, 2, "ColdChain Logistics", "Small", 95),
        (301, 3, "Precision Parts AG", "Large", 510),
        (302, 3, "MetalForm SA", "Mid", 210),
        (303, 3, "FoodPack Works", "Small", 120),
        (401, 4, "OfficeCare AG", "Large", 360),
        (402, 4, "Retail Field Services", "Mid", 155),
    ];
    let mut fact_rows = Vec::new();
    let mut incident_id = 1;
    for (company_index, (company_key, branch_key, _, _, employees)) in companies.iter().enumerate()
    {
        for (date_index, date) in dates.iter().enumerate() {
            let year = date["Year"].as_i64().expect("year") as i32;
            let branch_rate = branches
                .iter()
                .find(|branch| branch.0 == *branch_key)
                .map(|branch| branch.3)
                .expect("branch rate");
            let year_factor = 0.92 + f64::from(year - 2018) * 0.025;
            let company_factor = 0.82 + (company_index % 5) as f64 * 0.08;
            let quarterly_expected =
                (*employees as f64 / 1000.0) * branch_rate * year_factor * company_factor / 4.0;
            let accident_count = (quarterly_expected.round() as i64).max(1);
            let severe_flag = i64::from((company_index + date_index) % 7 == 0);
            let injury = injury_types[(company_index + date_index) % injury_types.len()];
            let cause = match (company_index + date_index) % 6 {
                0 => "slip-trip",
                1 => "fall-from-height",
                2 => "cut-puncture",
                3 => "caught-pinched",
                4 => "struck-by",
                _ => "overexertion",
            };
            let cost = accident_count as f64 * (1200.0 + severe_flag as f64 * 15500.0)
                + ((company_index * 311 + date_index * 97) % 700) as f64;
            let lost_days = accident_count * (2 + severe_flag * 18 + (date_index % 3) as i64);
            let exposure_hours = (*employees as f64) * 520.0;
            fact_rows.push(json!({
                "IncidentId": incident_id,
                "DateKey": date["DateKey"],
                "BranchKey": branch_key,
                "CompanyKey": company_key,
                "InjuryTypeKey": injury.0,
                "InjuryType": injury.1,
                "Cause": cause,
                "Severity": if severe_flag == 1 { "Severe" } else { "Standard" },
                "AccidentCount": accident_count,
                "SevereFlag": severe_flag,
                "Cost": (cost * 100.0).round() / 100.0,
                "LostDays": lost_days,
                "ExposureHours": exposure_hours,
                "RiskScore": ((branch_rate / 75.0) * 50.0 + severe_flag as f64 * 30.0 + (date_index % 5) as f64 * 3.0)
            }));
            incident_id += 1;
        }
    }
    let target_rows = branches
        .iter()
        .flat_map(|branch| {
            years.clone().map(move |year| {
                json!({
                    "BranchKey": branch.0,
                    "Year": year,
                    "TargetRate": (branch.3 * 0.82 * 10.0).round() / 10.0
                })
            })
        })
        .collect::<Vec<_>>();

    json!({
        "name": "EverythingAcceptance",
        "displayName": "Everything Acceptance",
        "description": "Offline-safe Desktop acceptance dataset covering every powerbi-cli command family.",
        "locale": "en-US",
        "tables": [
            {
                "name": "DimDate",
                "columns": [
                    {"name": "DateKey", "dataType": "int64", "isKey": true},
                    {"name": "Date", "dataType": "date", "formatString": "Short Date"},
                    {"name": "Year", "dataType": "int64"},
                    {"name": "Quarter", "dataType": "string"},
                    {"name": "MonthNo", "dataType": "int64"},
                    {"name": "MonthName", "dataType": "string"}
                ],
                "rows": dates
            },
            {
                "name": "DimBranch",
                "columns": [
                    {"name": "BranchKey", "dataType": "int64", "isKey": true},
                    {"name": "Branch", "dataType": "string"},
                    {"name": "RiskClass", "dataType": "string"},
                    {"name": "Country", "dataType": "string"}
                ],
                "rows": branches.iter().map(|branch| json!({
                    "BranchKey": branch.0,
                    "Branch": branch.1,
                    "RiskClass": branch.2,
                    "Country": "CH"
                })).collect::<Vec<_>>()
            },
            {
                "name": "DimCompany",
                "columns": [
                    {"name": "CompanyKey", "dataType": "int64", "isKey": true},
                    {"name": "BranchKey", "dataType": "int64"},
                    {"name": "Company", "dataType": "string"},
                    {"name": "SizeBand", "dataType": "string"},
                    {"name": "Employees", "dataType": "int64"}
                ],
                "rows": companies.iter().map(|company| json!({
                    "CompanyKey": company.0,
                    "BranchKey": company.1,
                    "Company": company.2,
                    "SizeBand": company.3,
                    "Employees": company.4
                })).collect::<Vec<_>>()
            },
            {
                "name": "DimInjuryType",
                "columns": [
                    {"name": "InjuryTypeKey", "dataType": "int64", "isKey": true},
                    {"name": "InjuryTypeName", "dataType": "string"}
                ],
                "rows": injury_types.iter().map(|injury| json!({
                    "InjuryTypeKey": injury.0,
                    "InjuryTypeName": injury.1
                })).collect::<Vec<_>>()
            },
            {
                "name": "FactIncidents",
                "columns": [
                    {"name": "IncidentId", "dataType": "int64", "isKey": true},
                    {"name": "DateKey", "dataType": "int64"},
                    {"name": "BranchKey", "dataType": "int64"},
                    {"name": "CompanyKey", "dataType": "int64"},
                    {"name": "InjuryTypeKey", "dataType": "int64"},
                    {"name": "InjuryType", "dataType": "string"},
                    {"name": "Cause", "dataType": "string"},
                    {"name": "Severity", "dataType": "string"},
                    {"name": "AccidentCount", "dataType": "int64", "formatString": "#,##0"},
                    {"name": "SevereFlag", "dataType": "int64"},
                    {"name": "Cost", "dataType": "decimal", "formatString": "$#,##0"},
                    {"name": "LostDays", "dataType": "int64", "formatString": "#,##0"},
                    {"name": "ExposureHours", "dataType": "double"},
                    {"name": "RiskScore", "dataType": "double", "formatString": "0.0"}
                ],
                "measures": [
                    {"name": "Total Incidents", "expression": "SUM('FactIncidents'[AccidentCount])", "formatString": "#,##0"},
                    {"name": "Total Cost", "expression": "SUM('FactIncidents'[Cost])", "formatString": "$#,##0"},
                    {"name": "Severe Incidents", "expression": "SUM('FactIncidents'[SevereFlag])", "formatString": "#,##0"},
                    {"name": "Lost Days", "expression": "SUM('FactIncidents'[LostDays])", "formatString": "#,##0"},
                    {"name": "Exposure FTE", "expression": "DIVIDE(SUM('FactIncidents'[ExposureHours]), 2080)", "formatString": "#,##0.0"},
                    {"name": "Incident Rate", "expression": "DIVIDE([Total Incidents] * 1000, [Exposure FTE])", "formatString": "0.0"},
                    {"name": "Severity Share", "expression": "DIVIDE([Severe Incidents], [Total Incidents])", "formatString": "0.0%"},
                    {"name": "Average Risk Score", "expression": "AVERAGE('FactIncidents'[RiskScore])", "formatString": "0.0"},
                    {"name": "Cost per Incident", "expression": "DIVIDE([Total Cost], [Total Incidents])", "formatString": "$#,##0"},
                    {"name": "High Cost Incidents", "expression": "CALCULATE([Total Incidents], FILTER('FactIncidents', 'FactIncidents'[Cost] >= 10000))", "formatString": "#,##0"}
                ],
                "rows": fact_rows
            },
            {
                "name": "FactTargets",
                "columns": [
                    {"name": "BranchKey", "dataType": "int64"},
                    {"name": "Year", "dataType": "int64"},
                    {"name": "TargetRate", "dataType": "double", "formatString": "0.0"}
                ],
                "measures": [
                    {"name": "Target Rate", "expression": "AVERAGE('FactTargets'[TargetRate])", "formatString": "0.0"}
                ],
                "rows": target_rows
            }
        ],
        "relationships": [
            {"name": "relFactIncidentsDimDateDateKey", "fromTable": "FactIncidents", "fromColumn": "DateKey", "toTable": "DimDate", "toColumn": "DateKey"},
            {"name": "relFactIncidentsDimBranchBranchKey", "fromTable": "FactIncidents", "fromColumn": "BranchKey", "toTable": "DimBranch", "toColumn": "BranchKey"},
            {"name": "relFactIncidentsDimCompanyCompanyKey", "fromTable": "FactIncidents", "fromColumn": "CompanyKey", "toTable": "DimCompany", "toColumn": "CompanyKey"},
            {"name": "relFactTargetsDimBranchBranchKey", "fromTable": "FactTargets", "fromColumn": "BranchKey", "toTable": "DimBranch", "toColumn": "BranchKey"}
        ],
        "pages": []
    })
}

macro_rules! visual {
    ($id:expr, $visual_type:expr, $title:expr, $x:expr, $y:expr, $width:expr, $height:expr, $bindings:expr) => {{
        let bindings = $bindings;
        json!({
            "id": $id,
            "type": $visual_type,
            "title": $title,
            "bindings": bindings.into_iter().map(|(role, field)| json!({"role": role, "field": field})).collect::<Vec<_>>(),
            "layout": { "x": $x, "y": $y, "width": $width, "height": $height }
        })
    }};
}

fn acceptance_dashboard() -> Value {
    json!({
        "schema": "powerbi-cli.dashboard.v1",
        "report": {
            "name": "EverythingAcceptance",
            "displayName": "Everything Acceptance",
            "locale": "en-US",
            "audience": "agents validating offline Power BI authoring",
            "questions": [
                "Which branches have the highest injury rate?",
                "Which injury types and accident paths drive cost?"
            ]
        },
        "model": {
            "measures": [
                {
                    "table": "FactIncidents",
                    "name": "Cost Severity Index",
                    "expression": "DIVIDE([Total Cost], [Lost Days])",
                    "formatString": "$#,##0"
                }
            ]
        },
        "pages": [
            {
                "id": "overview",
                "displayName": "Overview",
                "size": { "width": 1280, "height": 720 },
                "visuals": [
                    visual!("total_incidents", "card", "Total Incidents", 32, 32, 220, 112, vec![("Values", "FactIncidents[Total Incidents]")]),
                    visual!("severe_incidents", "card", "Severe Incidents", 276, 32, 220, 112, vec![("Values", "FactIncidents[Severe Incidents]")]),
                    visual!("incident_rate_drill", "lineChart", "Incident Rate Drilldown", 32, 180, 600, 340, vec![("Category", "DimDate[Year]"), ("Y", "FactIncidents[Incident Rate]"), ("Series", "DimBranch[Branch]")]),
                    visual!("company_detail", "tableEx", "Company Detail", 656, 180, 560, 340, vec![("Values", "DimBranch[Branch]"), ("Values", "DimCompany[Company]"), ("Values", "FactIncidents[Incident Rate]"), ("Values", "FactIncidents[Total Cost]")]),
                    visual!("branch_column", "clusteredColumnChart", "Branch Incidents", 32, 548, 560, 140, vec![("Category", "DimBranch[Branch]"), ("Y", "FactIncidents[Total Incidents]")])
                ]
            },
            {
                "id": "visual_catalog",
                "displayName": "Visual Catalog",
                "size": { "width": 1280, "height": 720 },
                "visuals": [
                    visual!("area", "areaChart", "Area Cost Trend", 32, 32, 380, 190, vec![("Category", "DimDate[Year]"), ("Y", "FactIncidents[Total Cost]")]),
                    visual!("stacked_area", "stackedAreaChart", "Stacked Area by Branch", 444, 32, 380, 190, vec![("Category", "DimDate[Year]"), ("Y", "FactIncidents[Total Incidents]"), ("Series", "DimBranch[Branch]")]),
                    visual!("bar", "clusteredBarChart", "Clustered Bar by Cause", 856, 32, 360, 190, vec![("Category", "FactIncidents[Cause]"), ("Y", "FactIncidents[Total Cost]")]),
                    visual!("stacked_bar", "barChart", "Stacked Bar Severity", 32, 264, 380, 190, vec![("Category", "FactIncidents[Cause]"), ("Y", "FactIncidents[Total Incidents]"), ("Series", "FactIncidents[Severity]")]),
                    visual!("stacked_column", "columnChart", "Stacked Column by Year", 444, 264, 380, 190, vec![("Category", "DimDate[Year]"), ("Y", "FactIncidents[Total Incidents]"), ("Series", "DimBranch[Branch]")]),
                    visual!("catalog_table", "tableEx", "Visual Catalog Table", 856, 264, 360, 360, vec![("Values", "DimBranch[Branch]"), ("Values", "FactIncidents[Cause]"), ("Values", "FactIncidents[Total Incidents]"), ("Values", "FactIncidents[Total Cost]")])
                ]
            },
            {
                "id": "scatter_drill",
                "displayName": "Scatter Drill",
                "size": { "width": 1280, "height": 720 },
                "visuals": [
                    visual!("bubble", "scatterChart", "Branch Injury Cost Bubble", 32, 92, 700, 460, vec![("Category", "FactIncidents[Cause]"), ("X", "FactIncidents[Incident Rate]"), ("Y", "FactIncidents[Severity Share]"), ("Size", "FactIncidents[Total Cost]"), ("Legend", "DimBranch[Branch]"), ("Tooltips", "FactIncidents[Average Risk Score]")]),
                    visual!("slicer_seed", "tableEx", "Branch Slicer Seed", 772, 92, 280, 210, vec![("Values", "DimBranch[Branch]")]),
                    visual!("scatter_detail", "tableEx", "Scatter Detail", 772, 332, 420, 220, vec![("Values", "DimBranch[Branch]"), ("Values", "FactIncidents[Cause]"), ("Values", "FactIncidents[InjuryType]"), ("Values", "FactIncidents[Total Cost]")])
                ]
            },
            {
                "id": "drillthrough_detail",
                "displayName": "Drillthrough Detail",
                "size": { "width": 1280, "height": 720 },
                "visuals": [
                    visual!("drill_table", "tableEx", "Company Drillthrough Detail", 40, 80, 760, 420, vec![("Values", "DimCompany[Company]"), ("Values", "DimBranch[Branch]"), ("Values", "FactIncidents[Cause]"), ("Values", "FactIncidents[Total Incidents]"), ("Values", "FactIncidents[Total Cost]")]),
                    visual!("drill_card", "card", "Drillthrough Cost", 840, 80, 260, 120, vec![("Values", "FactIncidents[Total Cost]")])
                ]
            }
        ]
    })
}

fn month_name(month: i32) -> &'static str {
    match month {
        1 => "Jan",
        4 => "Apr",
        7 => "Jul",
        10 => "Oct",
        _ => "Month",
    }
}

fn page_handle(pages: &Value, display_name: &str) -> String {
    pages["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .find(|page| page["displayName"] == display_name)
        .and_then(|page| page["handle"].as_str())
        .unwrap_or_else(|| panic!("page not found: {display_name}"))
        .to_string()
}

fn visual_handles_by_title(visuals: &Value) -> BTreeMap<String, String> {
    visuals["visuals"]
        .as_array()
        .expect("visuals")
        .iter()
        .filter_map(|visual| {
            Some((
                visual["title"].as_str()?.to_string(),
                visual["handle"].as_str()?.to_string(),
            ))
        })
        .collect()
}

fn slicer_handle(slicers: &Value) -> String {
    slicers["slicers"]
        .as_array()
        .expect("slicers")
        .first()
        .and_then(|slicer| slicer["handle"].as_str())
        .expect("slicer handle")
        .to_string()
}

fn report_dir(project: &Path) -> PathBuf {
    project.join("EverythingAcceptance.Report")
}

fn pages_json(project: &Path) -> PathBuf {
    report_dir(project)
        .join("definition")
        .join("pages")
        .join("pages.json")
}

fn page_names(project: &Path) -> Vec<String> {
    let value = read_json(&pages_json(project));
    value["pageOrder"]
        .as_array()
        .expect("pageOrder")
        .iter()
        .filter_map(|item| item.as_str().map(ToOwned::to_owned))
        .collect()
}

fn visual_json_by_title(project: &Path, title: &str) -> PathBuf {
    for page in page_names(project) {
        let visuals_dir = report_dir(project)
            .join("definition")
            .join("pages")
            .join(page)
            .join("visuals");
        for entry in fs::read_dir(&visuals_dir).expect("visuals dir") {
            let entry = entry.expect("visual entry");
            if !entry.file_type().expect("file type").is_dir() {
                continue;
            }
            let path = entry.path().join("visual.json");
            let value = read_json(&path);
            if value["annotations"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|annotation| {
                    annotation["name"] == "powerbi-cli.placeholderTitle"
                        && annotation["value"] == title
                })
            {
                return path;
            }
        }
    }
    panic!("visual title not found: {title}");
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("read json")).expect("parse json")
}

fn patch_json(path: &Path, patch: impl FnOnce(&mut Value)) {
    let mut value = read_json(path);
    patch(&mut value);
    write_json(path, &value);
}

fn install_advanced_model_fixtures(project: &Path) {
    let definition = project
        .join("EverythingAcceptance.SemanticModel")
        .join("definition");
    fs::create_dir_all(definition.join("roles")).expect("roles dir");
    fs::create_dir_all(definition.join("perspectives")).expect("perspectives dir");
    fs::create_dir_all(definition.join("cultures")).expect("cultures dir");
    fs::write(
        definition.join("roles").join("Safety.tmdl"),
        "role Safety\n\tmodelPermission: read\n\ttablePermission FactIncidents\n",
    )
    .expect("role fixture");
    fs::write(
        definition.join("perspectives").join("Executive.tmdl"),
        "perspective Executive\n\tperspectiveTable FactIncidents\n",
    )
    .expect("perspective fixture");
    fs::write(
        definition.join("cultures").join("de-CH.tmdl"),
        "culture 'de-CH'\n\ttranslation FactIncidents\n",
    )
    .expect("culture fixture");
    fs::write(
        definition.join("expressions.tmdl"),
        "expression RefreshDate = DateTime.LocalNow()\n",
    )
    .expect("expression fixture");
}

fn install_conditional_formatting_fixture(project: &Path, title: &str) {
    let path = visual_json_by_title(project, title);
    patch_json(&path, |visual| {
        visual["visual"]["objects"]["dataPoint"] = json!([{
            "properties": {
                "fill": { "solid": { "color": { "expr": { "Literal": { "Value": "'#2563EB'" } } } } },
                "conditionalFormatting": {
                    "rules": [{
                        "condition": { "min": 0, "max": 100 },
                        "color": "#16A34A"
                    }],
                    "gradient": { "min": "#F59E0B", "max": "#16A34A" }
                }
            }
        }]);
    });
}

fn categorical_filter(name: &str, table: &str, column: &str, value: &str) -> Value {
    let alias = table
        .chars()
        .find(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase().to_string())
        .unwrap_or_else(|| "t".to_string());
    json!({
        "name": name,
        "type": "Categorical",
        "field": {
            "Column": {
                "Expression": { "SourceRef": { "Entity": table } },
                "Property": column
            }
        },
        "filter": {
            "Version": 2,
            "From": [{ "Name": alias, "Entity": table, "Type": 0 }],
            "Where": [{
                "Condition": {
                    "In": {
                        "Expressions": [{
                            "Column": {
                                "Expression": { "SourceRef": { "Source": alias } },
                                "Property": column
                            }
                        }],
                        "Values": [[{ "Literal": { "Value": format!("'{}'", value.replace('\'', "''")) } }]]
                    }
                }
            }]
        },
        "howCreated": "User"
    })
}

fn install_slicer_fixture(project: &Path, title: &str) {
    let path = visual_json_by_title(project, title);
    patch_json(&path, |visual| {
        visual["name"] = json!("VisualContainerBranchSlicer");
        visual["annotations"] =
            json!([{ "name": "powerbi-cli.placeholderTitle", "value": "Branch Slicer" }]);
        visual["visual"]["visualType"] = json!("slicer");
        visual["visual"]["query"] = json!({
            "queryState": {
                "Values": {
                    "projections": [{
                        "field": {
                            "Column": {
                                "Expression": { "SourceRef": { "Entity": "DimBranch" } },
                                "Property": "Branch"
                            }
                        },
                        "queryRef": "DimBranch.Branch",
                        "nativeQueryRef": "Branch",
                        "displayName": "Branch"
                    }]
                }
            }
        });
        let mut filter = categorical_filter(
            "SlicerBranchSelection",
            "DimBranch",
            "Branch",
            "Construction",
        );
        filter["filterExpressionMetadata"] = json!({
            "cachedValueItems": [{
                "valueMap": { "0": "Construction" },
                "identities": []
            }]
        });
        visual["filterConfig"]["filters"] = json!([filter]);
        visual["visual"]["objects"] = json!({
            "general": [{
                "properties": {
                    "orientation": { "expr": { "Literal": { "Value": "'vertical'" } } },
                    "altText": { "expr": { "Literal": { "Value": "'Branch slicer'" } } }
                }
            }]
        });
    });
}

fn install_bookmark_fixtures(project: &Path) {
    let bookmarks_dir = report_dir(project).join("definition").join("bookmarks");
    fs::create_dir_all(&bookmarks_dir).expect("bookmarks dir");
    let first_page = page_names(project).first().expect("first page").to_string();
    write_json(
        &bookmarks_dir.join("bookmarks.json"),
        &json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmarksMetadata/1.0.0/schema.json",
            "items": [
                { "name": "BookmarkOverview" },
                { "name": "BookmarkScatter" }
            ]
        }),
    );
    for (name, display_name) in [
        ("BookmarkOverview", "Overview Bookmark"),
        ("BookmarkScatter", "Scatter Bookmark"),
    ] {
        write_json(
            &bookmarks_dir.join(format!("{name}.bookmark.json")),
            &json!({
                "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmark/2.1.0/schema.json",
                "displayName": display_name,
                "name": name,
                "options": {},
                "explorationState": {
                    "version": "1.3",
                    "activeSection": first_page,
                    "sections": {}
                }
            }),
        );
    }
}
