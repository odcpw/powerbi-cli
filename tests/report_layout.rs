mod common;

use common::{
    assert_json_snapshot, first_visual_json, patch_json, run_powerbi, scaffold_sales, stdout_json,
};
use serde_json::Value;
use std::fs;
use std::path::Path;

fn preview_page(value: &Value) -> Value {
    let mut page = value["preview"]["pages"][0].clone();
    // Page summaries intentionally carry absolute readback paths.  The grid
    // preview itself is path-free, so omit that summary from the golden.
    page.as_object_mut()
        .expect("preview page object")
        .remove("page");
    page
}

#[test]
fn named_template_dry_run_returns_svg_free_slots_and_does_not_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let visual_path = project
        .join("SalesOperations.Report/definition/pages/ReportSectionOverview/visuals")
        .read_dir()
        .expect("visual directory")
        .next()
        .expect("visual")
        .expect("visual entry")
        .path()
        .join("visual.json");
    let before = fs::read_to_string(&visual_path).expect("read visual");
    let project_arg = project.to_str().expect("project path");
    let output = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "overview",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["schema"], "powerbi-cli.report.layout.autoMutation.v1");
    assert_eq!(value["dryRun"], true);
    assert_eq!(value["preview"]["svg"], false);
    assert_eq!(value["preview"]["pages"][0]["template"]["name"], "overview");
    assert_eq!(value["preview"]["pages"][0]["grid"]["columns"], 12);
    assert!(
        value["preview"]["pages"][0]["slots"]
            .as_array()
            .expect("slots")
            .iter()
            .any(|slot| slot["name"] == "rail")
    );
    assert_eq!(
        fs::read_to_string(&visual_path).expect("read visual"),
        before
    );
}

#[test]
fn named_template_out_dir_is_deterministic_and_keeps_source_unchanged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let out_dir = temp.path().join("laid_out");
    let out_arg = out_dir.to_str().expect("out path");
    let first = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "kpi-strip-trend-breakdown",
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let first_json = stdout_json(&first);
    assert_eq!(first_json["mode"], "out-dir");
    assert_eq!(first_json["dryRun"], false);
    assert!(out_dir.is_dir());
    let source_again = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "kpi-strip-trend-breakdown",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(source_again.code, 0, "stderr: {}", source_again.stderr);
    assert_eq!(
        preview_page(&first_json),
        preview_page(&stdout_json(&source_again)),
        "out-dir and source dry-run must resolve identical grid coordinates"
    );
}

#[test]
fn legacy_preset_aliases_are_byte_identical_to_named_templates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let aliases = [
        ("overview", "overview"),
        ("dashboard", "overview"),
        ("analysis", "time-series"),
        ("focus", "time-series"),
        ("detail", "drillthrough-detail"),
        ("details", "drillthrough-detail"),
        ("grid", "kpi-strip-trend-breakdown"),
    ];

    for (preset, template) in aliases {
        let preset_output = run_powerbi(&[
            "report",
            "layout",
            "auto",
            "--project",
            project_arg,
            "--preset",
            preset,
            "--dry-run",
            "--json",
        ]);
        assert_eq!(
            preset_output.code, 0,
            "preset {preset} stderr: {}",
            preset_output.stderr
        );
        let template_output = run_powerbi(&[
            "report",
            "layout",
            "auto",
            "--project",
            project_arg,
            "--template",
            template,
            "--dry-run",
            "--json",
        ]);
        assert_eq!(
            template_output.code, 0,
            "template {template} stderr: {}",
            template_output.stderr
        );
        assert_eq!(
            preset_output.stdout, template_output.stdout,
            "legacy --preset {preset} must be byte-identical to --template {template}"
        );
    }
}

#[test]
fn post_build_layout_matches_resolved_slot_coordinates_and_replays_as_a_noop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("built_sales");
    let project_arg = project.to_str().expect("project path");
    let build = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        "examples/sales.dashboard.v2.json",
        "--out-dir",
        project_arg,
        "--json",
    ]);
    assert_eq!(build.code, 0, "build stderr: {}", build.stderr);

    let planned = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "overview",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(planned.code, 0, "planned stderr: {}", planned.stderr);
    let planned_json = stdout_json(&planned);
    let page = &planned_json["preview"]["pages"][0];
    let slots = page["slots"].as_array().expect("resolved slots");
    for assignment in page["assignments"].as_array().expect("assignments") {
        let slot_name = assignment["slot"].as_str().expect("assignment slot");
        let slot = slots
            .iter()
            .find(|slot| slot["name"] == slot_name)
            .unwrap_or_else(|| panic!("missing resolved slot {slot_name}"));
        for field in ["x", "y", "width", "height"] {
            assert_eq!(
                assignment["position"][field], slot["position"][field],
                "assignment {slot_name} must use the resolved grid {field}"
            );
        }
    }

    let applied = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "overview",
        "--in-place",
        "--json",
    ]);
    assert_eq!(applied.code, 0, "apply stderr: {}", applied.stderr);
    let applied_json = stdout_json(&applied);
    assert_eq!(
        applied_json["preview"]["pages"][0]["assignments"], page["assignments"],
        "in-place application must use the same slot coordinates as its dry-run"
    );

    let replay = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "overview",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(replay.code, 0, "replay stderr: {}", replay.stderr);
    let replay_json = stdout_json(&replay);
    assert_eq!(
        replay_json["changes"],
        Value::Array(Vec::new()),
        "replaying an applied grid layout must not produce additional writes"
    );
    assert_eq!(
        replay_json["preview"]["pages"][0]["assignments"], page["assignments"],
        "replayed layout must retain the original slot coordinates"
    );
}

#[test]
fn unknown_template_and_conflicting_layout_selectors_are_pointer_rich_refusals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let unknown = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "not-a-template",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unknown.code, 2);
    let unknown_error: Value = serde_json::from_str(unknown.stderr.trim()).expect("error JSON");
    assert_eq!(unknown_error["error"]["code"], "invalid_args");
    assert_eq!(unknown_error["error"]["pointer"], "/template");
    assert!(
        unknown_error["error"]["hint"]
            .as_str()
            .expect("hint")
            .contains("named templates")
    );

    let conflict = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "overview",
        "--preset",
        "grid",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(conflict.code, 2);
    let conflict_error: Value = serde_json::from_str(conflict.stderr.trim()).expect("error JSON");
    assert_eq!(conflict_error["error"]["pointer"], "/template");
}

#[test]
fn family_mismatch_is_a_structured_warning_and_grid_overrides_are_echoed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let output = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "time-series",
        "--grid",
        "columns=12,gutter=20,margin=32,rowUnit=8",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["layoutPlan"]["grid"]["margin"], Value::from(32.0));
    let warnings = value["warnings"].as_array().expect("warnings");
    assert!(warnings.iter().any(|warning| {
        warning["code"] == "design.slot_family_mismatch"
            && warning["pointer"]
                .as_str()
                .is_some_and(|pointer| pointer.starts_with("/pages/0/visuals/"))
    }));
}

#[test]
fn three_named_templates_have_stable_golden_slot_layouts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    for template in ["kpi-strip-trend-breakdown", "overview", "time-series"] {
        let output = run_powerbi(&[
            "report",
            "layout",
            "auto",
            "--project",
            project_arg,
            "--template",
            template,
            "--dry-run",
            "--json",
        ]);
        assert_eq!(output.code, 0, "{template} stderr: {}", output.stderr);
        let value = stdout_json(&output);
        assert_json_snapshot(&format!("report-layout-{template}"), &preview_page(&value));
    }
}

/// The page and visual directory names double as the stable name components
/// of a `visual:<page>:<name>` handle.
fn first_visual_identifiers(project: &Path) -> (String, String) {
    let visual_json = first_visual_json(project);
    let visual_dir = visual_json.parent().expect("visual dir");
    let visual_name = visual_dir
        .file_name()
        .expect("visual dir name")
        .to_str()
        .expect("visual name utf8")
        .to_string();
    let page_name = visual_dir
        .parent()
        .expect("visuals dir")
        .parent()
        .expect("page dir")
        .file_name()
        .expect("page dir name")
        .to_str()
        .expect("page name utf8")
        .to_string();
    (page_name, visual_name)
}

fn clone_first_visual(project_arg: &str, source_handle: &str, name: &str, rect: [f64; 4]) {
    let [x, y, width, height] = rect.map(|value| value.to_string());
    let output = run_powerbi(&[
        "report",
        "visuals",
        "clone",
        "--project",
        project_arg,
        "--handle",
        source_handle,
        "--name",
        name,
        "--x",
        &x,
        "--y",
        &y,
        "--width",
        &width,
        "--height",
        &height,
        "--in-place",
        "--json",
    ]);
    assert_eq!(output.code, 0, "clone {name} stderr: {}", output.stderr);
}

fn off_grid_findings(project_arg: &str) -> Vec<Value> {
    let audit = run_powerbi(&[
        "report",
        "audit",
        "--project",
        project_arg,
        "--rules",
        "design",
        "--json",
    ]);
    assert_eq!(audit.code, 0, "audit stderr: {}", audit.stderr);
    stdout_json(&audit)["findings"]
        .as_array()
        .expect("audit findings")
        .iter()
        .filter(|finding| finding["ruleId"] == "design.visual_off_grid")
        .cloned()
        .collect()
}

#[test]
fn snap_clears_off_grid_findings_on_a_page_too_dense_for_any_template() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let (page_name, visual_name) = first_visual_identifiers(&project);
    let source_handle = format!("visual:{page_name}:{visual_name}");

    // Twelve extra off-grid visuals overflow even the largest slot template.
    for index in 0..12 {
        clone_first_visual(
            project_arg,
            &source_handle,
            &format!("SnapDense{index}"),
            [
                25.0 + f64::from(index % 3) * 30.0,
                190.0 + f64::from(index) * 35.0,
                210.0,
                30.0,
            ],
        );
    }

    let before = off_grid_findings(project_arg);
    assert!(
        before.len() >= 12,
        "expected the cloned visuals to be off grid: {before:?}"
    );
    // The finding says what the predicate checks and names the guides.
    let message = before[0]["message"].as_str().expect("finding message");
    for fragment in ["column start guide", "column end guide", "row-unit"] {
        assert!(
            message.contains(fragment),
            "finding message must mention {fragment:?}: {message}"
        );
    }

    // Slot templates cannot fit the dense page; the refusal points at --snap.
    let template_attempt = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "kpi-strip-trend-breakdown",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(template_attempt.code, 2);
    let template_error: Value =
        serde_json::from_str(template_attempt.stderr.trim()).expect("error JSON");
    assert_eq!(template_error["error"]["code"], "invalid_args");
    assert!(
        template_error["error"]["hint"]
            .as_str()
            .expect("hint")
            .contains("--snap")
    );

    let snapped = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--snap",
        "--in-place",
        "--json",
    ]);
    assert_eq!(snapped.code, 0, "snap stderr: {}", snapped.stderr);
    let snapped_json = stdout_json(&snapped);
    assert_eq!(snapped_json["action"], "snap");
    assert_eq!(snapped_json["snap"], true);
    assert_eq!(snapped_json["layoutPlan"]["template"], Value::Null);
    assert!(
        !snapped_json["changes"]
            .as_array()
            .expect("changes")
            .is_empty(),
        "snapping a dense off-grid page must move visuals"
    );

    assert_eq!(
        off_grid_findings(project_arg),
        Vec::<Value>::new(),
        "snap must clear every design.visual_off_grid finding"
    );

    // Replaying the snap is a byte-level no-op.
    let replay = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--snap",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(replay.code, 0, "replay stderr: {}", replay.stderr);
    assert_eq!(stdout_json(&replay)["changes"], Value::Array(Vec::new()));
}

#[test]
fn snap_reports_emergent_overlaps_instead_of_writing_them_silently() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let (page_name, visual_name) = first_visual_identifiers(&project);
    let source_handle = format!("visual:{page_name}:{visual_name}");

    // Disjoint before the snap: the left visual's right edge (170) pulls to the
    // column end guide at 216 while the right visual's left edge (172) pulls to
    // the column start guide at 128, so the snapped rectangles collide.
    patch_json(&first_visual_json(&project), |visual| {
        visual["position"]["x"] = Value::from(24.0);
        visual["position"]["y"] = Value::from(200.0);
        visual["position"]["width"] = Value::from(146.0);
        visual["position"]["height"] = Value::from(80.0);
    });
    clone_first_visual(
        project_arg,
        &source_handle,
        "SnapOverlapTwin",
        [172.0, 200.0, 148.0, 80.0],
    );

    let output = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--page",
        &page_name,
        "--snap",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let twin_handle = format!("visual:{page_name}:SnapOverlapTwin");
    let overlap = value["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .find(|warning| {
            let pair = [
                warning["visual"].as_str().unwrap_or_default(),
                warning["otherVisual"].as_str().unwrap_or_default(),
            ];
            warning["code"] == "design.visual_overlap"
                && pair.contains(&twin_handle.as_str())
                && pair.contains(&source_handle.as_str())
        });
    assert!(
        overlap.is_some(),
        "snap must report the emergent overlap between {source_handle} and {twin_handle}: {}",
        value["warnings"]
    );
    assert_eq!(
        value["preview"]["pages"][0]["invariants"]["overlapFree"], false,
        "snap preview must not claim the page is overlap-free"
    );
}

#[test]
fn snap_conflicts_with_template_and_preset_selection() {
    for selector in [["--template", "overview"], ["--preset", "grid"]] {
        let output = run_powerbi(&[
            "report",
            "layout",
            "auto",
            "--snap",
            selector[0],
            selector[1],
            "--dry-run",
            "--json",
        ]);
        assert_eq!(output.code, 2, "selector {selector:?}");
        let error: Value = serde_json::from_str(output.stderr.trim()).expect("error JSON");
        assert_eq!(error["error"]["code"], "invalid_args");
        assert_eq!(error["error"]["pointer"], "/snap");
    }
}

#[test]
fn every_named_template_has_standard_and_wide_golden_coordinates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let add_page = run_powerbi(&[
        "report",
        "pages",
        "add",
        "--project",
        project_arg,
        "--name",
        "LayoutCanvas",
        "--display-name",
        "Layout Canvas",
        "--width",
        "1280",
        "--height",
        "720",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add_page.code, 0, "stderr: {}", add_page.stderr);

    let mut golden = serde_json::Map::new();
    for template in [
        "kpi-strip-trend-breakdown",
        "overview",
        "time-series",
        "ranking",
        "distribution",
        "comparison",
        "detail-table",
        "drillthrough-detail",
        "exception-list",
        "matrix-focus",
        "scatter-focus",
    ] {
        let mut sizes = serde_json::Map::new();
        for (label, size) in [("1280x720", "1280x720"), ("1920x1080", "1920x1080")] {
            let output = run_powerbi(&[
                "report",
                "layout",
                "auto",
                "--project",
                project_arg,
                "--page",
                "LayoutCanvas",
                "--template",
                template,
                "--page-size",
                size,
                "--dry-run",
                "--json",
            ]);
            assert_eq!(output.code, 0, "{template} {size}: {}", output.stderr);
            sizes.insert(label.to_string(), preview_page(&stdout_json(&output)));
        }
        golden.insert(template.to_string(), Value::Object(sizes));
    }
    assert_json_snapshot("report-layout-all-templates", &Value::Object(golden));
}
