use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct RunOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_powerbi(args: &[&str]) -> RunOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args(args)
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

fn scaffold_sales_project(root: &Path) -> PathBuf {
    let out_dir = root.join("sales_project");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out_dir.to_str().expect("output path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    out_dir
}

#[test]
fn add_static_table_dry_run_then_in_place_and_read_back() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let args = [
        "model",
        "tables",
        "add-static",
        "--project",
        project_arg,
        "--table",
        "Metric",
        "--column",
        "Metric",
        "--values-json",
        "[\"Count\",\"Cost\"]",
    ];

    let mut dry_args = args.to_vec();
    dry_args.extend(["--dry-run", "--include-raw", "--json"]);
    let dry = run_powerbi(&dry_args);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(dry_json["dryRun"], true);
    assert_eq!(dry_json["projectModified"], false);
    assert_eq!(dry_json["tablePlan"]["rowCount"], 2);
    assert!(
        dry_json["tablePlan"]["tmdl"]
            .as_str()
            .unwrap_or_default()
            .contains("type table [Metric = text]")
    );
    let table_path = project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("Metric.tmdl");
    assert!(!table_path.exists());

    let mut apply_args = args.to_vec();
    apply_args.extend(["--in-place", "--json"]);
    let apply = run_powerbi(&apply_args);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let apply_json = stdout_json(&apply);
    assert_eq!(apply_json["projectModified"], true);
    assert_eq!(apply_json["validation"]["ok"], true);
    let text = fs::read_to_string(&table_path).expect("static table TMDL");
    assert!(text.contains("{\"Count\"}"));
    assert!(text.contains("{\"Cost\"}"));

    let show = run_powerbi(&[
        "model",
        "partitions",
        "show",
        "--project",
        project_arg,
        "--handle",
        "partition:Metric:Metric",
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(show_json["partition"]["sourceKind"], "dummyMTable");
    assert_eq!(show_json["partition"]["offlineSafety"]["status"], "safe");

    let duplicate = run_powerbi(&apply_args);
    assert_eq!(duplicate.code, 2);
    assert!(duplicate.stderr.contains("already exists"));
}

#[test]
fn add_static_table_rejects_credentials_and_duplicate_labels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    for values in ["[\"Count\",\"count\"]", "[\"password=secret\",\"Cost\"]"] {
        let output = run_powerbi(&[
            "model",
            "tables",
            "add-static",
            "--project",
            project_arg,
            "--table",
            "Metric",
            "--column",
            "Metric",
            "--values-json",
            values,
            "--dry-run",
            "--json",
        ]);
        assert_eq!(output.code, 2);
        assert!(output.stdout.trim().is_empty());
        assert!(!output.stderr.trim().is_empty());
    }
}

#[test]
fn add_static_lookup_table_dry_run_then_in_place_and_validate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let args = [
        "model",
        "tables",
        "add-static",
        "--project",
        project_arg,
        "--table",
        "DimSegment",
        "--columns-json",
        "[\"Code\",\"Label\",\"Display\"]",
        "--rows-json",
        "[[\"A\",\"Alpha\",\"A - Alpha\"],[\"B\",\"Beta\",\"B - Beta\"]]",
    ];

    let mut dry_args = args.to_vec();
    dry_args.extend(["--dry-run", "--include-raw", "--json"]);
    let dry = run_powerbi(&dry_args);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(dry_json["tablePlan"]["kind"], "staticLookupTable");
    assert_eq!(dry_json["tablePlan"]["columnCount"], 3);
    assert_eq!(dry_json["tablePlan"]["rowCount"], 2);
    assert_eq!(dry_json["tablePlan"]["uniqueFirstColumn"], true);
    assert!(
        dry_json["tablePlan"]["tmdl"]
            .as_str()
            .unwrap_or_default()
            .contains("type table [Code = text, Label = text, Display = text]")
    );

    let mut apply_args = args.to_vec();
    apply_args.extend(["--in-place", "--json"]);
    let apply = run_powerbi(&apply_args);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let apply_json = stdout_json(&apply);
    assert_eq!(apply_json["validation"]["ok"], true);

    let table_path = project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("DimSegment.tmdl");
    let text = fs::read_to_string(&table_path).expect("lookup table TMDL");
    assert!(text.contains("column Code"));
    assert!(text.contains("column Label"));
    assert!(text.contains("column Display"));
    assert!(text.contains("{\"A\", \"Alpha\", \"A - Alpha\"}"));

    let validate = run_powerbi(&["validate", "--strict", project_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
}
