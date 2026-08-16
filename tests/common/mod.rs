//! Shared test harness helpers. Each test binary includes this module via
//! `mod common;`, so helpers unused by a given binary are expected dead code.
#![allow(dead_code)]

use serde_json::Value;
use std::process::Command;

pub struct RunOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn run_powerbi(args: &[&str]) -> RunOutput {
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

pub fn stdout_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stdout.trim()).expect("stdout JSON")
}

pub fn stderr_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stderr.trim()).expect("stderr JSON")
}

pub fn assert_unsupported_feature(stderr: &str, message_fragment: &str) -> Value {
    let value: Value = serde_json::from_str(stderr.trim()).expect("stderr JSON");
    assert_eq!(value["error"]["code"], Value::from("unsupported_feature"));
    assert_eq!(value["error"]["exitCode"], Value::from(2));
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains(message_fragment),
        "expected error message to contain {message_fragment:?}: {value}"
    );
    value
}
