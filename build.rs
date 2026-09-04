fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=testdata/desktop-proof");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    generate_desktop_proof_records();

    println!("cargo:rustc-env=POWERBI_CLI_GIT_SHA={}", git_sha());
    println!("cargo:rustc-env=POWERBI_CLI_BUILD_EPOCH={}", build_epoch());
}

fn generate_desktop_proof_records() {
    let manifest_dir = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"),
    );
    let proof_dir = manifest_dir.join("testdata/desktop-proof");
    let mut records = std::fs::read_dir(&proof_dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", proof_dir.display()))
        .map(|entry| {
            entry.unwrap_or_else(|error| panic!("read {} entry: {error}", proof_dir.display()))
        })
        .filter_map(|entry| {
            let path = entry.path();
            let kind = entry
                .file_type()
                .unwrap_or_else(|error| panic!("inspect {}: {error}", path.display()));
            if !kind.is_file()
                || !path
                    .extension()
                    .is_some_and(|extension| extension == "json")
            {
                return None;
            }
            Some(entry.file_name().into_string().unwrap_or_else(|name| {
                panic!(
                    "Desktop proof record filename is not UTF-8: {}",
                    name.to_string_lossy()
                )
            }))
        })
        .collect::<Vec<_>>();
    records.sort();

    let mut generated =
        String::from("const EMBEDDED_DESKTOP_PROOF_RECORDS: &[(&str, &str)] = &[\n");
    for file_name in records {
        println!("cargo:rerun-if-changed=testdata/desktop-proof/{file_name}");
        let relative = format!("testdata/desktop-proof/{file_name}");
        let include_suffix = format!("/{relative}");
        generated.push_str(&format!(
            "    ({relative:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {include_suffix:?}))),\n"
        ));
    }
    generated.push_str("];\n");

    let out_dir =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    std::fs::write(out_dir.join("desktop_proof_records.rs"), generated)
        .expect("write generated Desktop proof record index");
}

fn git_sha() -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let hex = sha
                .chars()
                .take(12)
                .filter(|character| character.is_ascii_hexdigit())
                .collect::<String>();
            if hex.len() == 12 {
                hex
            } else {
                "unknown".to_string()
            }
        }
        _ => "unknown".to_string(),
    }
}

fn build_epoch() -> u64 {
    if let Ok(value) = std::env::var("SOURCE_DATE_EPOCH")
        && let Ok(epoch) = value.trim().parse::<u64>()
    {
        return epoch;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
