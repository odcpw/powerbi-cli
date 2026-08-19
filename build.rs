fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    println!("cargo:rustc-env=POWERBI_CLI_GIT_SHA={}", git_sha());
    println!("cargo:rustc-env=POWERBI_CLI_BUILD_EPOCH={}", build_epoch());
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
