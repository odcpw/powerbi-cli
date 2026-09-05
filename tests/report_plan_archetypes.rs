//! Every checked-in archetype profile has an explicit planner evidence outcome.
mod common;

use common::{load_archetype, run_powerbi, stderr_json, stdout_json};
use serde_json::Value;
use std::fs;

#[test]
fn every_archetype_profile_plans_or_refuses_for_its_documented_date_evidence() {
    let mut profiles: Vec<_> = fs::read_dir("examples/archetypes")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_str().unwrap().ends_with(".profile.json"))
        .collect();
    profiles.sort();
    assert!(!profiles.is_empty());
    for profile in profiles {
        let name = profile
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .trim_end_matches(".profile.json");
        let expected_date = match name {
            "catalog-proof" => Some("CatalogFacts[YearStart]"),
            "flat-ops" => Some("WorkItems[WorkDate]"),
            "slicer-rail" => Some("DimDate[Date]"),
            "regional-sales" | "scatter-bubble" => None,
            _ => panic!("document the planner evidence outcome for {name}"),
        };
        let fixture = load_archetype(name);
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("plan.json");
        let args = [
            "report",
            "plan",
            "--schema",
            fixture.schema.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
            "--objective",
            "Executive overview",
            "--out",
            out.to_str().unwrap(),
            "--json",
        ];
        let run = run_powerbi(&args);
        if let Some(field) = expected_date {
            assert_eq!(run.code, 0, "{name}: {}", run.stderr);
            let response = stdout_json(&run);
            assert_eq!(response["schema"], "powerbi-cli.report.plan.v1");
            let planned_bytes = fs::read(&out).unwrap();
            let planned: Value = serde_json::from_slice(&planned_bytes).unwrap();
            assert!(
                planned["pages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|page| page["visuals"].as_array().unwrap().iter().any(
                        |visual| visual["type"] == "lineChart"
                            && visual["bindings"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|binding| binding["field"] == field
                                    && binding["role"] == "Category")
                    )),
                "{name}: the trend must use its declared date, not a numeric/name-only axis"
            );
            let validate = run_powerbi(&[
                "report",
                "spec",
                "validate",
                "--schema",
                fixture.schema.to_str().unwrap(),
                "--profile",
                profile.to_str().unwrap(),
                "--spec",
                out.to_str().unwrap(),
                "--json",
            ]);
            assert_eq!(validate.code, 0, "{name}: {}", validate.stderr);
            fs::remove_file(&out).unwrap();
            assert_eq!(run.code, run_powerbi(&args).code);
            assert_eq!(planned_bytes, fs::read(&out).unwrap());
        } else {
            assert_eq!(run.code, 10, "{name}: {}", run.stderr);
            assert!(run.stdout.is_empty());
            let error = stderr_json(&run);
            assert_eq!(error["error"]["code"], "plan.missing_input");
            assert_eq!(error["error"]["pointer"], "/schema/tables");
            assert_eq!(
                error["error"]["field"],
                "schema.tables[].columns[].type=date"
            );
            assert!(
                error["error"]["reason"]
                    .as_str()
                    .unwrap()
                    .contains("no supported date axis")
            );
            assert_eq!(run.stderr, run_powerbi(&args).stderr);
            assert!(!out.exists(), "{name}: refusal wrote a plan");
        }
    }
}
