//! Report visual object-property and projection display-name mutation tests.

mod common;

use common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

fn visual_handle(project: &Path, visual_path: &Path) -> String {
    let page_name = first_page_name(project);
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(visual_path).expect("visual json"))
            .expect("parse visual json");
    let name = visual["name"].as_str().expect("visual name");
    format!("visual:{page_name}:{name}")
}

fn card_visual_json(project: &Path) -> PathBuf {
    let visuals_dir = first_page_json(project)
        .parent()
        .expect("page dir")
        .join("visuals");
    let mut visual_json_paths = fs::read_dir(visuals_dir)
        .expect("visuals dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().expect("file type").is_dir())
        .map(|entry| entry.path().join("visual.json"))
        .collect::<Vec<_>>();
    visual_json_paths.sort();
    visual_json_paths
        .into_iter()
        .find(|path| {
            let value: Value =
                serde_json::from_str(&fs::read_to_string(path).expect("visual json"))
                    .expect("parse visual json");
            value["visual"]["visualType"].as_str() == Some("card")
        })
        .unwrap_or_else(|| first_visual_json(project))
}

fn first_projection_role(visual: &Value) -> String {
    let query_state = visual["visual"]["query"]["queryState"]
        .as_object()
        .expect("queryState");
    let mut roles = query_state.keys().cloned().collect::<Vec<_>>();
    roles.sort();
    roles
        .into_iter()
        .find(|role| {
            query_state[role]["projections"]
                .as_array()
                .is_some_and(|projections| !projections.is_empty())
        })
        .expect("projection role")
}

fn set_object_args<'a>(
    project: &'a str,
    handle: &'a str,
    object: &'a str,
    property: &'a str,
    value: &'a str,
    mode: &'a str,
) -> Vec<&'a str> {
    vec![
        "report",
        "visuals",
        "set-object",
        "--project",
        project,
        "--handle",
        handle,
        "--object",
        object,
        "--property",
        property,
        "--value",
        value,
        mode,
        "--json",
    ]
}

#[test]
fn set_object_dry_run_plans_changes_and_leaves_file_untouched() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visual_path = first_visual_json(&project);
    let handle = visual_handle(&project, &visual_path);
    let before = fs::read_to_string(&visual_path).expect("visual before");

    let output = run_powerbi(&set_object_args(
        project_arg,
        &handle,
        "categoryLabels",
        "fontSize",
        "20",
        "--dry-run",
    ));
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert!(output.stderr.trim().is_empty(), "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        Value::from("powerbi-cli.report.visuals.objectMutation.v1")
    );
    assert_eq!(value["dryRun"], Value::Bool(true));
    assert_eq!(value["mode"], Value::from("dry-run"));
    assert_eq!(value["changes"][0]["before"], Value::Null);
    assert_eq!(
        value["changes"][0]["after"],
        json!({"expr":{"Literal":{"Value":"20D"}}})
    );
    assert!(
        value["readbackCommand"]
            .as_str()
            .unwrap_or_default()
            .contains("report visuals show")
    );
    assert!(
        value["validateCommand"]
            .as_str()
            .unwrap_or_default()
            .contains("validate --strict")
    );
    assert_eq!(
        fs::read_to_string(&visual_path).expect("visual after dry-run"),
        before
    );
}

#[test]
fn set_object_in_place_writes_card_literals_and_stays_strict_valid() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visual_path = card_visual_json(&project);
    let handle = visual_handle(&project, &visual_path);

    let font_size = run_powerbi(&set_object_args(
        project_arg,
        &handle,
        "categoryLabels",
        "fontSize",
        "20",
        "--in-place",
    ));
    assert_eq!(font_size.code, 0, "stderr: {}", font_size.stderr);
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(&visual_path).expect("visual json"))
            .expect("parse visual json");
    assert_eq!(
        visual["visual"]["objects"]["categoryLabels"][0]["properties"]["fontSize"],
        json!({"expr":{"Literal":{"Value":"20D"}}})
    );

    let word_wrap = run_powerbi(&set_object_args(
        project_arg,
        &handle,
        "categoryLabels",
        "wordWrap",
        "true",
        "--in-place",
    ));
    assert_eq!(word_wrap.code, 0, "stderr: {}", word_wrap.stderr);
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(&visual_path).expect("visual json"))
            .expect("parse visual json");
    assert_eq!(
        visual["visual"]["objects"]["categoryLabels"][0]["properties"]["wordWrap"],
        json!({"expr":{"Literal":{"Value":"true"}}})
    );
    assert!(
        fs::read_to_string(&visual_path)
            .expect("visual json text")
            .contains("\"Value\": \"true\""),
        "wordWrap must encode a bare true literal"
    );

    let title = run_powerbi(&set_object_args(
        project_arg,
        &handle,
        "title",
        "text",
        "Rate zuletzt (BU je 1'000 FTE)",
        "--in-place",
    ));
    assert_eq!(title.code, 0, "stderr: {}", title.stderr);
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(&visual_path).expect("visual json"))
            .expect("parse visual json");
    assert_eq!(
        visual["visual"]["visualContainerObjects"]["title"][0]["properties"]["text"],
        json!({"expr":{"Literal":{"Value":"'Rate zuletzt (BU je 1''000 FTE)'"}}})
    );
    assert_strict_valid(&project);
}

#[test]
fn set_object_preserves_sibling_property_in_the_same_slot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visual_path = card_visual_json(&project);
    let handle = visual_handle(&project, &visual_path);

    let show = run_powerbi(&set_object_args(
        project_arg,
        &handle,
        "categoryLabels",
        "show",
        "false",
        "--in-place",
    ));
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);

    let font_size = run_powerbi(&set_object_args(
        project_arg,
        &handle,
        "categoryLabels",
        "fontSize",
        "20",
        "--in-place",
    ));
    assert_eq!(font_size.code, 0, "stderr: {}", font_size.stderr);
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(&visual_path).expect("visual json"))
            .expect("parse visual json");
    let properties = &visual["visual"]["objects"]["categoryLabels"][0]["properties"];
    assert_eq!(
        properties["show"],
        json!({"expr":{"Literal":{"Value":"false"}}})
    );
    assert_eq!(
        properties["fontSize"],
        json!({"expr":{"Literal":{"Value":"20D"}}})
    );
    assert_eq!(
        visual["visual"]["objects"]["categoryLabels"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn set_object_fails_closed_for_unknown_pairs_and_type_mismatches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visual_path = first_visual_json(&project);
    let handle = visual_handle(&project, &visual_path);
    let before = fs::read_to_string(&visual_path).expect("visual before");

    let unknown = run_powerbi(&set_object_args(
        project_arg,
        &handle,
        "legend",
        "show",
        "true",
        "--dry-run",
    ));
    assert_eq!(unknown.code, 2);
    assert!(
        unknown.stdout.trim().is_empty(),
        "stdout: {}",
        unknown.stdout
    );
    let unknown_err = stderr_json(&unknown);
    assert_eq!(
        unknown_err["error"]["code"],
        Value::from("unsupported_feature")
    );
    assert_eq!(unknown_err["error"]["exitCode"], Value::from(2));
    let unknown_message = unknown_err["error"]["message"].as_str().unwrap_or_default();
    assert!(unknown_message.contains("legend.show"), "{unknown_message}");
    assert!(
        unknown_message.contains("categoryLabels.fontSize"),
        "{unknown_message}"
    );

    let mismatch = run_powerbi(&set_object_args(
        project_arg,
        &handle,
        "categoryLabels",
        "fontSize",
        "true",
        "--dry-run",
    ));
    assert_eq!(mismatch.code, 2);
    assert!(
        mismatch.stdout.trim().is_empty(),
        "stdout: {}",
        mismatch.stdout
    );
    let mismatch_err = stderr_json(&mismatch);
    assert_eq!(mismatch_err["error"]["code"], Value::from("invalid_args"));
    assert_eq!(mismatch_err["error"]["exitCode"], Value::from(2));
    assert!(
        mismatch_err["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("must be a number")
    );
    assert_eq!(
        fs::read_to_string(&visual_path).expect("visual after fail-closed"),
        before
    );
}

#[test]
fn set_display_name_sets_clears_and_rejects_missing_roles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visual_path = card_visual_json(&project);
    let handle = visual_handle(&project, &visual_path);
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(&visual_path).expect("visual json"))
            .expect("parse visual json");
    let role = first_projection_role(&visual);

    let set = run_powerbi(&[
        "report",
        "visuals",
        "set-display-name",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--role",
        &role,
        "--display-name",
        "Rate zuletzt (BU je 1'000 FTE)",
        "--in-place",
        "--json",
    ]);
    assert_eq!(set.code, 0, "stderr: {}", set.stderr);
    let set_json = stdout_json(&set);
    assert_eq!(
        set_json["schema"],
        Value::from("powerbi-cli.report.visuals.displayNameMutation.v1")
    );
    assert_eq!(
        set_json["changes"][0]["after"],
        Value::from("Rate zuletzt (BU je 1'000 FTE)")
    );
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(&visual_path).expect("visual json"))
            .expect("parse visual json");
    assert_eq!(
        visual["visual"]["query"]["queryState"][&role]["projections"][0]["displayName"],
        Value::from("Rate zuletzt (BU je 1'000 FTE)")
    );

    let clear = run_powerbi(&[
        "report",
        "visuals",
        "set-display-name",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--role",
        &role,
        "--clear",
        "--in-place",
        "--json",
    ]);
    assert_eq!(clear.code, 0, "stderr: {}", clear.stderr);
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(&visual_path).expect("visual json"))
            .expect("parse visual json");
    assert!(
        visual["visual"]["query"]["queryState"][&role]["projections"][0]
            .get("displayName")
            .is_none()
    );

    let bad_role = run_powerbi(&[
        "report",
        "visuals",
        "set-display-name",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--role",
        "Series",
        "--display-name",
        "unused",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(bad_role.code, 2);
    assert!(
        bad_role.stdout.trim().is_empty(),
        "stdout: {}",
        bad_role.stdout
    );
    let error = stderr_json(&bad_role);
    assert_eq!(error["error"]["code"], Value::from("invalid_args"));
    let message = error["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("present roles"),
        "expected present-roles listing: {message}"
    );
    assert!(message.contains(&role), "{message}");
}
