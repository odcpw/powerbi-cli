use serde_json::Value;

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
