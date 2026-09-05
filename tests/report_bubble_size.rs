//! Fixture-gated bubble formatting never writes guessed PBIR.
mod common;

use common::{build_scatter_bubble, hash_tree, run_powerbi_owned, stderr_json};
use serde_json::{Value, json};
use std::fs;

#[test]
fn bubble_size_refuses_single_and_batch_in_every_mode_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let project = build_scatter_bubble(root.path());
    let scatter = walkdir::WalkDir::new(&project)
        .into_iter().filter_map(Result::ok)
        .find(|entry| entry.file_name() == "visual.json" &&
            serde_json::from_slice::<Value>(&fs::read(entry.path()).unwrap()).unwrap()
                ["visual"]["visualType"] == "scatterChart")
        .expect("generated scatter");
    let visual = scatter
        .path()
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let page = scatter
        .path()
        .ancestors()
        .nth(3)
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let handle = format!("visual:{page}:{visual}");
    let before = hash_tree(&project);
    let out = root.path().join("out");
    let batch = root.path().join("batch.json");
    fs::write(&batch, serde_json::to_vec(&json!({
        "schema":"powerbi-cli.ops.v1", "ops":[
            {"op":"setObject", "visual":handle, "object":"title", "property":"show",
             "value":{"expr":{"Literal":{"Value":"false"}}}},
            {"op":"setObject", "visual":handle, "object":"bubbles", "property":"bubbleSize", "value":20}
        ]
    })).unwrap()).unwrap();
    for mode in [
        vec!["--dry-run".to_string()],
        vec!["--in-place".to_string()],
        vec!["--out-dir".to_string(), out.display().to_string()],
    ] {
        for value in [
            Some("20"),
            Some("-50"),
            Some("50"),
            Some("-100"),
            Some("100"),
            None,
        ] {
            let mut args = vec![
                "report".into(),
                "visuals".into(),
                "set-object".into(),
                "--project".into(),
                project.display().to_string(),
                "--json".into(),
            ];
            if let Some(value) = value {
                args.extend([
                    "--handle".into(),
                    handle.clone(),
                    "--object".into(),
                    "bubbles".into(),
                    "--property".into(),
                    "bubbleSize".into(),
                    "--value".into(),
                    value.into(),
                ]);
            } else {
                args.extend(["--batch".into(), batch.display().to_string()]);
            }
            args.extend(mode.clone());
            let result = run_powerbi_owned(&args);
            assert_eq!(result.code, 2, "{}", result.stderr);
            assert!(result.stdout.is_empty());
            let error = stderr_json(&result);
            assert_eq!(error["error"]["code"], "unsupported_feature");
            assert!(
                error["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("Desktop-authored reference")
            );
            assert!(
                error["error"]["hint"]
                    .as_str()
                    .unwrap()
                    .contains("Size field binding")
            );
            assert_eq!(result.stderr, run_powerbi_owned(&args).stderr);
            assert_eq!(before, hash_tree(&project));
            assert!(!out.exists());
        }
    }
}
