use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
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

fn run_powerbi_without_oracle(args: &[&str]) -> RunOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .env_remove("POWERBI_DESKTOP_ORACLE")
        .args(args)
        .output()
        .expect("run powerbi-cli binary without Desktop oracle");
    RunOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn stdout_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stdout.trim()).expect("stdout JSON")
}

fn stderr_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stderr.trim()).expect("stderr JSON")
}

fn command_paths(value: &Value) -> Vec<String> {
    value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|command| command["path"].as_str().expect("command path").to_string())
        .collect()
}

fn command_by_path<'a>(commands: &'a [Value], path: &str) -> &'a Value {
    commands
        .iter()
        .find(|command| command["path"] == path)
        .unwrap_or_else(|| panic!("missing command catalog path {path}"))
}

fn collect_next_commands(value: &Value, commands: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key == "next" {
                    for command in child.as_array().expect("next must be an array") {
                        commands.push(
                            command
                                .as_str()
                                .expect("next entries must be command strings")
                                .to_string(),
                        );
                    }
                } else {
                    collect_next_commands(child, commands);
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_next_commands(child, commands);
            }
        }
        _ => {}
    }
}

fn assert_executable_command_template(command: &str, catalog_commands: &[Value]) {
    let words = command.split_whitespace().collect::<Vec<_>>();
    assert_eq!(
        words.first().copied(),
        Some("powerbi-cli"),
        "next entry is not an executable command template: {command}"
    );
    let mut index = 1;
    while index < words.len() {
        match words[index] {
            "--json" | "--format=json" | "-f=json" => index += 1,
            "--format" | "-f" => index += 2,
            _ => break,
        }
    }
    let command_words = &words[index..];
    let catalog_command = catalog_commands
        .iter()
        .filter_map(|candidate| {
            let path = candidate["path"].as_str()?;
            let path_words = path.split_whitespace().collect::<Vec<_>>();
            command_words
                .starts_with(&path_words)
                .then_some((path_words.len(), candidate))
        })
        .max_by_key(|(path_len, _)| *path_len)
        .map(|(_, candidate)| candidate)
        .unwrap_or_else(|| panic!("next command does not match a live capability path: {command}"));
    let path_len = catalog_command["path"]
        .as_str()
        .expect("catalog path")
        .split_whitespace()
        .count();
    let allowed_flags = catalog_command["flags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|flag| flag.split_whitespace().next())
        .filter(|flag| flag.starts_with('-'))
        .collect::<BTreeSet<_>>();
    for word in &command_words[path_len..] {
        if word.starts_with('-') && word.parse::<f64>().is_err() {
            let flag = word.split('=').next().expect("flag token");
            assert!(
                matches!(flag, "--json" | "--format" | "-f") || allowed_flags.contains(flag),
                "next command uses an uncataloged flag {flag}: {command}"
            );
        }
    }
    assert!(
        !(command.contains("--dry-run") && command.contains("--out-dir")),
        "next command combines mutually exclusive output modes: {command}"
    );
}

fn assert_error_envelope(output: &RunOutput, expected_code: &str) {
    assert!(output.stdout.trim().is_empty());
    let value = stderr_json(output);
    let top = value.as_object().expect("error envelope object");
    assert_eq!(top.len(), 1);
    let error = top["error"].as_object().expect("error object");
    assert_eq!(error["code"], expected_code);
    assert!(error["exitCode"].is_i64());
    assert!(error["message"].is_string());
    let allowed = ["code", "exitCode", "message", "hint", "suggestedCommands"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert!(error.keys().all(|key| allowed.contains(key.as_str())));
    if let Some(commands) = error.get("suggestedCommands") {
        assert!(
            commands
                .as_array()
                .expect("suggestedCommands array")
                .iter()
                .all(|command| command
                    .as_str()
                    .is_some_and(|command| command.starts_with("powerbi-cli ")))
        );
    }
}

#[test]
fn json_flag_is_accepted_after_command() {
    let output = run_powerbi(&["doctor", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert!(
        output.stderr.trim().is_empty(),
        "unexpected stderr: {}",
        output.stderr
    );
    let value = stdout_json(&output);
    assert_eq!(value["schema"], Value::from("powerbi-cli.doctor.v1"));
    assert_eq!(value["tool"], Value::from("powerbi-cli"));
    assert_eq!(value["ok"], Value::Bool(true));
    assert_eq!(value["exitCode"], Value::from(0));
    assert!(value["formatAssumptions"]["semanticModelFormat"].is_string());
    let checks = value["checks"].as_array().expect("doctor checks");
    assert!(checks.iter().any(|check| check["id"] == "powerBiDesktop"
        && check["status"].as_str().is_some()
        && check["severity"].as_str().is_some()));
    assert!(checks.iter().any(|check| check["id"] == "desktopProofLevel"
        && check["currentLevel"] == "desktop-window"
        && check["requiredCompatibilityLevel"] == "desktop-canvas-refresh"));
}

#[test]
fn scaffold_accepts_suffix_json_flag() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["ok"], Value::Bool(true));
    assert!(value["next"].as_array().expect("next").len() >= 2);
    assert!(value["next"].as_array().expect("next").iter().all(|entry| {
        entry
            .as_str()
            .is_some_and(|entry| entry.starts_with("powerbi-cli "))
    }));
    assert!(
        value["instructions"].as_array().expect("instructions")[0]
            .as_str()
            .unwrap_or_default()
            .starts_with("Open ")
    );
}

#[test]
fn capabilities_include_agent_contract_metadata() {
    let output = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["contractVersion"],
        Value::from("powerbi-cli.agent-capabilities.v1")
    );
    assert!(value["exitCodes"].as_array().expect("exitCodes").len() >= 5);
    assert_eq!(value["featurePolicy"]["noFakeFallbacks"], Value::Bool(true));
    assert_eq!(
        value["responseShapes"]["error"]["requiredFields"],
        json!(["error.code", "error.exitCode", "error.message"])
    );
    assert_eq!(
        value["responseShapes"]["success"]["mutationResults"]["requiredFields"],
        json!(["changes"])
    );
    assert!(
        value["diagnosticCodes"]
            .as_array()
            .expect("diagnosticCodes")
            .iter()
            .any(|code| code["code"] == "unsupported_feature")
    );
    assert!(
        value["globalFlags"]
            .as_array()
            .expect("globalFlags")
            .iter()
            .any(|flag| flag["flag"] == "--json" && flag["acceptedAnywhere"] == true)
    );
    assert!(
        value["architectureGuardrails"]
            .as_array()
            .expect("architectureGuardrails")
            .iter()
            .any(|rule| rule.as_str().unwrap_or_default().contains("src/main.rs"))
    );
    assert!(
        value["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .any(|command| command["path"] == "robot-docs guide"
                && command["followUpFields"].is_array())
    );
    let commands = value["commands"].as_array().expect("commands");
    for path in [
        "schema validate",
        "schema normalize",
        "profile infer",
        "profile validate",
        "profile summarize",
        "report build",
        "report spec validate",
        "report plan",
        "model roles show",
        "model perspectives show",
        "model cultures show",
        "model expressions show",
    ] {
        assert!(
            commands.iter().any(|command| command["path"] == path),
            "capabilities missing command path {path}"
        );
    }
    let report_build = commands
        .iter()
        .find(|command| command["path"] == "report build")
        .expect("report build command");
    assert_eq!(report_build["mutates"], Value::Bool(true));
    assert_eq!(report_build["requiresOutput"], Value::Bool(true));
    assert!(
        report_build["flags"]
            .as_array()
            .expect("report build flags")
            .iter()
            .any(|flag| flag == "--schema <schema.json>")
    );
    assert!(
        report_build["followUpFields"]
            .as_array()
            .expect("report build followUpFields")
            .iter()
            .any(|field| field == "changes[].after")
    );
    for path in [
        "schema normalize",
        "fixture normalize",
        "profile infer",
        "report themes extract",
        "report style extract",
        "report visuals formatting extract",
        "handoff rebind-plan",
    ] {
        let writer = command_by_path(commands, path);
        assert_eq!(writer["readOnly"], Value::Bool(false), "{path}");
        assert_eq!(writer["mutates"], Value::Bool(true), "{path}");
        assert_eq!(writer["mutatesProject"], Value::Bool(false), "{path}");
    }
    for path in [
        "model calculated-columns delete",
        "model measures delete",
        "model relationships delete",
        "report pages delete-empty",
        "report drillthrough clear",
        "report bookmarks delete",
        "report filters delete",
        "report filters clear",
        "report slicers clear",
        "report visuals delete",
    ] {
        assert_eq!(
            command_by_path(commands, path)["confirmRequiredForInPlace"],
            Value::Bool(true),
            "missing destructive confirmation metadata for {path}"
        );
    }
    assert!(
        report_build["followUpFields"]
            .as_array()
            .expect("report build followUpFields")
            .iter()
            .any(|field| field == "fixtureNormalizeCommand")
    );
    assert!(
        value["schemaManifest"]["dashboardSpecFields"]
            .as_array()
            .expect("dashboard spec fields")
            .iter()
            .any(|field| field == "pages[].visuals[].bindings[].field")
    );
    assert!(
        value["schemaManifest"]["profileFields"]
            .as_array()
            .expect("profile fields")
            .iter()
            .any(|field| field == "candidates.factTables")
    );
    assert_eq!(
        value["schemaManifest"]["semanticModelHandleEncoding"]["componentEscapes"],
        json!([
            {"character": "%", "encoding": "%25"},
            {"character": ":", "encoding": "%3A"}
        ])
    );
    let features = commands
        .iter()
        .find(|command| command["path"] == "features list")
        .expect("features list command");
    assert_eq!(features["readOnly"], Value::Bool(true));
    assert!(
        value["schemaManifest"]["featureCatalogFields"]
            .as_array()
            .expect("feature catalog fields")
            .iter()
            .any(|field| field == "refusalCode")
    );
    let diff = commands
        .iter()
        .find(|command| command["path"] == "diff")
        .expect("diff command");
    assert_eq!(diff["readOnly"], Value::Bool(true));
    assert!(
        diff["followUpFields"]
            .as_array()
            .expect("followUpFields")
            .iter()
            .any(|field| field == "changes[].handle")
    );
    assert!(
        diff["flags"]
            .as_array()
            .expect("diff flags")
            .iter()
            .any(|flag| flag == "--scope model.calculatedColumns")
    );
    assert!(
        diff["flags"]
            .as_array()
            .expect("diff flags")
            .iter()
            .any(|flag| flag == "--scope model.relationships")
    );

    let add_calculated_column = commands
        .iter()
        .find(|command| command["path"] == "model calculated-columns add")
        .expect("model calculated-columns add command");
    assert_eq!(add_calculated_column["mutates"], Value::Bool(true));
    assert_eq!(add_calculated_column["requiresOutput"], Value::Bool(true));
    assert!(
        add_calculated_column["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--dry-run")
    );
    assert!(
        add_calculated_column["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--data-type <type>")
    );
    assert!(
        add_calculated_column["followUpFields"]
            .as_array()
            .expect("followUpFields")
            .iter()
            .any(|field| field == "readbackCommand")
    );

    let add_measure = commands
        .iter()
        .find(|command| command["path"] == "model measures add")
        .expect("model measures add command");
    assert_eq!(add_measure["mutates"], Value::Bool(true));
    assert!(
        add_measure["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--dry-run")
    );
    assert!(
        add_measure["followUpFields"]
            .as_array()
            .expect("followUpFields")
            .iter()
            .any(|field| field == "readbackCommand")
    );

    let add_relationship = commands
        .iter()
        .find(|command| command["path"] == "model relationships add")
        .expect("model relationships add command");
    assert_eq!(add_relationship["mutates"], Value::Bool(true));
    assert_eq!(add_relationship["requiresOutput"], Value::Bool(true));
    assert!(
        add_relationship["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--from-table <table>")
    );
    assert!(
        add_relationship["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--dry-run")
    );

    let show_relationship = commands
        .iter()
        .find(|command| command["path"] == "model relationships show")
        .expect("model relationships show command");
    assert_eq!(show_relationship["readOnly"], Value::Bool(true));
    assert!(
        show_relationship["followUpFields"]
            .as_array()
            .expect("followUpFields")
            .iter()
            .any(|field| field == "relationship.from")
    );

    let visual_catalog = commands
        .iter()
        .find(|command| command["path"] == "report visuals catalog")
        .expect("report visuals catalog command");
    assert_eq!(visual_catalog["readOnly"], Value::Bool(true));
    assert!(
        visual_catalog["supportedVisualTypes"]
            .as_array()
            .expect("supported visual types")
            .iter()
            .any(|visual_type| visual_type == "barChart")
    );
    assert!(
        value["schemaManifest"]["visualCatalogFields"]
            .as_array()
            .expect("visual catalog fields")
            .iter()
            .any(|field| field == "visualTypes[].roles")
    );
}

#[test]
fn help_json_command_paths_match_capabilities_catalog() {
    let help = run_powerbi(&["--json"]);
    assert_eq!(help.code, 0, "stderr: {}", help.stderr);
    let help_value = stdout_json(&help);
    let help_commands = help_value["commands"]
        .as_array()
        .expect("help commands")
        .iter()
        .map(|path| path.as_str().expect("help command path").to_string())
        .collect::<Vec<_>>();

    let capabilities = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let catalog_value = stdout_json(&capabilities);
    assert_eq!(help_commands, command_paths(&catalog_value));
}

#[test]
fn version_is_first_class_catalog_command() {
    let output = run_powerbi(&["version", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert!(
        output.stderr.trim().is_empty(),
        "unexpected stderr: {}",
        output.stderr
    );
    let value = stdout_json(&output);
    assert_eq!(value["tool"], Value::from("powerbi-cli"));
    assert_eq!(value["binary"], Value::from("powerbi-cli"));
    assert!(
        value["version"]
            .as_str()
            .is_some_and(|version| !version.is_empty())
    );
    assert_eq!(
        value["contractVersion"],
        Value::from("powerbi-cli.agent-capabilities.v1")
    );

    let capabilities = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let catalog_value = stdout_json(&capabilities);
    let commands = catalog_value["commands"].as_array().expect("commands");
    let version = command_by_path(commands, "version");
    assert_eq!(version["readOnly"], Value::Bool(true));
    assert_eq!(version["mutates"], Value::Bool(false));
    assert_eq!(
        version["outputSchema"],
        Value::from("powerbi-cli.version.v1")
    );
}

#[test]
fn first_class_dispatch_roots_and_agent_commands_are_cataloged() {
    let output = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let commands = value["commands"].as_array().expect("commands");

    for path in [
        "version",
        "capabilities",
        "features list",
        "robot-docs guide",
        "--robot-triage",
        "robot-triage",
        "doctor",
        "desktop open-check",
        "desktop screenshot",
        "fixture normalize",
        "fixture verify",
        "scaffold",
        "schema validate",
        "profile infer",
        "inspect",
        "lint",
        "diff",
        "model dax bridge-plan",
        "model dax execute",
        "report tree",
        "report find",
        "report cat",
        "report query",
        "report audit",
        "report sanitize plan",
        "report sanitize apply",
        "handoff check",
        "validate",
    ] {
        command_by_path(commands, path);
    }

    let robot_alias = command_by_path(commands, "robot-triage");
    assert_eq!(robot_alias["jsonOnly"], Value::Bool(true));
    assert_eq!(robot_alias["outputSchema"], Value::from("robotTriage.v1"));

    let dax_bridge = command_by_path(commands, "model dax bridge-plan");
    assert_eq!(dax_bridge["readOnly"], Value::Bool(true));
    assert!(
        dax_bridge["tags"]
            .as_array()
            .expect("dax tags")
            .iter()
            .any(|tag| tag == "no-fallback")
    );

    let dax_execute = command_by_path(commands, "model dax execute");
    assert_eq!(dax_execute["readOnly"], Value::Bool(true));
    assert_eq!(dax_execute["returnsModelData"], Value::Bool(true));
    assert!(
        dax_execute["explicitOptIn"]
            .as_array()
            .expect("DAX execute opt-ins")
            .iter()
            .any(|value| value == "--allow-data-read")
    );

    let sanitize_apply = command_by_path(commands, "report sanitize apply");
    assert_eq!(sanitize_apply["mutates"], Value::Bool(true));
    assert_eq!(
        sanitize_apply["confirmRequiredForInPlace"],
        Value::Bool(true)
    );

    assert!(
        value["schemaManifest"]["reportObjectTreeFields"]
            .as_array()
            .expect("report object tree fields")
            .iter()
            .any(|field| field == "objects[].handle")
    );
    assert!(
        value["schemaManifest"]["reportSanitizePlanFields"]
            .as_array()
            .expect("report sanitize plan fields")
            .iter()
            .any(|field| field == "confirmToken")
    );
    assert!(
        value["schemaManifest"]["modelDaxBridgePlanFields"]
            .as_array()
            .expect("model dax bridge plan fields")
            .iter()
            .any(|field| field == "bridge.noFakeFallbacks")
    );
    assert!(
        value["schemaManifest"]["modelDaxExecuteFields"]
            .as_array()
            .expect("model DAX execute fields")
            .iter()
            .any(|field| field == "runtime.temporaryFilesRemoved")
    );
}

#[test]
fn ok_false_value_payloads_exit_nonzero_and_get_exit_code() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path();
    let report_dir = project.join("Broken.Report");
    fs::create_dir_all(&report_dir).expect("create report dir");
    fs::write(
        project.join("Broken.pbip"),
        r#"{"artifacts":[{"report":{"path":"Broken.Report"}}]}"#,
    )
    .expect("write pbip");
    fs::write(
        report_dir.join("definition.pbir"),
        r#"{"datasetReference":{"byPath":{"path":"../Broken.SemanticModel"}}}"#,
    )
    .expect("write pbir");

    let project_arg = project.to_str().expect("project path");
    let output = run_powerbi(&["lint", project_arg, "--json"]);
    assert_ne!(output.code, 0, "lint should fail for broken project");
    assert!(
        output.stderr.trim().is_empty(),
        "structured lint failures should stay on stdout, stderr: {}",
        output.stderr
    );
    let value = stdout_json(&output);
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["exitCode"], Value::from(output.code));
    assert_ne!(value["exitCode"], Value::from(0));
    assert!(
        value["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["severity"] == "error")
    );
}

#[test]
fn validators_fail_process_and_do_not_emit_unsafe_next_steps_when_invalid() {
    let schema = run_powerbi(&[
        "schema",
        "validate",
        "testdata/golden/sales.summary.json",
        "--json",
    ]);
    assert_eq!(schema.code, 10, "stderr: {}", schema.stderr);
    let schema_json = stdout_json(&schema);
    assert_eq!(schema_json["ok"], Value::Bool(false));
    assert_eq!(schema_json["exitCode"], Value::from(10));
    assert!(
        !schema_json["next"]
            .as_array()
            .expect("next")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report build"))
    );

    let profile = run_powerbi(&[
        "profile",
        "validate",
        "testdata/golden/sales.summary.json",
        "--json",
    ]);
    assert_eq!(profile.code, 10, "stderr: {}", profile.stderr);
    let profile_json = stdout_json(&profile);
    assert_eq!(profile_json["ok"], Value::Bool(false));
    assert_eq!(profile_json["exitCode"], Value::from(10));
    assert!(profile_json["next"].as_array().expect("next").is_empty());

    let spec = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        "testdata/golden/sales.summary.json",
        "--json",
    ]);
    assert_eq!(spec.code, 10, "stderr: {}", spec.stderr);
    let spec_json = stdout_json(&spec);
    assert_eq!(spec_json["ok"], Value::Bool(false));
    assert_eq!(spec_json["exitCode"], Value::from(10));
    assert!(spec_json["next"].as_array().expect("next").is_empty());

    let fields = run_powerbi(&[
        "report",
        "spec",
        "fields",
        "--schema",
        "testdata/golden/sales.summary.json",
        "--json",
    ]);
    assert_eq!(fields.code, 10, "stderr: {}", fields.stderr);
    let fields_json = stdout_json(&fields);
    assert_eq!(fields_json["ok"], Value::Bool(false));
    assert_eq!(fields_json["exitCode"], Value::from(10));
    assert!(
        fields_json["next"]
            .as_array()
            .expect("next")
            .iter()
            .all(|command| !command
                .as_str()
                .unwrap_or_default()
                .contains("report build"))
    );
}

#[test]
fn profile_infer_next_commands_point_at_supported_report_plan() {
    let output = run_powerbi(&[
        "profile",
        "infer",
        "--schema",
        "examples/sales.schema.json",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let next = value["next"].as_array().expect("next");
    assert!(next.iter().any(|command| {
        command
            .as_str()
            .unwrap_or_default()
            .contains("profile infer --schema examples/sales.schema.json --out <profile.json>")
    }));
    assert!(
        next.iter()
            .any(|command| command.as_str().unwrap_or_default().contains("report plan"))
    );
}

#[test]
fn feature_catalog_advertises_supported_drillthrough_slice() {
    let output = run_powerbi(&["features", "list", "--for", "drillthrough", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["schema"], Value::from("powerbi-cli.features.v1"));
    assert_eq!(value["policy"]["noFakeFallbacks"], Value::Bool(true));
    assert_eq!(value["matchedFeatures"], Value::from(1));
    let feature = &value["features"][0];
    assert_eq!(feature["id"], Value::from("report.drillthrough"));
    assert_eq!(feature["status"], Value::from("supported"));
    assert_eq!(feature["support"], Value::from("read-write-page-binding"));
    assert_eq!(feature["emitsPbir"], Value::Bool(true));
    assert_eq!(feature["refusalCode"], Value::Null);
    assert!(
        feature["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .any(|command| command == "report drillthrough set")
    );
    assert_eq!(feature["runtimeFallback"], Value::Bool(false));
    assert_eq!(
        feature["proofLevel"],
        Value::from("manual-desktop-canvas-refresh")
    );
    assert!(
        feature["referenceSignals"]
            .as_array()
            .expect("reference signals")
            .iter()
            .filter_map(Value::as_str)
            .any(|signal| signal.contains("2026-07-10")
                && signal.contains("retained in a private repository"))
    );
}

#[test]
fn feature_catalog_does_not_leave_supported_visuals_in_planned_types() {
    let output = run_powerbi(&["features", "list", "--for", "planned-types", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["matchedFeatures"], Value::from(1));
    let feature = &value["features"][0];
    assert_eq!(feature["id"], Value::from("report.visuals.planned-types"));
    let next_proof = feature["nextProof"]
        .as_array()
        .expect("nextProof")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !next_proof.to_ascii_lowercase().contains("scatter"),
        "scatter is supported by the generated visual catalog and should not remain in planned-types nextProof"
    );
    for visual in ["pie", "donut", "matrix", "slicer"] {
        assert!(
            !next_proof.to_ascii_lowercase().contains(visual),
            "{visual} is generated with manual Desktop canvas proof and should not remain in planned-types nextProof"
        );
    }
}

#[test]
fn new_visual_feature_entries_are_manually_desktop_canvas_proven() {
    for feature_id in [
        "report.visuals.category-share",
        "report.visuals.matrix",
        "report.slicer-authoring",
    ] {
        let output = run_powerbi(&["features", "list", "--for", feature_id, "--json"]);
        assert_eq!(output.code, 0, "{feature_id} stderr: {}", output.stderr);
        let value = stdout_json(&output);
        assert_eq!(value["matchedFeatures"], Value::from(1));
        let feature = &value["features"][0];
        assert_eq!(feature["id"], feature_id);
        assert_eq!(feature["status"], "supported");
        assert_eq!(feature["proofLevel"], "manual-desktop-canvas-refresh");
        assert_eq!(feature["emitsPbir"], Value::Bool(true));
        assert!(
            feature["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("canvas-proof.2026-07-10.refresh-session.json")
        );
        let next_proof = feature["nextProof"].as_array().expect("next proof");
        assert!(
            next_proof
                .iter()
                .filter_map(Value::as_str)
                .any(|step| step.contains("desktop-canvas-refresh"))
        );
        assert!(
            next_proof
                .iter()
                .filter_map(Value::as_str)
                .any(|step| step.contains("formatting"))
        );
    }
}

#[test]
fn singular_agent_aliases_route_to_existing_command_families() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let scaffold = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(scaffold.code, 0, "stderr: {}", scaffold.stderr);

    let measure = run_powerbi(&["model", "measure", "list", "--project", out, "--json"]);
    assert_eq!(measure.code, 0, "stderr: {}", measure.stderr);
    assert_eq!(
        stdout_json(&measure)["schema"],
        Value::from("powerbi-cli.model.measures.list.v1")
    );

    let relationship = run_powerbi(&["model", "relationship", "list", "--project", out, "--json"]);
    assert_eq!(relationship.code, 0, "stderr: {}", relationship.stderr);
    assert_eq!(
        stdout_json(&relationship)["schema"],
        Value::from("powerbi-cli.model.relationships.list.v1")
    );

    let calculated_column = run_powerbi(&[
        "model",
        "calculated-column",
        "list",
        "--project",
        out,
        "--json",
    ]);
    assert_eq!(
        calculated_column.code, 0,
        "stderr: {}",
        calculated_column.stderr
    );
    assert_eq!(
        stdout_json(&calculated_column)["schema"],
        Value::from("powerbi-cli.model.calculatedColumns.list.v1")
    );

    let visual = run_powerbi(&["report", "visual", "list", "--project", out, "--json"]);
    assert_eq!(visual.code, 0, "stderr: {}", visual.stderr);
    assert_eq!(
        stdout_json(&visual)["schema"],
        Value::from("powerbi-cli.report.visuals.list.v1")
    );
}

#[test]
#[cfg(windows)]
fn desktop_open_check_reports_launch_method_without_requiring_oracle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let scaffold = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(scaffold.code, 0, "stderr: {}", scaffold.stderr);

    let output = run_powerbi_without_oracle(&["desktop", "open-check", out, "--json"]);
    assert_ne!(
        output.code, 0,
        "open-check should not claim proof without oracle opt-in"
    );
    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        Value::from("powerbi-cli.desktop.openCheck.v1")
    );
    assert_eq!(value["changes"], serde_json::json!([]));
    assert_eq!(value["proof"]["level"], Value::from("unit-smoke"));
    assert_eq!(
        value["proof"]["observedStage"],
        Value::from("not-attempted")
    );
    assert!(value["proof"]["signals"]["launchMethod"].is_string());
    assert!(value["proof"]["signals"]["detectionPathUsedForLaunch"].is_boolean());
    assert_eq!(
        value["proof"]["signals"]["launchMethod"],
        Value::from("windows-file-association")
    );
    assert!(value["proof"]["signals"]["fileAssociationReason"].is_string());
}

#[test]
fn robot_docs_and_triage_are_first_class_agent_surfaces() {
    let guide = run_powerbi(&["robot-docs", "guide"]);
    assert_eq!(guide.code, 0, "stderr: {}", guide.stderr);
    assert!(guide.stdout.contains("# powerbi-cli Agent Guide"));
    assert!(guide.stdout.contains("Do not grow a monolith"));
    assert!(guide.stdout.contains("report bookmarks list/show"));
    assert!(
        guide
            .stdout
            .contains("report filters list/show/add/update/delete/clear")
    );
    assert!(guide.stdout.contains("report drillthrough set/show/clear"));
    assert!(guide.stdout.contains("report slicers list/show/clear"));
    assert!(
        guide
            .stdout
            .contains("report interactions list/show/set/disable")
    );

    let guide_json = run_powerbi(&["robot-docs", "guide", "--json"]);
    assert_eq!(guide_json.code, 0, "stderr: {}", guide_json.stderr);
    let value = stdout_json(&guide_json);
    assert!(
        value["markdown"]
            .as_str()
            .unwrap_or_default()
            .contains("Rules for agents")
    );

    let triage = run_powerbi(&["--robot-triage"]);
    assert_eq!(triage.code, 0, "stderr: {}", triage.stderr);
    let value = stdout_json(&triage);
    assert_eq!(value["health"]["offlineAuthoring"], Value::Bool(true));
    assert!(value["quickRef"]["discover"].is_string());
    assert!(value["quickRef"]["schemaValidate"].is_string());
    assert!(value["quickRef"]["profileInfer"].is_string());
    assert!(value["quickRef"]["reportSpecValidate"].is_string());
    assert!(value["quickRef"]["reportBuild"].is_string());
    assert!(value["quickRef"]["reportBookmarksList"].is_string());
    assert!(value["quickRef"]["reportFiltersList"].is_string());
    assert!(value["quickRef"]["reportFilterAddDryRun"].is_string());
    assert!(value["quickRef"]["reportFilterClearPageDryRun"].is_string());
    assert!(value["quickRef"]["reportSlicersList"].is_string());
    assert!(value["quickRef"]["reportSlicerClearDryRun"].is_string());
    assert!(value["quickRef"]["reportInteractionsList"].is_string());
    assert!(value["quickRef"]["reportInteractionSetDryRun"].is_string());
    assert!(value["quickRef"]["reportInteractionDisableDryRun"].is_string());
    assert!(value["quickRef"]["reportVisualsCatalog"].is_string());
    assert!(value["quickRef"]["reportVisualFormattingSetColorDryRun"].is_string());
    assert!(value["recommendedNext"].as_array().is_some());
}

#[test]
fn typo_errors_include_hint_and_suggested_command() {
    let output = run_powerbi(&["--jsno", "capabilities"]);
    assert_eq!(output.code, 2);
    assert!(output.stdout.trim().is_empty());
    let value = stderr_json(&output);
    assert_eq!(value["error"]["code"], Value::from("invalid_args"));
    assert!(
        value["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("--json")
    );
    assert!(
        value["error"]["suggestedCommands"]
            .as_array()
            .expect("suggestedCommands")
            .iter()
            .any(|command| command == "powerbi-cli --json capabilities")
    );
}

#[test]
fn zero_arg_commands_reject_trailing_arguments() {
    for command in ["version", "robot-triage", "doctor"] {
        let output = run_powerbi(&[command, "--bogus", "--json"]);
        assert_eq!(output.code, 2, "{command} stderr: {}", output.stderr);
        assert_error_envelope(&output, "invalid_args");
        assert!(
            stderr_json(&output)["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("does not accept arguments")
        );
    }
}

#[test]
fn representative_next_commands_match_live_dispatch_and_flags() {
    let capabilities = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let capability_value = stdout_json(&capabilities);
    let catalog_commands = capability_value["commands"].as_array().expect("commands");

    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let responses = [
        run_powerbi(&["doctor", "--json"]),
        run_powerbi(&["features", "list", "--for", "drillthrough", "--json"]),
        run_powerbi(&[
            "profile",
            "infer",
            "--schema",
            "examples/sales.schema.json",
            "--json",
        ]),
        run_powerbi(&[
            "scaffold",
            "--schema",
            "examples/sales.schema.json",
            "--out-dir",
            out,
            "--json",
        ]),
        run_powerbi(&["handoff", "check", out, "--json"]),
    ];
    let mut next_commands = Vec::new();
    for response in responses {
        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        collect_next_commands(&stdout_json(&response), &mut next_commands);
    }
    assert!(!next_commands.is_empty());
    for command in next_commands {
        assert_executable_command_template(&command, catalog_commands);
    }
}

#[test]
fn representative_failures_use_the_documented_error_envelope() {
    let failures = [
        (
            run_powerbi(&["version", "--bogus", "--json"]),
            "invalid_args",
        ),
        (
            run_powerbi(&["model", "roles", "add", "--json"]),
            "unsupported_feature",
        ),
        (
            run_powerbi(&[
                "schema",
                "validate",
                "this-file-does-not-exist.schema.json",
                "--json",
            ]),
            "file_not_found",
        ),
    ];
    for (output, expected_code) in failures {
        assert_ne!(output.code, 0);
        assert_error_envelope(&output, expected_code);
    }
}

#[test]
fn typo_recovery_for_bare_families_suggests_an_executable_discovery_command() {
    let output = run_powerbi(&["repoort", "--json"]);
    assert_eq!(output.code, 2);
    let value = stderr_json(&output);
    assert_eq!(
        value["error"]["suggestedCommands"],
        json!(["powerbi-cli --json capabilities --for report"])
    );
    let capabilities = run_powerbi(&["capabilities", "--json"]);
    let catalog = stdout_json(&capabilities);
    assert_executable_command_template(
        value["error"]["suggestedCommands"][0]
            .as_str()
            .expect("suggested command"),
        catalog["commands"].as_array().expect("commands"),
    );
}

#[test]
fn semantic_model_handles_percent_encode_colon_components_and_round_trip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema_path = temp.path().join("colon.schema.json");
    fs::write(
        &schema_path,
        serde_json::to_vec_pretty(&json!({
            "name": "ColonNames",
            "tables": [
                {
                    "name": "Sales:Facts",
                    "columns": [{"name": "Order:Id", "dataType": "int64"}],
                    "measures": [{"name": "Gross%:Sales", "expression": "1"}],
                    "rows": []
                },
                {
                    "name": "Dim:Orders",
                    "columns": [{"name": "Order:Id", "dataType": "int64"}],
                    "rows": []
                }
            ],
            "relationships": [{
                "name": "colon-endpoints",
                "fromTable": "Sales:Facts",
                "fromColumn": "Order:Id",
                "toTable": "Dim:Orders",
                "toColumn": "Order:Id"
            }]
        }))
        .expect("serialize schema"),
    )
    .expect("write schema");
    let schema_arg = schema_path.to_str().expect("schema path");

    let validate = run_powerbi(&["schema", "validate", schema_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], Value::Bool(true));

    let out_dir = temp.path().join("project");
    let out = out_dir.to_str().expect("out path");
    let scaffold = run_powerbi(&[
        "scaffold",
        "--schema",
        schema_arg,
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(scaffold.code, 0, "stderr: {}", scaffold.stderr);

    let inspect = run_powerbi(&["inspect", "--deep", out, "--json"]);
    assert_eq!(inspect.code, 0, "stderr: {}", inspect.stderr);
    let deep = stdout_json(&inspect);
    let facts = deep["deep"]["model"]["tables"]
        .as_array()
        .expect("tables")
        .iter()
        .find(|table| table["name"] == "Sales:Facts")
        .expect("colon table");
    assert_eq!(facts["handle"], "table:Sales%3AFacts");
    assert_eq!(
        facts["columns"][0]["handle"],
        "column:Sales%3AFacts:Order%3AId"
    );
    assert_eq!(
        facts["measures"][0]["handle"],
        "measure:Sales%3AFacts:Gross%25%3ASales"
    );
    assert_eq!(
        facts["partitions"][0]["handle"],
        "partition:Sales%3AFacts:Sales%3AFacts"
    );

    let show_measure = run_powerbi(&[
        "model",
        "measures",
        "show",
        "--project",
        out,
        "--handle",
        "measure:Sales%3AFacts:Gross%25%3ASales",
        "--json",
    ]);
    assert_eq!(show_measure.code, 0, "stderr: {}", show_measure.stderr);
    assert_eq!(
        stdout_json(&show_measure)["measure"]["table"],
        "Sales:Facts"
    );
    assert_eq!(
        stdout_json(&show_measure)["measure"]["name"],
        "Gross%:Sales"
    );

    let show_partition = run_powerbi(&[
        "model",
        "partitions",
        "show",
        "--project",
        out,
        "--handle",
        "partition:Sales%3AFacts:Sales%3AFacts",
        "--json",
    ]);
    assert_eq!(show_partition.code, 0, "stderr: {}", show_partition.stderr);
    assert_eq!(
        stdout_json(&show_partition)["partition"]["table"],
        "Sales:Facts"
    );

    let relationships =
        run_powerbi(&["model", "relationships", "list", "--project", out, "--json"]);
    assert_eq!(relationships.code, 0, "stderr: {}", relationships.stderr);
    let relationship_json = stdout_json(&relationships);
    let relationship = &relationship_json["relationships"][0];
    assert_eq!(
        relationship["from"]["columnHandle"],
        "column:Sales%3AFacts:Order%3AId"
    );
    assert_eq!(
        relationship["to"]["columnHandle"],
        "column:Dim%3AOrders:Order%3AId"
    );

    let calculated_out = temp.path().join("calculated");
    let add_column = run_powerbi(&[
        "model",
        "calculated-columns",
        "add",
        "--project",
        out,
        "--table",
        "Sales:Facts",
        "--name",
        "Band:Name",
        "--expression",
        "1",
        "--data-type",
        "int64",
        "--out-dir",
        calculated_out.to_str().expect("calculated path"),
        "--json",
    ]);
    assert_eq!(add_column.code, 0, "stderr: {}", add_column.stderr);
    let show_column = run_powerbi(&[
        "model",
        "calculated-columns",
        "show",
        "--project",
        calculated_out.to_str().expect("calculated path"),
        "--handle",
        "column:Sales%3AFacts:Band%3AName",
        "--json",
    ]);
    assert_eq!(show_column.code, 0, "stderr: {}", show_column.stderr);
    assert_eq!(
        stdout_json(&show_column)["calculatedColumn"]["name"],
        "Band:Name"
    );

    let malformed = run_powerbi(&[
        "model",
        "measures",
        "show",
        "--project",
        out,
        "--handle",
        "measure:Sales%ZZFacts:Gross%25%3ASales",
        "--json",
    ]);
    assert_eq!(malformed.code, 2);
    assert_error_envelope(&malformed, "invalid_args");
    assert!(
        stderr_json(&malformed)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid measure handle")
    );
}

#[test]
fn scaffold_classifies_unavailable_column_types_as_unsupported_features() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema_path = temp.path().join("unsupported-type.schema.json");
    fs::write(
        &schema_path,
        serde_json::to_vec_pretty(&json!({
            "name": "UnsupportedType",
            "tables": [{
                "name": "Places",
                "columns": [{"name": "Location", "dataType": "geography"}],
                "rows": []
            }]
        }))
        .expect("serialize schema"),
    )
    .expect("write schema");
    let out_dir = temp.path().join("project");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        schema_path.to_str().expect("schema path"),
        "--out-dir",
        out_dir.to_str().expect("out path"),
        "--json",
    ]);
    assert_eq!(output.code, 2);
    assert_error_envelope(&output, "unsupported_feature");
    assert_eq!(
        stderr_json(&output)["error"]["message"],
        "unsupported column dataType: geography"
    );
    assert!(!out_dir.exists());
}
