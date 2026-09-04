mod common;

use common::{CliRun, archetype_names, load_archetype, run_powerbi_owned};
use serde_json::Value;
use std::path::Path;

#[test]
fn every_archetype_completes_the_offline_authoring_loop() {
    let temp = tempfile::tempdir().expect("e2e tempdir");

    for name in archetype_names() {
        let fixture = load_archetype(name);
        let case_dir = temp.path().join(name);
        std::fs::create_dir(&case_dir).expect("create archetype work directory");
        let inferred_profile = case_dir.join("inferred.profile.json");
        let planned_spec = case_dir.join("planned.dashboard.json");
        let project = case_dir.join("project");
        let replay_project = case_dir.join("project-replay");
        let normalized_summary = case_dir.join("normalized.summary.json");

        step(
            name,
            "schema validate",
            run_powerbi_owned(&[
                "schema".into(),
                "validate".into(),
                path_arg(&fixture.schema),
                "--json".into(),
            ]),
        );
        step(
            name,
            "profile infer",
            run_powerbi_owned(&[
                "profile".into(),
                "infer".into(),
                "--schema".into(),
                path_arg(&fixture.schema),
                "--out".into(),
                path_arg(&inferred_profile),
                "--json".into(),
            ]),
        );
        step(
            name,
            "profile validate",
            run_powerbi_owned(&[
                "profile".into(),
                "validate".into(),
                path_arg(&inferred_profile),
                "--json".into(),
            ]),
        );
        step(
            name,
            "report plan",
            run_powerbi_owned(&[
                "report".into(),
                "plan".into(),
                "--schema".into(),
                path_arg(&fixture.schema),
                "--profile".into(),
                path_arg(&inferred_profile),
                "--objective".into(),
                format!("Offline {name} overview"),
                "--out".into(),
                path_arg(&planned_spec),
                "--json".into(),
            ]),
        );
        step(
            name,
            "planned spec validate",
            run_powerbi_owned(&[
                "report".into(),
                "spec".into(),
                "validate".into(),
                "--schema".into(),
                path_arg(&fixture.schema),
                "--profile".into(),
                path_arg(&inferred_profile),
                "--spec".into(),
                path_arg(&planned_spec),
                "--json".into(),
            ]),
        );
        step(
            name,
            "fixture spec validate",
            run_powerbi_owned(&[
                "report".into(),
                "spec".into(),
                "validate".into(),
                "--schema".into(),
                path_arg(&fixture.schema),
                "--profile".into(),
                path_arg(&fixture.profile),
                "--spec".into(),
                path_arg(&fixture.spec),
                "--json".into(),
            ]),
        );
        step(name, "report build", fixture.build_into(&project));
        step(
            name,
            "validate strict",
            run_powerbi_owned(&[
                "validate".into(),
                "--strict".into(),
                path_arg(&project),
                "--json".into(),
            ]),
        );
        step(
            name,
            "handoff check",
            run_powerbi_owned(&[
                "handoff".into(),
                "check".into(),
                path_arg(&project),
                "--json".into(),
            ]),
        );
        step(
            name,
            "lint",
            run_powerbi_owned(&["lint".into(), path_arg(&project), "--json".into()]),
        );
        step(
            name,
            "triage",
            run_powerbi_owned(&["triage".into(), path_arg(&project), "--json".into()]),
        );
        step(
            name,
            "fixture normalize",
            run_powerbi_owned(&[
                "fixture".into(),
                "normalize".into(),
                path_arg(&project),
                "--out".into(),
                path_arg(&normalized_summary),
                "--json".into(),
            ]),
        );
        step(
            name,
            "deterministic replay build",
            fixture.build_into(&replay_project),
        );
        let verify = step(
            name,
            "fixture verify",
            run_powerbi_owned(&[
                "fixture".into(),
                "verify".into(),
                path_arg(&replay_project),
                "--expected".into(),
                path_arg(&normalized_summary),
                "--json".into(),
            ]),
        );
        assert_eq!(
            verify["verification"]["same"],
            Value::Bool(true),
            "{name}: fixture verification did not match"
        );
    }
}

fn step(archetype: &str, phase: &str, run: CliRun) -> Value {
    assert_eq!(
        run.exit, 0,
        "{archetype}: {phase} failed\nargv: {:?}\nstdout: {}\nstderr: {}",
        run.argv, run.stdout, run.stderr
    );
    serde_json::from_str(run.stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "{archetype}: {phase} stdout was not JSON: {error}\nstdout: {}",
            run.stdout
        )
    })
}

fn path_arg(path: &Path) -> String {
    path.to_str().expect("test path is UTF-8").to_string()
}
