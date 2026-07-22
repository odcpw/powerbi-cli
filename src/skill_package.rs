use crate::project_io::write_text_atomic;
use crate::{CliError, CliResult, EXIT_SUCCESS, canonical_display};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const SKILL_NAME: &str = "powerbi-cli";
const SKILL_MD: &str = include_str!("../skills/powerbi-cli/SKILL.md");
const DESKTOP_RUNTIME_REFERENCE: &str =
    include_str!("../skills/powerbi-cli/references/desktop-runtime-regression.md");

const EMBEDDED_FILES: &[(&str, &str)] = &[
    ("SKILL.md", SKILL_MD),
    (
        "references/desktop-runtime-regression.md",
        DESKTOP_RUNTIME_REFERENCE,
    ),
];

pub(crate) fn skill_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(
            CliError::invalid_args("skill requires a subcommand: install or status")
                .with_hint("Install the embedded canonical Codex skill, or verify the global copy.")
                .with_suggested_command("powerbi-cli skill install --json")
                .with_suggested_command("powerbi-cli skill status --json"),
        );
    };
    reject_extra_args(action, rest)?;
    let root = codex_skill_root()?;
    match action.as_str() {
        "install" | "sync" => install_to(&root),
        "status" | "verify" | "check" => status_at(&root, false),
        other => Err(
            CliError::invalid_args(format!("unknown skill command: {other}"))
                .with_hint("Use `skill install` or `skill status`.")
                .with_suggested_command("powerbi-cli skill install --json")
                .with_suggested_command("powerbi-cli skill status --json"),
        ),
    }
}

fn reject_extra_args(action: &str, args: &[String]) -> CliResult<()> {
    if let Some(argument) = args.first() {
        return Err(CliError::invalid_args(format!(
            "skill {action} accepts no arguments: {argument}"
        ))
        .with_hint("Global --json may appear anywhere; no path is required.")
        .with_suggested_command(format!("powerbi-cli skill {action} --json")));
    }
    Ok(())
}

fn codex_skill_root() -> CliResult<PathBuf> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(codex_home).join("skills").join(SKILL_NAME));
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .ok_or_else(|| {
            CliError::unexpected(
                "cannot locate Codex home; CODEX_HOME, USERPROFILE, and HOME are unset",
            )
            .with_hint("Set CODEX_HOME to the Codex configuration directory and retry.")
        })?;
    Ok(PathBuf::from(home)
        .join(".codex")
        .join("skills")
        .join(SKILL_NAME))
}

fn install_to(root: &Path) -> CliResult<Value> {
    reject_link(root, "skill directory")?;
    fs::create_dir_all(root).map_err(|error| {
        CliError::unexpected(format!(
            "create Codex skill directory {}: {error}",
            root.display()
        ))
    })?;

    let mut changed = Vec::new();
    for (relative, content) in EMBEDDED_FILES {
        let path = root.join(relative);
        reject_link(&path, "skill file")?;
        let same = fs::read_to_string(&path).is_ok_and(|existing| existing == *content);
        if same {
            continue;
        }
        if let Some(parent) = path.parent() {
            reject_link(parent, "skill file parent")?;
            fs::create_dir_all(parent).map_err(|error| {
                CliError::unexpected(format!(
                    "create Codex skill directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        write_text_atomic(&path, content)?;
        changed.push(relative.to_string());
    }

    let mut result = status_at(root, true)?;
    result["changed"] = Value::Bool(!changed.is_empty());
    result["changes"] = json!(changed);
    result["reloadRequired"] = Value::Bool(!changed.is_empty());
    result["next"] = if changed.is_empty() {
        json!(["powerbi-cli skill status --json"])
    } else {
        json!([
            "Start a new Codex session so the installed skill catalog reloads.",
            "powerbi-cli skill status --json"
        ])
    };
    Ok(result)
}

fn status_at(root: &Path, installed_by_command: bool) -> CliResult<Value> {
    let files = EMBEDDED_FILES
        .iter()
        .map(|(relative, content)| {
            let path = root.join(relative);
            let expected_sha256 = sha256(content.as_bytes());
            let actual = fs::read(&path).ok();
            let actual_sha256 = actual.as_deref().map(sha256);
            json!({
                "path": canonical_display(&path),
                "relativePath": relative,
                "present": actual.is_some(),
                "matchesEmbedded": actual_sha256.as_deref() == Some(expected_sha256.as_str()),
                "expectedSha256": expected_sha256,
                "actualSha256": actual_sha256
            })
        })
        .collect::<Vec<_>>();
    let present = files
        .iter()
        .all(|file| file["present"].as_bool() == Some(true));
    let in_sync = files
        .iter()
        .all(|file| file["matchesEmbedded"].as_bool() == Some(true));
    Ok(json!({
        "schema": "powerbi-cli.skill.status.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "skill": SKILL_NAME,
        "root": canonical_display(root),
        "installed": present,
        "inSync": in_sync,
        "installedByCommand": installed_by_command,
        "sourceOfTruth": "embedded-from-repository-skills/powerbi-cli",
        "files": files,
        "changed": false,
        "changes": [],
        "reloadRequired": false,
        "next": if in_sync {
            vec!["Start a new Codex session if the skill was installed during this session."]
        } else {
            vec!["powerbi-cli skill install --json"]
        }
    }))
}

fn reject_link(path: &Path, label: &str) -> CliResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(CliError::validation_failed(
            format!("{label} must not be a symbolic link: {}", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliError::unexpected(format!(
            "inspect {label} {}: {error}",
            path.display()
        ))),
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent_and_repairs_only_owned_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("skills").join(SKILL_NAME);

        let first = install_to(&root).expect("install skill");
        assert_eq!(first["changed"], true);
        assert_eq!(first["inSync"], true);

        let extra = root.join("notes.txt");
        fs::write(&extra, "keep").expect("write unrelated skill note");
        fs::write(root.join("SKILL.md"), "stale").expect("drift skill");
        let repaired = install_to(&root).expect("repair skill");
        assert_eq!(repaired["changed"], true);
        assert_eq!(repaired["changes"], json!(["SKILL.md"]));
        assert_eq!(fs::read_to_string(extra).expect("read extra"), "keep");

        let unchanged = install_to(&root).expect("idempotent install");
        assert_eq!(unchanged["changed"], false);
        assert_eq!(unchanged["inSync"], true);
    }

    #[test]
    fn status_reports_missing_and_drifted_files_without_writing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("missing-skill");
        let missing = status_at(&root, false).expect("missing status");
        assert_eq!(missing["installed"], false);
        assert_eq!(missing["inSync"], false);
        assert!(!root.exists());
    }
}
