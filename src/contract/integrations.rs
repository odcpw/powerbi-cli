//! Optional integration and embedded-skill capability descriptors.

use serde_json::{Value, json};

pub(super) fn commands() -> Vec<Value> {
    vec![
        json!({
            "path": "integrations status",
            "aliases": [],
            "usage": "powerbi-cli integrations status [--deep] [--component modeling-mcp|report-authoring|desktop-bridge] --json",
            "summary": "Inspect the exact optional Microsoft Power BI toolchain without installation or registry access",
            "tags": ["microsoft", "integration", "supply-chain", "offline", "agent"],
            "readOnly": true,
            "mutates": false,
            "networkRequired": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.integrations.status.v1",
            "flags": ["--deep", "--component modeling-mcp|report-authoring|desktop-bridge", "--json", "--format json"],
            "examples": ["powerbi-cli integrations status --json", "powerbi-cli integrations status --component report-authoring --deep --json"],
            "limitations": ["Shallow status launches no child. Deep status runs bounded exact checks against an already installed version-addressed private cache; neither mode installs or contacts a registry.", "The installed-tree checksum detects accidental drift; the same-user cache is not a privileged trust store or a signature boundary."],
            "followUpFields": ["ready", "mode", "selectedComponent", "platform", "lock.id", "lock.fingerprint", "node", "cache", "components[].id", "components[].state", "components[].ready", "childProcessesLaunched", "next"]
        }),
        json!({
            "path": "integrations install",
            "aliases": [],
            "usage": "powerbi-cli integrations install --allow-network --json",
            "summary": "Install and atomically activate the committed exact Microsoft Power BI npm graph",
            "tags": ["microsoft", "integration", "supply-chain", "install", "network", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "networkRequired": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.integrations.install.v1",
            "flags": ["--allow-network", "--json", "--format json"],
            "examples": ["powerbi-cli integrations install --allow-network --json"],
            "limitations": ["The network opt-in is mandatory. npm receives an allowlisted environment; normal commands never install, download, npm, or npx."],
            "followUpFields": ["ok", "readOnly", "mutates", "mutatesProject", "networkRequired", "lockId", "lockFingerprint", "cachePath", "activationResult", "priorActiveVersion", "components", "changes", "next"]
        }),
        json!({
            "path": "skill status",
            "aliases": ["skill verify", "skill check", "skills status"],
            "usage": "powerbi-cli skill status --json",
            "summary": "Verify that the globally installed Codex skill exactly matches the repository-embedded canonical skill",
            "tags": ["skill", "codex", "install", "verify", "agent", "no-python"],
            "readOnly": true,
            "mutates": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.skill.status.v1",
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli skill status --json"],
            "followUpFields": ["installed", "inSync", "sourceOfTruth", "root", "files[].relativePath", "files[].present", "files[].matchesEmbedded", "next"]
        }),
        json!({
            "path": "skill install",
            "aliases": ["skill sync", "skills install"],
            "usage": "powerbi-cli skill install --json",
            "summary": "Install or repair the canonical embedded Codex skill without Python, network access, or an external script",
            "tags": ["skill", "codex", "install", "repair", "agent", "no-python"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "networkRequired": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.skill.status.v1",
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli skill install --json"],
            "limitations": ["Writes only the skill files owned by powerbi-cli under CODEX_HOME/skills/powerbi-cli (or the default ~/.codex path); unrelated files are preserved.", "Start a new Codex session after a changed install so the formal skill catalog reloads."],
            "followUpFields": ["installed", "inSync", "changed", "changes", "reloadRequired", "root", "files", "next"]
        }),
    ]
}
