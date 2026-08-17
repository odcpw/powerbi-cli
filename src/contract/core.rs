//! Shared contract envelopes, diagnostics, catalogs, and façade implementations.

use super::{desktop, integrations, model, report, workflow_pkg};
use crate::feature_catalog::{feature_catalog_schema_fields, feature_policy_json};
use crate::visual_catalog::{
    schema_golden_visual_type_names, supported_visual_type_names, visual_type_contracts,
};
use crate::{
    CliError, CliResult, EXIT_FILE_NOT_FOUND, EXIT_INVALID_ARGS, EXIT_ORACLE_FAILED,
    EXIT_ORACLE_UNAVAILABLE, EXIT_PROOF_INCOMPLETE, EXIT_SUCCESS, EXIT_UNEXPECTED,
    EXIT_VALIDATION_FAILED, PBIP_SCHEMA, REPORT_DEFINITION_SCHEMA,
    SEMANTIC_MODEL_DEFINITION_SCHEMA,
};
use serde_json::{Value, json};

pub(crate) const CONTRACT_VERSION: &str = "powerbi-cli.agent-capabilities.v1";

const PROOF_LEVELS: &[(&str, &str)] = &[
    (
        "unit-smoke",
        "Covered by local cargo tests; does not claim Desktop compatibility",
    ),
    (
        "schema-golden",
        "Generated output is covered by exact schema/golden assertions without a completed Desktop canvas oracle",
    ),
    (
        "desktop-golden-pending",
        "A Desktop-authored reference shape and local generation/golden tests exist, but the generated fixture has not completed the Desktop canvas, refresh, save, and reopen oracle",
    ),
    (
        "manual-desktop-canvas-refresh",
        "A generated fixture was manually opened, rendered, refreshed, and inspected in Power BI Desktop with a committed proof record",
    ),
    (
        "desktop-canvas-refresh",
        "Automated Desktop oracle proof observed rendered pages and refresh and rejected blank canvases or issue dialogs",
    ),
];

pub(crate) fn help_text() -> String {
    r#"powerbi-cli helps agents author offline-safe Power BI PBIP projects.

Usage:
  powerbi-cli version --json
  powerbi-cli --json capabilities [--for <filter>]
  powerbi-cli features list [--for <feature-filter>] --json
  powerbi-cli package inspect <file.pbix|file.pbit|file.zip> --json
  powerbi-cli package extract <file.pbix|file.pbit|file.zip> --out-dir <dir> [--max-entries <n>] [--max-entry-bytes <n>] [--max-total-bytes <n>] [--max-compression-ratio <n>] --json
  powerbi-cli package import <file.pbix|file.pbit|file.zip> --out-dir <project-dir> --json
  powerbi-cli package source-pack --project <project-dir-or.pbip> --out <archive.pbit> --json
  powerbi-cli package export-plan --project <project-dir-or.pbip> --json
  powerbi-cli robot-docs guide [--json]
  powerbi-cli --robot-triage
  powerbi-cli robot-triage
  powerbi-cli --json doctor
  powerbi-cli integrations status [--deep] [--component modeling-mcp|report-authoring|desktop-bridge] --json
  powerbi-cli integrations install --allow-network --json
  powerbi-cli skill status --json
  powerbi-cli skill install --json
  powerbi-cli workflow plan --project <project-or.pbip> --profile <source-profile.json> --out <new-plan.json> --out-dir <new-project-dir> [--resource <name>=<path>] --json
  powerbi-cli workflow run --plan <plan.json> --confirm <plan-fingerprint> --json
  powerbi-cli workflow verify --plan <plan.json> --json
  powerbi-cli workflow synthesize --project <project-dir-or.pbip> --expressions <expressions.tmdl> --out-dir <new-project-dir> [--map <schema.item>=<ExpressionName>] --json
  powerbi-cli desktop open <project-dir-or.pbip-or.pbix> [--preflight strict|normal|skip] --json
  powerbi-cli desktop close --json
  powerbi-cli desktop open-check <project-dir-or.pbip-or.pbix> --json
  powerbi-cli desktop screenshot <project-dir-or.pbip-or.pbix> --out <evidence.png> --json
  powerbi-cli desktop bridge status [--pid <pid>] --json
  powerbi-cli desktop bridge reload --project <project-dir-or.pbip> --pid <pid> --json
  powerbi-cli desktop bridge screenshot-page --project <project-dir-or.pbip> --pid <pid> --page <id> --out <new.png> --json
  powerbi-cli desktop bridge screenshot-all --project <project-dir-or.pbip> --pid <pid> --out-dir <new-dir> --json
  powerbi-cli fixture normalize <project-dir-or.pbip> --json
  powerbi-cli fixture verify <project-dir-or.pbip> --expected <summary.json> --json
  powerbi-cli --json scaffold --schema <schema.json> --out-dir <project-dir> [--force]
  powerbi-cli --json inspect <project-dir-or.pbip>
  powerbi-cli lint <project-dir-or.pbip> --json
  powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> --json
  powerbi-cli model tables add-static --project <project-dir-or.pbip> --table <table> --column <column> --values-json '["One","Two"]' --dry-run --json
  powerbi-cli model columns set-sort-by --project <project-dir-or.pbip> --table <table> --column <column> --by <sort-column> --dry-run --json
  powerbi-cli model calculated-columns list --project <project-dir-or.pbip> --json
  powerbi-cli model calculated-columns show --project <project-dir-or.pbip> --handle <column-handle> --json
  powerbi-cli model calculated-columns add --project <project-dir-or.pbip> --table <table> --name <column> --expression <dax> --data-type <type> --dry-run --json
  powerbi-cli model calculated-columns update --project <project-dir-or.pbip> --handle <column-handle> --expression <dax> --dry-run --json
  powerbi-cli model calculated-columns delete --project <project-dir-or.pbip> --handle <column-handle> --dry-run --json
  powerbi-cli model measures list --project <project-dir-or.pbip> --json
  powerbi-cli model measures show --project <project-dir-or.pbip> --handle <measure-handle> --json
  powerbi-cli model measures add --project <project-dir-or.pbip> --table <table> --name <measure> --expression <dax> --dry-run --json
  powerbi-cli model measures update --project <project-dir-or.pbip> --handle <measure-handle> --expression <dax> --dry-run --json
  powerbi-cli model measures delete --project <project-dir-or.pbip> --handle <measure-handle> --dry-run --json
  powerbi-cli model relationships list --project <project-dir-or.pbip> --json
  powerbi-cli model relationships show --project <project-dir-or.pbip> --handle <relationship-handle> --json
  powerbi-cli model relationships add --project <project-dir-or.pbip> --from-table <table> --from-column <column> --to-table <table> --to-column <column> --dry-run --json
  powerbi-cli model relationships update --project <project-dir-or.pbip> --handle <relationship-handle> --cross-filtering-behavior <mode> --dry-run --json
  powerbi-cli model relationships delete --project <project-dir-or.pbip> --handle <relationship-handle> --dry-run --json
  powerbi-cli model partitions list --project <project-dir-or.pbip> --json
  powerbi-cli model partitions show --project <project-dir-or.pbip> --handle <partition-handle> [--include-source] --json
  powerbi-cli model dax bridge-plan --project <project-dir-or.pbip> --json
  powerbi-cli model dax dependencies --project <project-dir-or.pbip> --json
  powerbi-cli model dax lint --project <project-dir-or.pbip> --json
  powerbi-cli model dax execute --project <project-dir-or.pbip-or.pbix> --query-file <query.dax> --allow-data-read --json
  powerbi-cli model live export-tmdl --document <project-dir-or.pbip-or.pbix> --out-dir <fresh-dir> --allow-model-read --json
  powerbi-cli model advanced inventory --project <project-dir-or.pbip> --json
  powerbi-cli model roles list --project <project-dir-or.pbip> --json
  powerbi-cli model perspectives list --project <project-dir-or.pbip> --json
  powerbi-cli model cultures list --project <project-dir-or.pbip> --json
  powerbi-cli model expressions list --project <project-dir-or.pbip> --json
  powerbi-cli source-template list --project <project-dir-or.pbip> --json
  powerbi-cli source-template show --project <project-dir-or.pbip> --handle <source-template-handle> --json
  powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind <sql|postgres|odbc|excel> --dry-run --json
  powerbi-cli source-template apply --project <project-dir-or.pbip> --handle <source-template-handle> --server <server> --database <database> --dry-run --json
  powerbi-cli report design-plan --project <project-dir-or.pbip> --json
  powerbi-cli report tree --project <project-dir-or.pbip> --json
  powerbi-cli report find --project <project-dir-or.pbip> --kind <kind> --json
  powerbi-cli report cat --project <project-dir-or.pbip> --handle <object-handle> --json
  powerbi-cli report query --project <project-dir-or.pbip> --selector <selector> --json
  powerbi-cli report audit --project <project-dir-or.pbip> --json
  powerbi-cli report sanitize plan --project <project-dir-or.pbip> --json
  powerbi-cli report sanitize apply --project <project-dir-or.pbip> --dry-run --json
  powerbi-cli report wireframe export <project-dir-or.pbip> --json
  powerbi-cli report layout auto --project <project-dir-or.pbip> --page <page-handle> --dry-run --json
  powerbi-cli report pages list --project <project-dir-or.pbip> --json
  powerbi-cli report pages show --project <project-dir-or.pbip> --handle <page-handle> --json
  powerbi-cli report pages add --project <project-dir-or.pbip> --display-name <name> --dry-run --json
  powerbi-cli report pages clone --project <project-dir-or.pbip> --from <page-name-or-handle> --new-name <ReportSectionX> --dry-run --json
  powerbi-cli report pages update --project <project-dir-or.pbip> --handle <page-handle> --display-name <name> --dry-run --json
  powerbi-cli report pages reorder --project <project-dir-or.pbip> --order <page-handle,...> --dry-run --json
  powerbi-cli report pages set-active --project <project-dir-or.pbip> --handle <page-handle> --dry-run --json
  powerbi-cli report pages delete-empty --project <project-dir-or.pbip> --handle <page-handle> --dry-run --json
  powerbi-cli report drilldown set-hierarchy --project <project-dir-or.pbip> --handle <visual-handle> --field <table[column]> --field <table[column]> --dry-run --json
  powerbi-cli report drillthrough set --project <project-dir-or.pbip> --page <page-handle> --target <table[column]> --dry-run --json
  powerbi-cli report drillthrough show --project <project-dir-or.pbip> --page <page-handle> --json
  powerbi-cli report drillthrough clear --project <project-dir-or.pbip> --page <page-handle> --dry-run --json
  powerbi-cli report bookmarks list --project <project-dir-or.pbip> --json
  powerbi-cli report bookmarks show --project <project-dir-or.pbip> --handle <bookmark-handle> --json
  powerbi-cli report bookmarks set-display-name --project <project-dir-or.pbip> --handle <bookmark-handle> --display-name <text> --dry-run --json
  powerbi-cli report bookmarks reorder --project <project-dir-or.pbip> --order <bookmark-handle,...> --dry-run --json
  powerbi-cli report bookmarks delete --project <project-dir-or.pbip> --handle <bookmark-handle> --dry-run --json
  powerbi-cli report filters list --project <project-dir-or.pbip> --json
  powerbi-cli report filters show --project <project-dir-or.pbip> --handle <filter-handle> --json
  powerbi-cli report filters add --project <project-dir-or.pbip> --target <table[column]> (--value <value> | --min <number> [--max <number>] | --top <N> --by <measure> | --relative <last|next|this> --unit <unit> --span <N>) --dry-run --json
  powerbi-cli report filters update --project <project-dir-or.pbip> --handle <filter-handle> (--display-name <label> | --values-json <json-array>) --dry-run --json
  powerbi-cli report filters delete --project <project-dir-or.pbip> --handle <filter-handle> --dry-run --json
  powerbi-cli report filters clear --project <project-dir-or.pbip> --page <page-handle> --dry-run --json
  powerbi-cli report slicers list --project <project-dir-or.pbip> --json
  powerbi-cli report slicers show --project <project-dir-or.pbip> --handle <slicer-handle> --json
  powerbi-cli report slicers clear --project <project-dir-or.pbip> --handle <slicer-handle> --dry-run --json
  powerbi-cli report interactions list --project <project-dir-or.pbip> --json
  powerbi-cli report interactions show --project <project-dir-or.pbip> --handle <interaction-handle> --json
  powerbi-cli report interactions set --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --type <mode> --dry-run --json
  powerbi-cli report interactions disable --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --dry-run --json
  powerbi-cli report themes show --project <project-dir-or.pbip> --json
  powerbi-cli report themes extract --project <project-dir-or.pbip> --out <theme-bundle.json> --json
  powerbi-cli report themes apply --project <project-dir-or.pbip> --bundle <theme-bundle.json> --dry-run --json
  powerbi-cli report themes presets list --json
  powerbi-cli report themes apply-preset --project <project-dir-or.pbip> --preset risk-dashboard --dry-run --json
  powerbi-cli report style extract --project <project-dir-or.pbip> --out <style-bundle.json> --json
  powerbi-cli report style apply --project <project-dir-or.pbip> --bundle <style-bundle.json> --dry-run --json
  powerbi-cli report visuals list --project <project-dir-or.pbip> --json
  powerbi-cli report visuals show --project <project-dir-or.pbip> --handle <visual-handle> --json
  powerbi-cli report visuals catalog --json
  powerbi-cli report visuals formatting list --project <project-dir-or.pbip> --json
  powerbi-cli report visuals formatting show --project <project-dir-or.pbip> --handle <visual-handle> --json
  powerbi-cli report visuals formatting conditional-formatting list --project <project-dir-or.pbip> --json
  powerbi-cli report visuals formatting conditional-formatting show --project <project-dir-or.pbip> --handle <visual-handle> --json
  powerbi-cli report visuals formatting extract --project <project-dir-or.pbip> --handle <visual-handle> --out <formatting-bundle.json> --json
  powerbi-cli report visuals formatting apply --project <project-dir-or.pbip> --handle <visual-handle> --bundle <formatting-bundle.json> --dry-run --json
  powerbi-cli report visuals formatting set-text --project <project-dir-or.pbip> --handle <visual-handle> --title <text> --dry-run --json
  powerbi-cli report visuals formatting set-color --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color <hex> --dry-run --json
  powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --title <title> --binding "role=Values,table=<table>,measure=<measure>" --dry-run --json
  powerbi-cli report visuals add-card --project <project-dir-or.pbip> --page <page-handle> --measure <Table.Measure> --title <text> --x <n> --y <n> --width <n> --height <n> --dry-run --json
  powerbi-cli report visuals add-slicer --project <project-dir-or.pbip> --page <page-handle> --field <Table.Column> --title <text> --x <n> --y <n> --width <n> --height <n> --dry-run --json
  powerbi-cli report visuals add-textbox --project <project-dir-or.pbip> --page <page-handle> --title <text> --text <paragraph> --x <n> --y <n> --width <n> --height <n> --dry-run --json
  powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json
  powerbi-cli report visuals delete --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json
  powerbi-cli report visuals set-position --project <project-dir-or.pbip> --handle <visual-handle> --x <n> --y <n> --dry-run --json
  powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> --handle <visual-handle> --bindings-json <json> --dry-run --json
  powerbi-cli report visuals set-topn-guard --project <project-dir-or.pbip> --handle <visual-handle> --field <Table.Column> --order-by <Table.Measure> --top <N> --dry-run --json
  powerbi-cli report visuals set-object --project <project-dir-or.pbip> --handle <visual-handle> --object <name> --property <name> --value <raw> --dry-run --json
  powerbi-cli report visuals set-display-name --project <project-dir-or.pbip> --handle <visual-handle> --role <Values|Category|Series|X|Y|Y2|Size|Rows|Columns|Tooltips> --display-name <text> --dry-run --json
  powerbi-cli report spec fields --schema <schema.json> --json
  powerbi-cli report plan --schema <schema.json> --profile <profile.json> --objective <goal> --out <dashboard.json> --json
  powerbi-cli report spec validate --schema <schema.json> --spec <dashboard.json> --json
  powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir> --json
  powerbi-cli handoff check <project-dir-or.pbip> [--target offline|work] --json
  powerbi-cli handoff rebind-plan <project-dir-or.pbip> [--out <file.md>] [--force] --json
  powerbi-cli --json validate [--strict] [--backend native|microsoft-report|all] <project-dir-or.pbip>

Agent contract:
  --json and --format json are global and may appear before or after the command.
  stdout is data; stderr is diagnostics. Mutations require --dry-run, --in-place, or --out-dir and emit follow-up inspect/validate/readback commands.

The scaffold command writes a PBIP project with PBIR report files and TMDL
semantic model files. Generated models use inline dummy M tables, not real data
connections or imported cache files.
"#
    .to_string()
}

pub(crate) fn help_json() -> Value {
    json!({
        "tool": "powerbi-cli",
        "summary": "Agent-oriented Power BI PBIP/PBIR/TMDL authoring helper",
        "contractVersion": CONTRACT_VERSION,
        "firstCommands": [
            "powerbi-cli version --json",
            "powerbi-cli --json capabilities",
            "powerbi-cli features list --json",
            "powerbi-cli robot-docs guide",
            "powerbi-cli --json doctor",
            "powerbi-cli schema validate <schema.json> --json",
            "powerbi-cli profile infer --schema <schema.json> --out <profile.json> --json",
            "powerbi-cli report spec fields --schema <schema.json> --profile <profile.json> --json",
            "powerbi-cli report plan --schema <schema.json> --profile <profile.json> --objective <dashboard-goal> --out <dashboard.json> --json",
            "powerbi-cli report spec validate --schema <schema.json> --profile <profile.json> --spec <dashboard.json> --json",
            "powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir> --json"
        ],
        "commands": command_paths()
    })
}

pub(crate) fn capabilities(args: &[String]) -> CliResult<Value> {
    let filter = parse_filter(args, "capabilities")?;
    let focused = filter.is_some();
    let mut commands = command_catalog();
    if let Some(filter) = &filter {
        commands.retain(|command| command_matches_filter(command, filter));
    }
    let matched_commands = commands.len();
    let hint = filter.as_ref().and_then(|filter| {
        (matched_commands == 0).then(|| {
            format!(
                "No live command matched `{filter}`. Run `powerbi-cli --json capabilities` for the full contract."
            )
        })
    });

    Ok(json!({
        "tool": "powerbi-cli",
        "binary": "powerbi-cli",
        "version": env!("CARGO_PKG_VERSION"),
        "contractVersion": CONTRACT_VERSION,
        "stability": "alpha-agent-contract",
        "primaryUser": "AI agents authoring offline-safe Power BI projects",
        "stdout": "data-only",
        "stderr": "diagnostics-only",
        "outputModes": ["json via --json or --format json; accepted before or after command"],
        "globalFlags": global_flags(),
        "exitCodes": exit_codes(),
        "diagnosticCodes": diagnostic_codes(),
        "responseShapes": response_shapes(),
        "featurePolicy": feature_policy_json(),
        "filter": filter,
        "scope": if focused { "focused" } else { "full" },
        "matchedCommands": matched_commands,
        "hint": hint,
        "commands": commands,
        "schemaManifest": if focused { Value::Null } else { schema_manifest() },
        "generatedVisualContract": if focused { Value::Null } else { generated_visual_contract() },
        "desktopProofedArchetypes": if focused { Value::Null } else { desktop_proofed_archetypes() },
        "formatTargets": if focused { Value::Null } else { format_targets() },
        "omittedCatalogs": if focused {
            json!(["schemaManifest", "generatedVisualContract", "desktopProofedArchetypes", "formatTargets"])
        } else {
            json!([])
        },
        "fullContractCommand": if focused {
            Value::String("powerbi-cli --json capabilities".to_string())
        } else {
            Value::Null
        },
        "proofLevels": proof_levels(),
        "architectureGuardrails": architecture_guardrails(),
        "designRules": design_rules()
    }))
}

pub(crate) fn robot_docs_json() -> Value {
    json!({
        "tool": "powerbi-cli",
        "contractVersion": CONTRACT_VERSION,
        "markdown": robot_docs_markdown(),
        "followUpCommands": [
            "powerbi-cli --json capabilities",
            "powerbi-cli --json doctor",
            "powerbi-cli --json validate <project-dir-or.pbip>"
        ]
    })
}

pub(crate) fn robot_docs_markdown() -> String {
    r#"# powerbi-cli Agent Guide

Use `powerbi-cli` to author PBIP/PBIR/TMDL projects away from corporate data. It does not write PBIX binaries, credentials, or Power BI Desktop cache files.

Rules for agents:
- Prefer `--json` for all machine reads. The flag may appear before or after the command.
- Successful JSON payloads are family-specific. Semantic mutation results and report build expose `changes[]`; readers may not. Failures use the stable stderr shape `{error:{code,exitCode,message,hint?,suggestedCommands?}}`.
- Execute strings from `next[]` and `suggestedCommands[]`; prose belongs in `instructions[]` or `notes[]`.
- Start with `powerbi-cli --json capabilities` and trust that payload over memory.
- Use `powerbi-cli version --json` for a cheap provenance check before relying on cached command knowledge.
- Use `powerbi-cli features list --json` to distinguish supported, read-only, planned, and explicitly refused Power BI feature surfaces. If a command returns `error.code = "unsupported_feature"`, stop or choose a supported workflow; do not raw-patch guessed PBIR/TMDL.
- Use `package inspect/extract/import/source-pack/export-plan` for PBIX/PBIT package boundaries. Extraction has streaming entry-count, per-entry, total-size, and compression-ratio limits. `source-pack` accepts only documented PBIP/PBIR/TMDL files and generated sidecars, refuses dot-directories/unknown files, and scans every included file before writing; `export-plan` is a Desktop handoff plan for opaque Desktop binaries.
- For arbitrary dashboards, start with `schema validate`, `profile infer`, `report plan`, `report spec validate`, then `report build`.
- After any scaffold, report build, or mutation, run the returned inspect and validate commands.
- Use `diff <before> <after> --json` to verify measure-level semantic changes after mutations; pass `--scope model.calculatedColumns` for calculated columns or `--scope model.relationships` for relationships.
- Use `model measures list/show/add/update/delete` for DAX measure authoring; `--expression-file <path|->` accepts UTF-8 multiline DAX as an alternative to `--expression` and trims trailing newlines. Updates refuse unsupported Desktop-authored TMDL metadata, local validation proves file structure, and Power BI Desktop remains the DAX compatibility oracle.
- Use `model columns set-sort-by` to set or clear a same-table TMDL `sortByColumn` property with guarded output semantics.
- Use `model calculated-columns list/show/add/update/delete` for DAX calculated column authoring; input type `date` normalizes to TMDL `dateTime` with a default `Short Date` format, updates refuse unsupported Desktop-authored TMDL metadata, and calculated columns may require refresh after Desktop opens the project.
- Reuse returned semantic-model handles. Literal `%` and `:` inside table, column, measure, and partition components are encoded as `%25` and `%3A` so handles round-trip without ambiguity.
- Use `model dax dependencies/lint/bridge-plan` to enumerate DAX expressions, static references, obvious broken dependencies, and validation boundaries. On an opted-in Windows oracle machine, `model dax execute` can run a bounded read-only EVALUATE query against the exact already-open PBIP or PBIX document; it never launches Desktop or returns the query text. `model live export-tmdl` uses the same exact live-engine identity and the pinned local Microsoft Modeling MCP to publish one credential-scanned semantic-model TMDL definition into a fresh output directory. It does not export report pages or claim full PBIX-to-PBIP conversion. PBIP live preflight ignores only each selected artifact's root `.pbi/` runtime directory; PBIX preflight verifies the package/report/DataModel shape. Strict offline validation, packaging, workflow, and handoff still reject PBIP runtime state.
- Use `model advanced inventory`, `model roles list/show`, `model perspectives list/show`, `model cultures list/show`, and `model expressions list/show` for advanced TMDL readback. Mutations remain fixture-gated.
- Use `model relationships list/show/add/update/delete` for model relationships. Endpoint rewiring is delete+add in this alpha surface; `update` changes active state and cross-filtering behavior.
- Use `model partitions list/show` to inspect generated dummy M partitions and their offline safety classification.
- Use `source-template add/list/show` to store credential-free SQL Server, PostgreSQL, ODBC, or Excel rebind metadata as sidecar JSON.
- Use `source-template apply` to replace one safe generated dummy partition with a concrete credential-free source. Existing recognized credential-free SQL, PostgreSQL, ODBC, or external-file sources require `--replace-existing` plus the exact `--confirm <partition-handle>`; unresolved placeholders, unknown/web sources, embedded credentials, and unconfirmed replacements are refused.
- Use `handoff rebind-plan` to map dummy partitions to source templates and generate a self-contained work-machine runbook; `--out <file.md>` refuses an existing file unless `--force` is passed.
- Use `fixture normalize` and `fixture verify` to create deterministic golden summaries for generated or Desktop-authored PBIP fixtures.
- Use `desktop open` for one interactive CLI-owned Power BI Desktop session for a PBIP or PBIX document and always finish with idempotent `desktop close`; opening another managed session closes the prior owned session first. PBIP preflight defaults to `strict`; use `--preflight normal` for structural validation without lint or explicit `--preflight skip` when a known lint defect must not block a Desktop proof loop. PBIX gets bounded native archive preflight and delegates rendering to Desktop. Use `desktop open-check` and `desktop screenshot` for one-shot evidence; they always attempt bounded identity-checked cleanup and report unresolved ownership. Launch/capture commands require an opt-in Windows oracle machine with `POWERBI_DESKTOP_ORACLE=1`; `desktop close` intentionally does not, so cleanup remains available. Default CI should treat oracle-unavailable as expected. `desktop-launch` and `desktop-window` are observation stages, not members of the closed proof-level ladder. Window/title signals and screenshots still do not prove canvas render or refresh.
- Use `report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir>` as the macro surface for generic dashboard generation; it compiles only supported spec features and returns proof/handoff follow-up commands.
- Use `report spec fields --schema <schema.json> [--profile <profile.json>]` to get exact column/measure binding references before writing a dashboard spec.
- Use `report plan --schema <schema.json> --profile <profile.json> --objective <goal> --out <dashboard.json>` to create a deterministic starter dashboard spec, then `report spec validate --schema <schema.json> --spec <dashboard.json>` before build.
- Use project-only `report design-plan --project <project>` to get visual opportunities from an already scaffolded project.
- Use `report tree/find/cat/query` for stable report-object navigation across pages, visuals, bindings, filters, slicers, bookmarks, and interactions. Use `--include-raw` only when you explicitly need raw PBIR JSON.
- Use `report audit` and `report sanitize plan/apply` before handoff when a Desktop-authored or template-derived report might contain persisted filter/slicer/bookmark state, literal values, or stale interaction references.
- Use `report pages list/show/add/clone/update/reorder/set-active/delete-empty`, `report layout auto`, `report drilldown set-hierarchy`, `report drillthrough set/show/clear`, `report bookmarks list/show/set-display-name/reorder/delete`, `report filters list/show/add/update/delete/clear`, `report slicers list/show/clear`, `report interactions list/show/set/disable`, and `report visuals list/show/catalog/formatting list/formatting show/formatting conditional-formatting list/show/formatting extract/formatting apply/formatting set-text/formatting set-color/add/add-card/add-slicer/add-textbox/clone/delete/set-position/set-bindings/set-topn-guard/set-object/set-display-name` for PBIR layout navigation, deterministic visual arrangement, chart hierarchy axes, same-report drillthrough page bindings, bookmark/filter/slicer/interaction inventory and readback, guarded categorical/range/TopN/relative-date filter authoring, type-preserving filter updates, deletion and owner-scoped clear, guarded slicer selection clear, guarded interaction overrides, guarded page cloning and metadata/order edits, visual type/role discovery, safe visual formatting inventory and bundle portability, conditional-formatting readback, typed title/static-color formatting and rejected alt-text cleanup, safe visual creation/cloning/deletion, small-visual scaffolding (KPI cards, slicers, reading-guide textboxes), geometry edits, field-well binding replacement, declarative visual TopN guard filters, curated visual object properties, and projection display names.
- Use `report style inspect/extract/apply/diff` for master-style bundles that combine report themeCollection and per-visual formatting payloads. Review literal text before applying a style bundle with `--allow-literal-text`.
- Use `report themes show/extract/apply`, `report themes presets list/show`, and `report themes apply-preset` for report-level theme bundles and built-in registered-resource theme presets. Theme copy is not per-visual formatting copy.
- Run `handoff check <project>` for an offline/dummy project. For a canonical live-source PBIP going to its work network, use `handoff check <project> --target work`; recognized connectors and unknown M explicitly trusted with the table annotation `PowerBICli_SourceKind = ModelDerived` are then accepted, while credentials, caches, binaries, embedded data, and unannotated unknown sources still fail.
- Start measure mutations with `--dry-run`; use `--in-place` or `--out-dir <dir>` only after the returned TMDL block looks right.
- Keep real data, credentials, gateway names, `.pbix`, `.pbit`, `.pbi/cache.abf`, and `localSettings.json` out of offline projects.
- Treat schema-golden visual bindings as exact local compatibility assertions, not automated Desktop proof. Card, tableEx, lineChart, scatterChart, and hundredPercentStackedColumnChart replicate Desktop-rendered 2026-08 pilot fixtures at schema-golden; pie, donut, matrix, and slicer retain separate manual canvas/refresh evidence. Same-report drillthrough currently has schema-golden proof; end-to-end Desktop interaction proof remains open.
- Bind measures to card Values and line-chart Y. Scatter X/Y/Size and 100% stacked-column Y also accept columns and emit the Desktop-proven explicit Sum Aggregation shape (`Sum(Table.Column)` / `Summe von Column`). Other bare value-axis columns remain unsupported_feature.
- Do not grow a monolith: add new command families in focused modules.

Common workflow:
1. `powerbi-cli schema validate <schema.json> --json`
2. `powerbi-cli profile infer --schema <schema.json> --out <profile.json> --json`
3. `powerbi-cli report spec fields --schema <schema.json> --profile <profile.json> --json`
4. `powerbi-cli report plan --schema <schema.json> --profile <profile.json> --objective <dashboard-goal> --out <dashboard.json> --json`
5. `powerbi-cli report spec validate --schema <schema.json> --profile <profile.json> --spec <dashboard.json> --json`
6. `powerbi-cli report build --schema <schema.json> --profile <profile.json> --spec <dashboard.json> --out-dir <project-dir> --json`
7. `powerbi-cli inspect --deep <project-dir> --json`
8. `powerbi-cli validate --strict <project-dir> --json`
9. `powerbi-cli handoff check <project-dir> --json`
10. `powerbi-cli fixture normalize <project-dir> --out <summary.json> --json`
11. Open the `.pbip` in Power BI Desktop at work and rebind dummy `#table(...)` partitions to corporate sources.
"#
    .to_string()
}

pub(crate) fn robot_triage() -> Value {
    json!({
        "tool": "powerbi-cli",
        "contractVersion": CONTRACT_VERSION,
        "quickRef": {
            "discover": "powerbi-cli --json capabilities",
            "version": "powerbi-cli version --json",
            "featureCatalog": "powerbi-cli features list --json",
            "guide": "powerbi-cli robot-docs guide",
            "robotTriage": "powerbi-cli robot-triage",
            "doctor": "powerbi-cli --json doctor",
            "skillStatus": "powerbi-cli skill status --json",
            "skillInstall": "powerbi-cli skill install --json",
            "desktopOpen": "powerbi-cli desktop open <project-dir-or.pbip-or.pbix> [--preflight strict|normal|skip] --json",
            "desktopClose": "powerbi-cli desktop close --json",
            "desktopOpenCheck": "powerbi-cli desktop open-check <project-dir-or.pbip-or.pbix> --json",
            "fixtureNormalize": "powerbi-cli fixture normalize <project-dir-or.pbip> --json",
            "fixtureVerify": "powerbi-cli fixture verify <project-dir-or.pbip> --expected <summary.json> --json",
            "schemaValidate": "powerbi-cli schema validate <schema.json> --json",
            "schemaNormalize": "powerbi-cli schema normalize <schema.json> --out <canonical.json> --json",
            "profileInfer": "powerbi-cli profile infer --schema <schema.json> --out <profile.json> --json",
            "profileValidate": "powerbi-cli profile validate <profile.json> --json",
            "reportSpecFields": "powerbi-cli report spec fields --schema <schema.json> --profile <profile.json> --json",
            "reportPlan": "powerbi-cli report plan --schema <schema.json> --profile <profile.json> --objective <dashboard-goal> --out <dashboard.json> --json",
            "reportSpecValidate": "powerbi-cli report spec validate --schema <schema.json> --profile <profile.json> --spec <dashboard.json> --json",
            "reportBuild": "powerbi-cli report build --schema <schema.json> --profile <profile.json> --spec <dashboard.json> --out-dir <project-dir> --json",
            "packageSourcePack": "powerbi-cli package source-pack --project <project-dir-or.pbip> --out <archive.pbit> --json",
            "scaffold": "powerbi-cli --json scaffold --schema examples/sales.schema.json --out-dir build/sales",
            "inspect": "powerbi-cli --json inspect <project-dir-or.pbip>",
            "diff": "powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> --json",
            "calculatedColumnList": "powerbi-cli model calculated-columns list --project <project-dir-or.pbip> --json",
            "calculatedColumnAddDryRun": "powerbi-cli model calculated-columns add --project <project-dir-or.pbip> --table <table> --name <column> --expression <dax> --data-type string --dry-run --json",
            "measureList": "powerbi-cli model measures list --project <project-dir-or.pbip> --json",
            "measureAddDryRun": "powerbi-cli model measures add --project <project-dir-or.pbip> --table <table> --name <measure> --expression <dax> --dry-run --json",
            "columnSetSortByDryRun": "powerbi-cli model columns set-sort-by --project <project-dir-or.pbip> --table <table> --column <column> --by <sort-column> --dry-run --json",
            "relationshipList": "powerbi-cli model relationships list --project <project-dir-or.pbip> --json",
            "relationshipAddDryRun": "powerbi-cli model relationships add --project <project-dir-or.pbip> --from-table <table> --from-column <column> --to-table <table> --to-column <column> --dry-run --json",
            "partitionList": "powerbi-cli model partitions list --project <project-dir-or.pbip> --json",
            "modelDaxBridgePlan": "powerbi-cli model dax bridge-plan --project <project-dir-or.pbip> --json",
            "modelDaxExecute": "POWERBI_DESKTOP_ORACLE=1 powerbi-cli model dax execute --project <project-dir-or.pbip-or.pbix> --query-file <query.dax> --allow-data-read --json",
            "modelLiveExportTmdl": "POWERBI_DESKTOP_ORACLE=1 powerbi-cli model live export-tmdl --document <project-dir-or.pbip-or.pbix> --out-dir <fresh-dir> --allow-model-read --json",
            "workflowSynthesize": "powerbi-cli workflow synthesize --project <project-dir-or.pbip> --expressions <expressions.tmdl> --out-dir <new-project-dir> --json",
            "sourceTemplateList": "powerbi-cli source-template list --project <project-dir-or.pbip> --json",
            "sourceTemplateAddSqlDryRun": "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind sql --dry-run --json",
            "sourceTemplateApplyDryRun": "powerbi-cli source-template apply --project <project-dir-or.pbip> --handle <source-template-handle> --server <server> --database <database> --dry-run --json",
            "reportDesignPlan": "powerbi-cli report design-plan --project <project-dir-or.pbip> --json",
            "reportTree": "powerbi-cli report tree --project <project-dir-or.pbip> --json",
            "reportFind": "powerbi-cli report find --project <project-dir-or.pbip> --kind visual --json",
            "reportCat": "powerbi-cli report cat --project <project-dir-or.pbip> --handle <object-handle> --json",
            "reportQuery": "powerbi-cli report query --project <project-dir-or.pbip> --selector kind:visual --json",
            "reportAudit": "powerbi-cli report audit --project <project-dir-or.pbip> --json",
            "reportSanitizePlan": "powerbi-cli report sanitize plan --project <project-dir-or.pbip> --json",
            "reportSanitizeApplyDryRun": "powerbi-cli report sanitize apply --project <project-dir-or.pbip> --dry-run --json",
            "reportLayoutAutoDryRun": "powerbi-cli report layout auto --project <project-dir-or.pbip> --page <page-handle> --preset overview --dry-run --json",
            "reportPagesList": "powerbi-cli report pages list --project <project-dir-or.pbip> --json",
            "reportPageAddDryRun": "powerbi-cli report pages add --project <project-dir-or.pbip> --display-name <name> --dry-run --json",
            "reportPageCloneDryRun": "powerbi-cli report pages clone --project <project-dir-or.pbip> --from <page-name-or-handle> --new-name <ReportSectionX> --dry-run --json",
            "reportPageSetActiveDryRun": "powerbi-cli report pages set-active --project <project-dir-or.pbip> --handle <page-handle> --dry-run --json",
            "reportDrilldownSetHierarchyDryRun": "powerbi-cli report drilldown set-hierarchy --project <project-dir-or.pbip> --handle <visual-handle> --field 'DimDate[FiscalYear]' --field 'DimDate[Month]' --dry-run --json",
            "reportDrillthroughSetDryRun": "powerbi-cli report drillthrough set --project <project-dir-or.pbip> --page <page-handle> --target <table[column]> --dry-run --json",
            "reportDrillthroughShow": "powerbi-cli report drillthrough show --project <project-dir-or.pbip> --page <page-handle> --json",
            "reportBookmarksList": "powerbi-cli report bookmarks list --project <project-dir-or.pbip> --json",
            "reportBookmarksShow": "powerbi-cli report bookmarks show --project <project-dir-or.pbip> --handle <bookmark-handle> --json",
            "reportFiltersList": "powerbi-cli report filters list --project <project-dir-or.pbip> --json",
            "reportFiltersShow": "powerbi-cli report filters show --project <project-dir-or.pbip> --handle <filter-handle> --json",
            "reportFilterAddDryRun": "powerbi-cli report filters add --project <project-dir-or.pbip> --target <table[column]> --value <value> --dry-run --json",
            "reportFilterUpdateDryRun": "powerbi-cli report filters update --project <project-dir-or.pbip> --handle <filter-handle> --display-name <label> --dry-run --json",
            "reportFilterDeleteDryRun": "powerbi-cli report filters delete --project <project-dir-or.pbip> --handle <filter-handle> --dry-run --json",
            "reportFilterClearPageDryRun": "powerbi-cli report filters clear --project <project-dir-or.pbip> --page <page-handle> --dry-run --json",
            "reportSlicersList": "powerbi-cli report slicers list --project <project-dir-or.pbip> --json",
            "reportSlicersShow": "powerbi-cli report slicers show --project <project-dir-or.pbip> --handle <slicer-handle> --json",
            "reportSlicerClearDryRun": "powerbi-cli report slicers clear --project <project-dir-or.pbip> --handle <slicer-handle> --dry-run --json",
            "reportInteractionsList": "powerbi-cli report interactions list --project <project-dir-or.pbip> --json",
            "reportInteractionsShow": "powerbi-cli report interactions show --project <project-dir-or.pbip> --handle <interaction-handle> --json",
            "reportInteractionSetDryRun": "powerbi-cli report interactions set --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --type DataFilter --dry-run --json",
            "reportInteractionDisableDryRun": "powerbi-cli report interactions disable --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --dry-run --json",
            "reportThemesShow": "powerbi-cli report themes show --project <project-dir-or.pbip> --json",
            "reportThemesExtract": "powerbi-cli report themes extract --project <source-project-or.pbip> --out theme-bundle.json --json",
            "reportThemesApplyDryRun": "powerbi-cli report themes apply --project <target-project-or.pbip> --bundle theme-bundle.json --dry-run --json",
            "reportThemesPresets": "powerbi-cli report themes presets list --json",
            "reportThemesApplyPresetDryRun": "powerbi-cli report themes apply-preset --project <target-project-or.pbip> --preset risk-dashboard --dry-run --json",
            "reportVisualsList": "powerbi-cli report visuals list --project <project-dir-or.pbip> --json",
            "reportVisualsCatalog": "powerbi-cli report visuals catalog --json",
            "reportVisualFormattingList": "powerbi-cli report visuals formatting list --project <project-dir-or.pbip> --json",
            "reportVisualFormattingShow": "powerbi-cli report visuals formatting show --project <project-dir-or.pbip> --handle <visual-handle> --json",
            "reportVisualFormattingExtract": "powerbi-cli report visuals formatting extract --project <source-project-or.pbip> --handle <source-visual-handle> --out visual-formatting-bundle.json --json",
            "reportVisualFormattingApplyDryRun": "powerbi-cli report visuals formatting apply --project <target-project-or.pbip> --handle <target-visual-handle> --bundle visual-formatting-bundle.json --dry-run --json",
            "reportVisualFormattingSetTextDryRun": "powerbi-cli report visuals formatting set-text --project <project-dir-or.pbip> --handle <visual-handle> --title <text> --dry-run --json",
            "reportVisualFormattingSetColorDryRun": "powerbi-cli report visuals formatting set-color --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json",
            "reportVisualAddDryRun": "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --title <title> --binding \"role=Values,table=<table>,measure=<measure>\" --dry-run --json",
            "reportVisualAddCardDryRun": "powerbi-cli report visuals add-card --project <project-dir-or.pbip> --page <page-handle> --measure <Table.Measure> --title <text> --x 40 --y 40 --width 200 --height 120 --dry-run --json",
            "reportVisualAddSlicerDryRun": "powerbi-cli report visuals add-slicer --project <project-dir-or.pbip> --page <page-handle> --field <Table.Column> --title <text> --x 40 --y 40 --width 240 --height 80 --dry-run --json",
            "reportVisualAddTextboxDryRun": "powerbi-cli report visuals add-textbox --project <project-dir-or.pbip> --page <page-handle> --title <text> --text <paragraph> --x 40 --y 520 --width 400 --height 120 --dry-run --json",
            "reportVisualCloneDryRun": "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json",
            "reportVisualDeleteDryRun": "powerbi-cli report visuals delete --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json",
            "reportVisualSetPositionDryRun": "powerbi-cli report visuals set-position --project <project-dir-or.pbip> --handle <visual-handle> --x 40 --y 40 --dry-run --json",
            "reportVisualSetBindingsDryRun": "powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> --handle <visual-handle> --bindings-json '[{\"role\":\"Values\",\"table\":\"FactSales\",\"measure\":\"Total Revenue\"}]' --dry-run --json",
            "reportVisualSetTopnGuardDryRun": "powerbi-cli report visuals set-topn-guard --project <project-dir-or.pbip> --handle <visual-handle> --field DimCustomer.CustomerName --order-by 'FactSales[Total Revenue]' --top 28 --dry-run --json",
            "reportVisualSetObjectDryRun": "powerbi-cli report visuals set-object --project <project-dir-or.pbip> --handle <visual-handle> --object categoryLabels --property fontSize --value 20 --dry-run --json",
            "reportVisualSetDisplayNameDryRun": "powerbi-cli report visuals set-display-name --project <project-dir-or.pbip> --handle <visual-handle> --role Values --display-name <text> --dry-run --json",
            "handoffCheck": "powerbi-cli handoff check <project-dir-or.pbip> --json",
            "handoffRebindPlan": "powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json",
            "validate": "powerbi-cli --json validate <project-dir-or.pbip>"
        },
        "recommendedNext": [
            "Run capabilities and read commands[].followUpFields before mutating.",
            "For arbitrary dashboards, validate schema/profile, run report plan or author a spec, validate the spec, then use report build.",
            "Do not expand visual families without Desktop-authored golden fixtures."
        ],
        "health": {
            "offlineAuthoring": true,
            "pbixGeneration": false,
            "desktopOracleRequiredForCompatibilityClaims": true,
            "noFakeFallbacks": true,
            "monolithGuard": "new features must land in focused modules, not src/main.rs"
        },
        "commands": command_catalog()
    })
}

fn parse_filter(args: &[String], command: &str) -> CliResult<Option<String>> {
    let mut filter = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--for" => {
                filter = Some(
                    args.get(i + 1)
                        .ok_or_else(|| CliError::invalid_args("--for requires a value"))?
                        .to_ascii_lowercase(),
                );
                i += 2;
            }
            other => {
                return Err(CliError::invalid_args(format!("unknown {command} flag: {other}"))
                    .with_hint(format!(
                        "Run `powerbi-cli --json {command}` or `powerbi-cli --json {command} --for <filter>`."
                    ))
                    .with_suggested_command(format!("powerbi-cli --json {command}")));
            }
        }
    }
    Ok(filter)
}

fn command_matches_filter(command: &Value, filter: &str) -> bool {
    command["path"]
        .as_str()
        .unwrap_or_default()
        .contains(filter)
        || command["summary"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains(filter)
        || command["tags"].as_array().is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(filter)
            })
        })
}

pub(crate) fn suggested_command_path(args: &[String]) -> Option<String> {
    let attempted = normalized_command_tokens(args);
    if attempted.is_empty() {
        return None;
    }
    let paths = command_paths();

    let suffix_matches = paths
        .iter()
        .filter(|path| {
            normalized_command_tokens(&[path.as_str().to_string()]).ends_with(&attempted)
        })
        .collect::<Vec<_>>();
    if suffix_matches.len() == 1 {
        return suffix_matches.first().map(|path| (*path).clone());
    }

    let mut attempted_sorted = attempted.clone();
    attempted_sorted.sort();
    let reordered_matches = paths
        .iter()
        .filter(|path| {
            let mut candidate = normalized_command_tokens(&[path.as_str().to_string()]);
            candidate.sort();
            candidate == attempted_sorted
        })
        .collect::<Vec<_>>();
    (reordered_matches.len() == 1).then(|| reordered_matches[0].clone())
}

fn normalized_command_tokens(args: &[String]) -> Vec<String> {
    args.iter()
        .take_while(|arg| !arg.starts_with('-'))
        .flat_map(|arg| {
            arg.split(|character: char| character.is_whitespace() || character == '-')
                .filter(|part| !part.is_empty())
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn command_paths() -> Vec<String> {
    command_catalog()
        .into_iter()
        .filter_map(|command| command["path"].as_str().map(ToOwned::to_owned))
        .collect()
}

fn command_catalog() -> Vec<Value> {
    let mut commands = vec![
        json!({
            "path": "capabilities",
            "usage": "powerbi-cli --json capabilities [--for <filter>]",
            "summary": "List the agent-facing command contract; focused queries omit unrelated large catalogs",
            "tags": ["agent", "discovery", "contract"],
            "readOnly": true,
            "mutates": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "capabilities.v1",
            "flags": ["--for <filter>", "--json", "--format json"],
            "examples": ["powerbi-cli --json capabilities", "powerbi-cli capabilities --json --for scaffold"],
            "followUpFields": ["scope", "commands[].usage", "commands[].examples", "exitCodes", "omittedCatalogs", "fullContractCommand", "schemaManifest"]
        }),
        json!({
            "path": "version",
            "usage": "powerbi-cli version --json",
            "summary": "Return the binary version and agent contract version for provenance checks",
            "tags": ["agent", "discovery", "version", "contract"],
            "readOnly": true,
            "mutates": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.version.v1",
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli version --json", "powerbi-cli --json version"],
            "followUpFields": ["tool", "binary", "version", "contractVersion"]
        }),
        json!({
            "path": "features list",
            "usage": "powerbi-cli features list [--for <feature-filter>] --json",
            "summary": "List supported, fixture-gated, planned, and explicitly refused Power BI feature surfaces",
            "tags": ["agent", "discovery", "features", "proof", "no-fallback"],
            "readOnly": true,
            "mutates": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.features.v1",
            "flags": ["--for <feature-filter>", "--json", "--format json"],
            "examples": [
                "powerbi-cli features list --json",
                "powerbi-cli features list --for drillthrough --json",
                "powerbi-cli features list --for supported --json"
            ],
            "followUpFields": ["policy.noFakeFallbacks", "features[].id", "features[].status", "features[].support", "features[].proofLevel", "features[].refusalCode"]
        }),
    ];
    commands.extend(workflow_pkg::package_commands());
    commands.extend([
        json!({
            "path": "robot-docs guide",
            "usage": "powerbi-cli robot-docs guide [--json]",
            "summary": "Print the in-tool agent guide so agents do not need external docs",
            "tags": ["agent", "guide", "docs"],
            "readOnly": true,
            "mutates": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "robotDocsGuide.v1",
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli robot-docs guide", "powerbi-cli --json robot-docs guide"],
            "followUpFields": ["markdown", "followUpCommands"]
        }),
        json!({
            "path": "--robot-triage",
            "aliases": ["robot-triage"],
            "usage": "powerbi-cli --robot-triage",
            "summary": "Return quick reference, recommended next steps, health, and command catalog in one call",
            "tags": ["agent", "triage", "mega-command"],
            "readOnly": true,
            "mutates": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "robotTriage.v1",
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli --robot-triage", "powerbi-cli --json --robot-triage"],
            "followUpFields": ["quickRef", "recommendedNext", "health", "commands"]
        }),
        json!({
            "path": "robot-triage",
            "aliases": ["--robot-triage"],
            "usage": "powerbi-cli robot-triage",
            "summary": "Alias for --robot-triage when an agent expects a normal command token",
            "tags": ["agent", "triage", "mega-command", "alias"],
            "readOnly": true,
            "mutates": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "robotTriage.v1",
            "jsonOnly": true,
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli robot-triage", "powerbi-cli --json robot-triage"],
            "followUpFields": ["quickRef", "recommendedNext", "health", "commands"]
        }),
        json!({
            "path": "doctor",
            "usage": "powerbi-cli --json doctor",
            "summary": "Report local Power BI Desktop detection and format assumptions",
            "tags": ["agent", "diagnostics", "desktop"],
            "readOnly": true,
            "mutates": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.doctor.v1",
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli doctor --json"],
            "followUpFields": ["schema", "ok", "exitCode", "checks[].id", "checks[].status", "checks[].next", "checks[].instructions", "powerBiDesktop", "microsoftIntegrations", "formatAssumptions", "offlineSafety", "next"]
        }),
    ]);
    commands.extend(workflow_pkg::workflow_commands());
    commands.extend(integrations::commands());
    commands.extend(desktop::commands());
    commands.extend([
        json!({
            "path": "fixture normalize",
            "aliases": ["fixture summary", "fixtures normalize"],
            "usage": "powerbi-cli fixture normalize <project-dir-or.pbip> [--out <summary.json>] --json",
            "summary": "Emit a deterministic path-free summary for generated or Desktop-authored PBIP golden fixtures",
            "tags": ["fixture", "golden", "summary", "desktop", "oracle", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "writesArtifactWhenOutProvided": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.fixture.summary.v1",
            "flags": ["<project-dir-or.pbip>", "--project <project-dir-or.pbip>", "--out <summary.json>", "--json", "--format json"],
            "examples": ["powerbi-cli fixture normalize build/sales --json", "powerbi-cli fixture normalize build/sales --out testdata/golden/sales.summary.json --json"],
            "followUpFields": ["fingerprint", "counts", "model", "report", "pbir.pages[].visuals[].fingerprints.visualContainerObjects", "verification", "next"]
        }),
        json!({
            "path": "fixture verify",
            "aliases": ["fixtures verify"],
            "usage": "powerbi-cli fixture verify <project-dir-or.pbip> --expected <summary.json> [--write-actual <path>] --json",
            "summary": "Compare a project against a committed normalized fixture summary, returning the actual JSON and pointer differences without writing by default",
            "tags": ["fixture", "golden", "summary", "verify", "desktop", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "readOnlyByDefault": true,
            "mutatingFlags": ["--write-actual <path>"],
            "optionalArtifactWrite": "--write-actual writes the actual summary only on mismatch; without it fixture verify performs no writes",
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.fixture.summary.v1",
            "flags": ["<project-dir-or.pbip>", "--project <project-dir-or.pbip>", "--expected <summary.json>", "--write-actual <path>", "--json", "--format json"],
            "examples": ["powerbi-cli fixture verify build/sales --expected testdata/golden/sales.summary.json --json", "powerbi-cli fixture verify build/sales --expected testdata/golden/sales.summary.json --write-actual build/sales.actual.json --json"],
            "followUpFields": ["ok", "exitCode", "fingerprint", "verification.same", "verification.differences", "verification.actual", "verification.actualWritten"]
        }),
        json!({
            "path": "scaffold",
            "usage": "powerbi-cli --json scaffold --schema <schema.json> --out-dir <project-dir> [--force]",
            "summary": "Create an offline-safe PBIP project from a schema manifest",
            "tags": ["pbip", "pbir", "tmdl", "offline", "semantic-model"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "beta-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "scaffoldResult.v1",
            "flags": ["--schema <schema.json>", "--out-dir <project-dir>", "--out <project-dir>", "--force", "--json", "--format json"],
            "examples": [
                "powerbi-cli --json scaffold --schema examples/sales.schema.json --out-dir build/sales",
                "powerbi-cli scaffold --schema examples/archetypes/regional-sales.schema.json --out-dir build/regional-sales --json"
            ],
            "followUpFields": ["projectDir", "pbip", "reportDir", "semanticModelDir", "counts", "next", "instructions"]
        }),
        json!({
            "path": "schema validate",
            "usage": "powerbi-cli schema validate <schema.json> --json",
            "summary": "Validate a data schema manifest before report planning or PBIP generation",
            "tags": ["schema", "manifest", "dashboard", "agent", "validation"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.schema.validate.v1",
            "flags": ["<schema.json>", "--schema <schema.json>", "--json", "--format json"],
            "examples": ["powerbi-cli schema validate examples/sales.schema.json --json"],
            "followUpFields": ["ok", "counts", "tables", "warnings", "errors", "next"]
        }),
        json!({
            "path": "schema normalize",
            "usage": "powerbi-cli schema normalize <schema.json> --out <canonical.json> --json",
            "summary": "Write a canonical pretty-printed schema manifest for review and reproducible dashboard builds",
            "tags": ["schema", "manifest", "normalize", "golden", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "writesArtifact": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.schema.normalize.v1",
            "flags": ["<schema.json>", "--schema <schema.json>", "--out <canonical.json>", "--json", "--format json"],
            "examples": ["powerbi-cli schema normalize examples/sales.schema.json --out build/sales.schema.normalized.json --json"],
            "followUpFields": ["ok", "schemaPath", "normalizedOut", "counts", "next"]
        }),
        json!({
            "path": "profile infer",
            "usage": "powerbi-cli profile infer --schema <schema.json> [--out <profile.json>] --json",
            "summary": "Infer an advisory data profile from schema metadata and embedded dummy rows",
            "tags": ["profile", "schema", "dashboard", "inference", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "writesArtifactWhenOutProvided": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.profile.infer.v1",
            "flags": ["--schema <schema.json>", "--out <profile.json>", "--rows <dummy.csv|json> (planned)", "--json", "--format json"],
            "examples": ["powerbi-cli profile infer --schema examples/sales.schema.json --out build/sales.profile.json --json"],
            "followUpFields": ["profile", "profile.tables", "profile.candidates", "next"]
        }),
        json!({
            "path": "profile validate",
            "usage": "powerbi-cli profile validate <profile.json> --json",
            "summary": "Validate a data profile document used by dashboard planning/build flows",
            "tags": ["profile", "validation", "dashboard", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.profile.validate.v1",
            "flags": ["<profile.json>", "--json", "--format json"],
            "examples": ["powerbi-cli profile validate build/sales.profile.json --json"],
            "followUpFields": ["ok", "summary", "errors", "next"]
        }),
        json!({
            "path": "profile summarize",
            "usage": "powerbi-cli profile summarize <profile.json> --json",
            "summary": "Return a compact summary of a dashboard data profile",
            "tags": ["profile", "summary", "dashboard", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.profile.summary.v1",
            "flags": ["<profile.json>", "--json", "--format json"],
            "examples": ["powerbi-cli profile summarize build/sales.profile.json --json"],
            "followUpFields": ["ok", "summary", "errors"]
        }),
        json!({
            "path": "inspect",
            "usage": "powerbi-cli --json inspect [--deep] <project-dir-or.pbip>",
            "summary": "Summarize a PBIP project and, with --deep, return stable handles for report/model objects",
            "tags": ["pbip", "inspect", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "inspectResult.v1",
            "flags": ["--deep", "--json", "--format json"],
            "examples": ["powerbi-cli inspect build/sales --json", "powerbi-cli inspect --deep build/sales --json"],
            "followUpFields": ["projectDir", "counts", "warnings", "errors", "deep.handles", "deep.model.tables", "deep.report.pages"]
        }),
        json!({
            "path": "lint",
            "usage": "powerbi-cli lint <project-dir-or.pbip> --json",
            "summary": "Run typed PBIP/PBIR/TMDL quality checks, including heuristic M buffer-reuse and untyped-expansion warnings, and return structured findings",
            "tags": ["pbip", "pbir", "tmdl", "m", "validation", "lint", "buffer", "expansion", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "lintResult.v1",
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli lint build/sales --json"],
            "diagnosticCodes": ["m.unbuffered_reuse", "m.untyped_expansion"],
            "limitations": [
                "m.unbuffered_reuse is a warning-only heuristic over partition and named-expression M let steps; it does not prove folding or refresh performance and never fails validation by itself.",
                "m.untyped_expansion is a warning-only heuristic over literal Table.ExpandTableColumn name lists in partition M; it warns only when an expanded column maps to a numeric TMDL sourceColumn without Table.TransformColumnTypes, and never fails validation by itself."
            ],
            "followUpFields": ["ok", "counts", "findings", "next"]
        }),
        json!({
            "path": "diff",
            "usage": "powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> [--scope model.measures|model.calculatedColumns|model.relationships] --json",
            "summary": "Compare two PBIP projects using normalized semantic summaries and stable handles",
            "tags": ["pbip", "tmdl", "diff", "semantic", "measure", "calculated-column", "relationship", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "diffResult.v1",
            "flags": ["--scope model.measures", "--scope model.calculatedColumns", "--scope model.relationships", "--json", "--format json"],
            "examples": ["powerbi-cli diff build/sales build/sales-v2 --json", "powerbi-cli diff build/sales build/sales-v2 --scope model.calculatedColumns --json", "powerbi-cli diff build/sales build/sales-v2 --scope model.relationships --json"],
            "followUpFields": ["same", "summary", "changes[].kind", "changes[].op", "changes[].handle", "changes[].fieldsChanged", "changes[].before", "changes[].after", "next"]
        }),
    ]);
    commands.extend(model::commands());
    commands.extend(workflow_pkg::source_template_commands());
    commands.extend(report::commands());
    commands.extend([
        json!({
            "path": "handoff check",
            "usage": "powerbi-cli handoff check <project-dir-or.pbip> [--target offline|work] --json",
            "summary": "Classify an offline/dummy or work-network/live-source PBIP handoff after partition-shape, credential, PII-suspect text, cache, binary, and embedded-data checks",
            "tags": ["handoff", "offline", "work", "safety", "partition", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "handoffCheck.v1",
            "flags": ["--project <project-dir-or.pbip>", "--target offline|work", "--json", "--format json"],
            "examples": ["powerbi-cli handoff check build/sales --json", "powerbi-cli handoff check report/live.pbip --target work --json", "powerbi-cli handoff-check build/sales --json"],
            "followUpFields": ["ok", "exitCode", "target", "sourceMode", "status", "safeForOfflineHandoff", "safeForWorkHandoff", "counts.safeForTargetPartitions", "counts.acceptedLivePartitions", "counts.reviewPartitions", "counts.reviewFindings", "findings", "partitions", "next", "instructions"]
        }),
        json!({
            "path": "handoff rebind-plan",
            "aliases": ["handoff rebind", "handoff-rebind-plan"],
            "usage": "powerbi-cli handoff rebind-plan <project-dir-or.pbip> [--project <project-dir-or.pbip>] [--templates <source-templates.json|->] [--table <table>] [--partition <partition-handle>] [--allow-unmapped] [--out <file.md>] [--force] --json",
            "summary": "Generate a redacted work-machine rebind plan and suppress runbook materialization when a template or partition contains credentials",
            "tags": ["handoff", "offline", "rebind", "source-template", "partition", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "writesArtifactWhenOutProvided": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.handoff.rebind-plan.v1",
            "flags": ["--project <project-dir-or.pbip>", "--templates <source-templates.json|->", "--table <table>", "--partition <partition-handle-or-name>", "--allow-unmapped", "--out <file.md>", "--out-file <file.md>", "--force", "--json", "--format json"],
            "examples": ["powerbi-cli handoff rebind-plan build/sales --json", "powerbi-cli handoff rebind-plan build/sales --out work-machine-rebind.md --json", "powerbi-cli handoff rebind-plan build/sales --out work-machine-rebind.md --force --json", "powerbi-cli handoff rebind build/sales --json", "powerbi-cli handoff-rebind-plan build/sales --json"],
            "followUpFields": ["ok", "complete", "status", "counts", "plans[].partitionHandle", "plans[].template", "instructionsMarkdown", "runbookRequestedPath", "runbookPath", "runbookWritten", "materializationBlocked", "materializationBlockReasons", "handoffCheckCommand", "validateCommand", "next"]
        }),
        json!({
            "path": "validate",
            "usage": "powerbi-cli --json validate [--strict] [--backend native|microsoft-report|all] <project-dir-or.pbip>",
            "summary": "Run native PBIP/PBIR/TMDL validation by default, or explicitly add the exact official Microsoft report validator",
            "tags": ["pbip", "pbir", "tmdl", "validation", "offline", "microsoft", "no-fallback"],
            "readOnly": true,
            "mutates": false,
            "stability": "stable-shape",
            "proofLevel": "unit-smoke",
            "outputSchema": "validateResult.v1",
            "outputSchemas": {
                "native": "validateResult.v1",
                "microsoft-report": "powerbi-cli.validate.microsoft-report.v1",
                "all": "powerbi-cli.validate.all.v1"
            },
            "flags": ["--strict", "--backend native|microsoft-report|all", "--json", "--format json"],
            "examples": ["powerbi-cli --json validate build/sales", "powerbi-cli validate --strict build/sales --json", "powerbi-cli validate build/sales --backend microsoft-report --json", "powerbi-cli validate build/sales --strict --backend all --json"],
            "limitations": ["Native remains the default. microsoft-report runs only the installed exact official validator with --no-schema and emits powerbi-cli.validate.microsoft-report.v1. all requires both validators to complete successfully."],
            "followUpFields": ["ok", "exitCode", "backend", "counts", "warnings", "errors", "lint", "validators.native", "validators.microsoftReport"]
        }),
    ]);
    commands
}

fn global_flags() -> Vec<Value> {
    vec![
        json!({"flag": "--json", "summary": "Emit machine-readable JSON on stdout", "acceptedAnywhere": true}),
        json!({"flag": "--format json", "summary": "Alias for --json", "acceptedAnywhere": true}),
    ]
}

fn exit_codes() -> Vec<Value> {
    vec![
        json!({"code": EXIT_SUCCESS, "name": "success", "meaning": "Command completed successfully"}),
        json!({"code": EXIT_INVALID_ARGS, "name": "invalid_args", "meaning": "The invocation or manifest input is invalid"}),
        json!({"code": EXIT_FILE_NOT_FOUND, "name": "file_not_found", "meaning": "A requested project, schema, or referenced file was missing"}),
        json!({"code": EXIT_VALIDATION_FAILED, "name": "validation_failed", "meaning": "PBIP/PBIR/TMDL structure or offline-safety validation failed"}),
        json!({"code": EXIT_PROOF_INCOMPLETE, "name": "proof_incomplete", "meaning": "An oracle launch succeeded, but the requested higher-level proof or evidence was not completed before its observation budget expired"}),
        json!({"code": EXIT_ORACLE_UNAVAILABLE, "name": "oracle_unavailable", "meaning": "A requested Desktop oracle proof is unavailable on this machine or not explicitly enabled"}),
        json!({"code": EXIT_ORACLE_FAILED, "name": "oracle_failed", "meaning": "A requested Desktop oracle proof was attempted but failed"}),
        json!({"code": EXIT_UNEXPECTED, "name": "unexpected", "meaning": "Unexpected filesystem or serialization failure"}),
    ]
}

fn diagnostic_codes() -> Vec<Value> {
    vec![
        json!({"code": "invalid_args", "exitCode": EXIT_INVALID_ARGS}),
        json!({"code": "unsupported_feature", "exitCode": EXIT_INVALID_ARGS}),
        json!({"code": "file_not_found", "exitCode": EXIT_FILE_NOT_FOUND}),
        json!({"code": "validation_failed", "exitCode": EXIT_VALIDATION_FAILED}),
        json!({"code": "integrity_failed", "exitCode": EXIT_VALIDATION_FAILED}),
        json!({"code": "proof_incomplete", "exitCode": EXIT_PROOF_INCOMPLETE}),
        json!({"code": "oracle_unavailable", "exitCode": EXIT_ORACLE_UNAVAILABLE}),
        json!({"code": "dependency_unavailable", "exitCode": EXIT_ORACLE_UNAVAILABLE}),
        json!({"code": "oracle_failed", "exitCode": EXIT_ORACLE_FAILED}),
        json!({"code": "backend_failed", "exitCode": EXIT_ORACLE_FAILED}),
        json!({"code": "protocol_failed", "exitCode": EXIT_ORACLE_FAILED}),
        json!({"code": "unexpected", "exitCode": EXIT_UNEXPECTED}),
    ]
}

fn schema_manifest() -> Value {
    let mut manifest = json!({
        "fields": ["name", "displayName", "locale", "tables", "relationships", "pages"],
        "tableFields": ["name", "columns", "measures", "rows"],
        "columnFields": ["name", "dataType", "description", "formatString", "sourceColumn", "isHidden", "isKey", "summarizeBy", "sortByColumn"],
        "calculatedColumnFields": ["name", "expression", "dataType", "description", "formatString", "summarizeBy", "displayFolder", "isHidden"],
        "measureFields": ["name", "expression", "description", "formatString", "displayFolder"],
        "relationshipFields": ["name", "fromTable", "fromColumn", "toTable", "toColumn", "crossFilteringBehavior", "isActive"],
        "semanticModelHandleEncoding": {
            "separator": ":",
            "componentEscapes": [
                {"character": "%", "encoding": "%25"},
                {"character": ":", "encoding": "%3A"}
            ],
            "appliesTo": ["table", "measure", "column", "partition"]
        },
        "partitionFields": ["handle", "table", "name", "expressionKind", "mode", "sourceKind", "offlineSafety", "sourcePreview", "source", "sourceIncluded"],
        "partitionSourceKinds": ["dummyMTable", "modelDerived", "sqlDatabase", "postgresqlDatabase", "odbcDataSource", "webContents", "externalFile", "unknown", "missing"],
        "modelDaxBridgePlanFields": ["ok", "projectDir", "counts.measures", "counts.calculatedColumns", "daxInventory.measures[].handle", "daxInventory.measures[].expression", "daxInventory.calculatedColumns[].handle", "daxInventory.calculatedColumns[].expression", "bridge.required", "bridge.supportedEngines", "bridge.noFakeFallbacks", "validationBridge.offlineDaxParser.available", "next"],
        "modelDaxExecuteFields": ["ok", "exitCode", "document.kind", "document.path", "query.source", "query.lengthBytes", "query.fingerprint", "query.textReturned", "safety.readOnlyQueryFormsOnly", "safety.allowDataRead", "safety.exactOpenProjectMatchRequired", "safety.autoLaunch", "safety.modelWrites", "limits.maxRows", "limits.maxCellChars", "limits.timeoutMs", "stage", "engine.kind", "engine.desktopProcessId", "engine.modelProcessId", "engine.port", "columns[].ordinal", "columns[].name", "columns[].dataType", "rows", "counts.rows", "counts.columns", "counts.truncatedCells", "truncation.rows", "truncation.cells", "runtime.temporaryFilesRemoved", "diagnostics", "validation", "next"],
        "modelStaticTableMutationFields": ["ok", "dryRun", "mode", "projectModified", "target.handle", "target.table", "target.column", "target.columns", "tablePlan.kind", "tablePlan.dataType", "tablePlan.dataTypes", "tablePlan.columnCount", "tablePlan.rowCount", "tablePlan.uniqueFirstColumn", "tablePlan.relationshipCount", "changes", "validation", "readbackCommand", "inspectCommand", "validateCommand"],
        "modelDaxDependenciesFields": ["analysisBoundary.daxEngineValidated", "counts", "expressions[].handle", "expressions[].tableColumns", "expressions[].measureReferences", "graph.edges", "findings", "validation", "next"],
        "modelAdvancedInventoryFields": ["families[].family", "families[].count", "families[].records[].handle", "families[].records[].summary", "validation", "next"],
        "packageInspectFields": ["package", "packageKind", "packageClass", "archive.kind", "archive.entries", "archive.byCategory", "sourceRoots", "support.canExtractSafeMetadata", "support.canImportSourceProject", "support.canWriteBinaryPackage", "entries[].name", "entries[].category", "entries[].safeForMetadataExtract", "next"],
        "sourceTemplateFields": ["handle", "name", "partitionHandle", "table", "partition", "kind", "parameters", "mTemplate", "description", "safety"],
        "sourceTemplateKinds": ["sql", "postgres", "odbc", "excel"],
        "rebindPlanFields": ["handle", "partitionHandle", "table", "partition", "currentSourceKind", "sourceRange", "template", "mTemplate", "manualSteps"],
        "profileFields": ["schema", "source", "tables", "tables[].role", "tables[].rowCount", "tables[].columns", "tables[].columns[].roles", "candidates.factTables", "candidates.dimensionTables", "candidates.dateColumns", "candidates.numericColumns", "candidates.categoryColumns", "warnings"],
        "dashboardSpecFields": ["schema", "report.name", "report.displayName", "report.audience", "report.questions", "model.measures", "pages[].id", "pages[].displayName", "pages[].size", "pages[].visuals", "pages[].visuals[].type", "pages[].visuals[].mode", "pages[].visuals[].singleSelect", "pages[].visuals[].bindings", "pages[].visuals[].bindings[].field"],
        "reportSpecFieldsInventoryFields": ["ok", "exitCode", "supportedVisualTypes", "tables[].name", "tables[].profileRole", "tables[].rowCount", "tables[].columns[].reference", "tables[].columns[].roles", "tables[].columns[].structuredBinding", "tables[].measures[].reference", "tables[].measures[].structuredBinding", "fields[].reference", "examples", "next"],
        "reportBuildFields": ["ok", "changed", "dryRun", "projectDir", "inputs", "compiled.counts", "changes[].kind", "changes[].action", "changes[].path", "changes[].before", "changes[].after", "profileSummary", "executedPrimitives", "operations", "warnings", "inspectCommand", "validateCommand", "handoffCheckCommand", "fixtureNormalizeCommand", "desktopOpenCheckCommand", "proof", "next"],
        "modelColumnSortByMutationFields": ["ok", "exitCode", "dryRun", "mode", "projectModified", "target.handle", "target.table", "target.column", "target.sortByColumn", "target.previousSortByColumn", "changes", "validation", "readbackCommand", "inspectCommand", "validateCommand"],
        "lintFindingCodes": ["m.unbuffered_reuse", "m.untyped_expansion"],
        "desktopOpenFields": ["ok", "exitCode", "document", "preflight.mode", "preflight.defaulted", "preflight.applicable", "preflight.performed", "preflight.validationPerformed", "preflight.lintPerformed", "preflight.skipped", "preflight.ok", "session.state", "session.owned", "session.desktopProcessId", "session.desktopProcessCreationTimeUtc", "session.desktopExecutablePath", "session.receiptPath", "session.cleanupCommand", "session.priorSessionCleanup", "oracle", "validation", "proof", "diagnostics", "next"],
        "desktopCloseFields": ["ok", "exitCode", "session.state", "session.alreadyClosed", "session.document", "session.documentKind", "session.documentName", "session.desktopProcessId", "session.desktopProcessCreationTimeUtc", "session.receiptPath", "session.receiptRemoved", "cleanup.attempted", "cleanup.closed", "cleanup.identityMatched", "cleanup.targeted", "cleanup.targetedProcessIds", "cleanup.remainingProcessIds", "cleanup.errors", "next"],
        "desktopOpenCheckFields": ["ok", "exitCode", "changes", "document", "oracle.available", "oracle.desktopVersion", "oracle.detection", "validation", "validation.strict", "validation.strict.lint", "proof.level", "proof.observedStage", "proof.status", "proof.passed", "proof.claimedCompatibility", "proof.requiresManualReview", "proof.requiredCompatibilityLevel", "proof.timeoutMs", "proof.timeoutScope", "proof.signals", "proof.signals.windowObserved", "proof.signals.titleMatched", "proof.signals.observedWindowTitle", "proof.signals.windowSelectionReason", "proof.signals.observation", "proof.signals.observation.exactTitleCandidateCount", "proof.signals.cleanup", "proof.signals.cleanup.targeted", "proof.unprovenSignals", "proof.compatibility", "proof.manualReview", "diagnostics", "next"],
        "desktopScreenshotFields": ["ok", "exitCode", "changes", "document", "oracle.available", "oracle.desktopVersion", "validation", "proof.level", "proof.observedStage", "proof.status", "proof.claimedCompatibility", "proof.timeoutMs", "proof.timeoutScope", "proof.signals.windowObserved", "proof.signals.titleMatched", "proof.signals.observedWindowTitle", "proof.signals.windowSelectionReason", "proof.signals.observation", "proof.signals.observation.exactTitleCandidateCount", "proof.signals.screenshotCaptured", "proof.signals.screenshotPath", "proof.signals.screenshotActivationSucceeded", "proof.signals.screenshotForegroundVerified", "proof.signals.screenshotForegroundProcessId", "proof.signals.cleanup", "proof.signals.cleanup.targeted", "screenshot.path", "screenshot.captured", "screenshot.format", "screenshot.display", "screenshot.width", "screenshot.height", "screenshot.activationSucceeded", "screenshot.foregroundVerified", "screenshot.foregroundProcessId", "screenshot.allowUnverifiedCapture", "screenshot.purpose", "screenshot.automatedCompatibilityProof", "screenshot.limitations", "diagnostics", "next"],
        "fixtureSummaryFields": ["schema", "summaryVersion", "fingerprint", "project", "counts", "counts.explicitInteractions", "counts.unsupportedInteractions", "counts.staleInteractionVisualReferences", "model.tables", "model.relationships", "report.interactionSemantics", "report.pages", "pbir.reportDefinitionVersion", "pbir.filters.counts", "pbir.filters.items", "validation", "lint", "verification"],
        "fixtureVerificationFields": ["mode", "expected", "actualWritten", "actual", "same", "differences"],
        "fixtureReportPageFields": ["ordinal", "name", "displayName", "width", "height", "displayOption", "isActive", "visuals", "interactionCount", "interactions"],
        "fixtureReportInteractionFields": ["ordinal", "interactionType", "unsupported", "staleVisualReference", "sourceName", "targetName", "source", "target"],
        "fixtureReportInteractionRefFields": ["found", "handle", "name", "title", "visualType"],
        "fixtureReportInteractionSemanticsFields": ["mode", "missingRowsMean", "supportedTypes"],
        "fixturePbirFields": ["reportDefinitionVersion", "filters"],
        "fixturePbirFilterFields": ["scope", "owner", "ordinal", "name", "filterType", "unsupported", "target", "conditionSummary", "literalCount", "desktopSafeName", "categoricalVersion", "fromCount", "whereCount", "whereUsesSourceAlias"],
        "fixtureDifferenceFields": ["path", "expected", "actual"],
        "featureCatalogFields": feature_catalog_schema_fields(),
        "reportPageFields": ["handle", "name", "displayName", "ordinal", "width", "height", "displayOption", "isActive", "visualCount", "visualHandles"],
        "reportPageMutationFields": ["dryRun", "target", "changes[].kind", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "reportBookmarkFields": ["handle", "ordinal", "name", "displayName", "schema", "schemaVersion", "path", "jsonPointer", "fingerprint", "group", "options", "state", "unsupported", "unsupportedReasons", "safety", "raw"],
        "reportBookmarkMetadataFields": ["path", "items", "groups", "orderedNames", "diagnostics"],
        "reportBookmarkSafetyFields": ["dataValueRisk", "mayContainDataValues", "literalCountInBookmarkState", "rawIncluded", "findings"],
        "reportBookmarkMutationFields": ["dryRun", "mode", "action", "target", "changes", "readbackCommand", "validateCommand"],
        "reportFilterFields": ["handle", "handleIdentity", "handleAmbiguous", "scope", "ordinal", "arrayOrigin", "name", "displayName", "filterType", "unsupported", "target", "conditionSummary", "path", "jsonPointer", "fingerprint", "owner", "page", "visual", "safety", "raw"],
        "reportFilterSafetyFields": ["dataValueRisk", "mayContainDataValues", "literalCountInFilterDefinition", "rawIncluded", "findings"],
        "reportFilterAddMutationFields": ["dryRun", "mode", "target.handle", "target.target", "target.safety", "owner", "filterPlan.beforeCount", "filterPlan.afterCount", "filterPlan.jsonPointer", "filterPlan.rawAfterIncluded", "filterPlan.after", "changes[].path", "changes[].jsonPointer", "changes[].parentJsonPointer", "changes[].before", "changes[].after", "readbackCommand", "filterReadbackCommand", "ownerReadbackCommand", "rawReviewCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "reportFilterUpdateMutationFields": ["dryRun", "mode", "target.handle", "filterPlan.before", "filterPlan.after", "filterPlan.rawIncluded", "filterPlan.changed", "changes[].path", "changes[].jsonPointer", "changes[].parentJsonPointer", "changes[].before", "changes[].after", "readbackCommand", "filterReadbackCommand", "ownerReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "reportFilterMutationFields": ["dryRun", "mode", "target.handle", "filterPlan.before", "filterPlan.after", "filterPlan.rawBeforeIncluded", "changes[].path", "changes[].jsonPointer", "changes[].parentJsonPointer", "changes[].before", "changes[].after", "readbackCommand", "ownerReadbackCommand", "rawReviewCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "reportFilterClearMutationFields": ["dryRun", "mode", "selector.kind", "selector.stableId", "confirmToken", "counts.matchedFilters", "counts.clearedFilters", "targets[].handle", "filterPlan.before", "filterPlan.after", "filterPlan.arrayEdits", "filterPlan.rawBeforeIncluded", "changes[].path", "changes[].jsonPointer", "changes[].parentJsonPointer", "changes[].before", "changes[].after", "readbackCommand", "ownerReadbackCommand", "rawReviewCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "reportSlicerFields": ["handle", "visualHandle", "name", "title", "visualType", "page", "path", "position", "bindingCount", "bindings", "target", "targets", "state", "fingerprint", "safety", "raw"],
        "reportSlicerStateFields": ["fieldCount", "queryRoles", "filterConfigFilters", "legacyFilters", "hasVisualObjects", "hasSelectionState", "hasCachedDisplayState"],
        "reportSlicerSafetyFields": ["dataValueRisk", "mayContainDataValues", "literalCountInSlicerState", "rawIncluded", "findings"],
        "reportSlicerClearMutationFields": ["dryRun", "mode", "target.handle", "confirmToken", "counts.matchedSlicers", "counts.clearedFilterEntries", "counts.filterConfigFilters", "counts.legacyFilters", "slicerPlan.beforeState", "slicerPlan.afterState", "slicerPlan.arrayEdits", "slicerPlan.rawBeforeIncluded", "changes[].path", "changes[].jsonPointer", "changes[].parentJsonPointer", "changes[].before", "changes[].after", "readbackCommand", "visualReadbackCommand", "rawReviewCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "reportInteractionFields": ["handle", "ordinal", "interactionType", "unsupported", "page", "sourceName", "targetName", "source", "target", "path", "jsonPointer", "fingerprint", "semantics", "safety", "raw"],
        "reportInteractionSourceTargetFields": ["found", "handle", "name", "title", "visualType", "path"],
        "reportInteractionSemanticsFields": ["mode", "missingRowsMean", "supportedTypes"],
        "reportInteractionMutationFields": ["dryRun", "mode", "target", "interactionPlan.before", "interactionPlan.after", "interactionPlan.existed", "interactionPlan.changed", "changes[].kind", "changes[].action", "changes[].path", "changes[].jsonPointer", "changes[].before", "changes[].after", "readbackCommand", "pageReadbackCommand", "sourceVisualReadbackCommand", "targetVisualReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "reportDesignPlanFields": ["profile", "candidates.dateColumns", "candidates.categoryColumns", "candidates.numericColumns", "candidates.measures", "opportunities[].kind", "opportunities[].command", "recommendedWorkflow"],
        "reportObjectFields": ["handle", "kind", "name", "title", "visualType", "parentHandle", "path", "jsonPointer", "safety", "raw"],
        "reportObjectTreeFields": ["ok", "projectDir", "counts", "tree.handle", "tree.kind", "tree.children", "objects[].handle", "objects[].kind", "objects[].parentHandle", "objects[].path", "next"],
        "reportObjectFindFields": ["ok", "predicates", "objects[].handle", "objects[].kind", "objects[].path", "counts.matched", "next"],
        "reportObjectCatFields": ["ok", "object.handle", "object.kind", "object.path", "raw", "rawIncluded", "next"],
        "reportObjectQueryFields": ["ok", "selector", "objects[].handle", "objects[].kind", "counts.matched", "next"],
        "reportAuditFields": ["ok", "profile", "counts.findings", "counts.bySeverity", "findings[].ruleId", "findings[].severity", "findings[].handle", "findings[].message", "recommendedActions", "unsupportedActions", "next"],
        "reportSanitizePlanFields": ["ok", "profile", "planFingerprint", "confirmToken", "actions[].kind", "actions[].handles", "actions[].applySupported", "actions[].blockedReason", "actions[].jsonPointers", "next"],
        "reportSanitizeApplyFields": ["ok", "dryRun", "mode", "planFingerprint", "actions[].kind", "actions[].handles", "changes[].path", "changes[].jsonPointer", "postAudit", "validateCommand", "readbackCommand", "next"],
        "reportLayoutAutoMutationFields": ["dryRun", "mode", "layoutPlan.pages", "layoutPlan.changedVisuals", "changes[].path", "changes[].visual", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "reportDrilldownHierarchyMutationFields": ["dryRun", "mode", "target.handle", "hierarchyPlan.fields", "hierarchyPlan.before", "hierarchyPlan.after", "changes[].jsonPointer", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "reportThemeFields": ["handle", "state", "name", "fingerprint", "reportJsonPath", "themeCollection", "registeredThemes", "safety"],
        "reportThemeBundleFields": ["schema", "bundleVersion", "sourceFingerprint", "theme", "themeCollection", "registeredThemes", "safety"],
        "reportThemePresetFields": ["presets[].id", "presets[].name", "presets[].command", "preset.id", "preset.bundle", "preset.fingerprint"],
        "visualFields": ["name", "visualType", "title", "mode", "bindings", "x", "y", "z", "width", "height", "tabOrder"],
        "visualBindingFields": ["role", "table", "column", "measure", "displayName", "formatString", "sortDirection"],
        "visualCatalogFields": ["supportedVisualTypes", "visualTypes[].visualType", "visualTypes[].aliases", "visualTypes[].proofLevel", "visualTypes[].roles", "templateOnlyVisualTypes", "plannedVisualTypes", "next"],
        "visualFormattingFields": ["rawIncluded", "formatObjectContainerCount", "formatCardCount", "formatPropertyCount", "unsupportedContainerCount", "literalValueCount", "sources", "objectNames", "containers", "safety"],
        "visualFormattingContainerFields": ["source", "objectName", "shape", "unsupportedShape", "cardCount", "propertyCount", "selectorCount", "literalValueCount", "propertyNames", "cards", "raw"],
        "visualFormattingBundleFields": ["schema", "bundleVersion", "sourceFingerprint", "source.visual", "formatting.visualObjects", "formatting.topLevelObjects", "summary", "safety"],
        "reportStyleBundleFields": ["schema", "source", "themeCollection", "visualStyles[].visualType", "visualStyles[].ordinalWithinType", "visualStyles[].formatting", "visualStyles[].safety", "policy"],
        "visualConditionalFormattingFields": ["rawIncluded", "signalCount", "signalTypes", "formatObjectNames", "signals[].pointer", "signals[].type", "safety"],
        "visualFormattingMutationFields": ["dryRun", "mode", "source.fingerprint", "target.handle", "formattingPlan.before", "formattingPlan.after", "formattingPlan.safety", "changes[].jsonPointers", "changes[].before", "changes[].after", "readbackCommand", "rawReviewCommand", "visualReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "visualFormattingTextMutationFields": ["dryRun", "mode", "target.handle", "textPlan.strategy", "textPlan.requested", "textPlan.before", "textPlan.after", "changes[].jsonPointers", "changes[].before", "changes[].after", "readbackCommand", "rawReviewCommand", "visualReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "visualFormattingColorMutationFields": ["dryRun", "mode", "target.handle", "colorPlan.strategy", "colorPlan.requested", "colorPlan.before", "colorPlan.after", "changes[].jsonPointers", "changes[].before", "changes[].after", "readbackCommand", "rawReviewCommand", "visualReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "visualMutationFields": ["dryRun", "target", "visualPlan.before", "visualPlan.after", "bindingPlan.before", "bindingPlan.after", "changes[].kind", "changes[].action", "changes[].path", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "visualCloneMutationFields": ["dryRun", "mode", "source.handle", "target.handle", "clonePlan.strategy", "clonePlan.sourcePath", "clonePlan.targetPath", "clonePlan.position.before", "clonePlan.position.after", "changes[].path", "changes[].after", "readbackCommand", "slicerReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "visualDeleteMutationFields": ["dryRun", "mode", "target.handle", "target.page.handle", "deletePlan.before", "deletePlan.after", "changes[].kind", "changes[].action", "changes[].path", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "visualBindingMutationFields": ["bindingPlan.before", "bindingPlan.after", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
        "columnDataTypes": ["string", "int64", "double", "decimal", "date", "dateTime", "boolean"],
        "samples": ["examples/sales.schema.json", "examples/archetypes/regional-sales.schema.json"]
    });
    manifest["packageImportFields"] = json!([
        "ok",
        "exitCode",
        "action",
        "package",
        "packageKind",
        "packageClass",
        "sourceRoot",
        "outDir",
        "counts.extracted",
        "counts.skipped",
        "validation",
        "next"
    ]);
    manifest["packageSourcePackFields"] = json!([
        "ok",
        "changed",
        "dryRun",
        "projectDir",
        "pbip",
        "package",
        "packageKind",
        "packageClass",
        "entries[].name",
        "entries[].category",
        "validation",
        "next"
    ]);
    manifest["reportPlanFields"] = json!([
        "ok",
        "schemaPath",
        "profilePath",
        "specPath",
        "intent.text",
        "profileSummary",
        "spec",
        "compiled.counts",
        "decisions",
        "warnings",
        "next"
    ]);
    manifest
}

fn generated_visual_contract() -> Value {
    json!({
        "summary": "Generated dashboard specs and report visuals add use this small visual role contract. Exact card, tableEx, lineChart, scatterChart, and hundredPercentStackedColumnChart visual.json goldens replicate Desktop-rendered shapes from the 2026-08 production pilot and carry schema-golden proof. Use clone/template workflows for visuals outside the catalog.",
        "supportedVisualTypes": supported_visual_type_names(),
        "visualTypes": visual_type_contracts(),
        "schemaGoldenVisualTypes": schema_golden_visual_type_names(),
        "desktopGoldenPendingVisualTypes": supported_visual_type_names().into_iter().filter(|visual_type| !schema_golden_visual_type_names().contains(visual_type)).collect::<Vec<_>>(),
        "bindingManualDesktopCanvasRefreshVisualTypes": ["pieChart", "donutChart", "pivotTable", "slicer"],
        "slicerModes": ["Basic", "Dropdown", "Between"],
        "bindingFields": ["role", "field", "table", "column", "measure", "displayName", "formatString", "sortDirection"],
        "bindingRules": [
            "Prefer structured bindings with table plus column or measure to avoid ambiguity.",
            "Legacy field strings use Table[Name] and fail when a column and measure share the same name.",
            "Category, Series (including scatter color grouping), Rows, Columns, scatter Category, and slicer Values bindings must resolve to columns.",
            "Card Values and line-chart Y require measures; table Values may resolve to columns or measures.",
            "Scatter X/Y/Size and hundredPercentStackedColumnChart Y accept measures or columns; columns emit Function 0 Aggregation expressions with queryRef Sum(Table.Column) and nativeQueryRef Summe von <Column>.",
            "Scatter detail identity uses Category; Details is rejected with a Category repair hint.",
            "One model field may appear only once per visual until Desktop-authored duplicate queryRef numbering is available.",
            "Pie and donut require exactly one Category plus one or more Y measures and emit a default descending sort by the first Y binding.",
            "Line charts require Category, optional Series, and one or more Y measures; clustered-column combo charts require Category, one or more Y column measures, and one or more Y2 line measures.",
            "Explicit sort uses sortDirection=Descending on at most one projected measure; ascending and multi-key sorts remain fixture-gated.",
            "Slicer mode is Basic by default; Basic, Dropdown, and Between are generated, singleSelect=true emits the native slicer selection property, and generated slicers contain no persisted selection filter. Between is intended for numeric or date range columns."
        ]
    })
}

fn desktop_proofed_archetypes() -> Value {
    json!([
        {
            "id": "flat-ops",
            "schema": "examples/archetypes/flat-ops.schema.json",
            "profile": "examples/archetypes/flat-ops.profile.json",
            "spec": "examples/archetypes/flat-ops.dashboard.json",
            "golden": "testdata/golden/archetypes/flat-ops.summary.json",
            "desktopProof": "testdata/desktop-proof/flat-ops.desktop-proof.json",
            "proofLevel": "desktop-golden-pending",
            "bindingProofLevel": "manual-desktop-canvas-refresh",
            "status": "title-reverification-pending",
            "note": "The recorded Desktop proof remains binding/canvas evidence; current generated bytes add a Desktop-authored title container and await open/refresh/save re-verification.",
            "visualTypes": ["card", "clusteredBarChart", "tableEx"]
        },
        {
            "id": "scatter-bubble",
            "schema": "examples/archetypes/scatter-bubble.schema.json",
            "profile": "examples/archetypes/scatter-bubble.profile.json",
            "spec": "examples/archetypes/scatter-bubble.dashboard.json",
            "golden": "testdata/golden/archetypes/scatter-bubble.summary.json",
            "desktopProof": "testdata/desktop-proof/scatter-bubble.desktop-proof.json",
            "proofLevel": "desktop-golden-pending",
            "bindingProofLevel": "manual-desktop-canvas-refresh",
            "status": "title-reverification-pending",
            "note": "The recorded Desktop proof remains binding/canvas evidence; current generated bytes add a Desktop-authored title container and await open/refresh/save re-verification.",
            "visualTypes": ["scatterChart", "tableEx"]
        },
        {
            "id": "catalog-proof",
            "schema": "examples/archetypes/catalog-proof.schema.json",
            "profile": "examples/archetypes/catalog-proof.profile.json",
            "spec": "examples/archetypes/catalog-proof.dashboard.json",
            "golden": "testdata/golden/archetypes/catalog-proof.summary.json",
            "desktopProof": "testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json",
            "proofLevel": "desktop-golden-pending",
            "bindingProofLevel": "manual-desktop-canvas-refresh",
            "status": "title-reverification-pending",
            "note": "Power BI Desktop Store 2.155.756.0 proved the binding/canvas baseline. Current generated bytes add a Desktop-authored title container and await open/refresh/save re-verification.",
            "visualTypes": ["pieChart", "donutChart", "pivotTable", "slicer", "lineChart"]
        }
    ])
}

fn format_targets() -> Value {
    json!({
        "project": {"format": "PBIP", "schema": PBIP_SCHEMA},
        "report": {"format": "PBIR enhanced report format", "schema": REPORT_DEFINITION_SCHEMA},
        "semanticModel": {"format": "TMDL", "schema": SEMANTIC_MODEL_DEFINITION_SCHEMA}
    })
}

fn proof_levels() -> Vec<Value> {
    PROOF_LEVELS
        .iter()
        .map(|(name, meaning)| json!({"name": name, "meaning": meaning}))
        .collect()
}

fn response_shapes() -> Value {
    json!({
        "success": {
            "transport": "stdout",
            "familySpecific": true,
            "commonRequiredFields": [],
            "okExitCodeRule": "Result payloads with an ok field also expose exitCode and may use ok=false with a nonzero exit on stdout; successful readers without an ok field may omit both.",
            "readers": "Reader families expose their documented records/counts fields and may omit changes.",
            "mutationResults": {
                "requiredFields": ["changes"],
                "appliesTo": "Mutation response schemas and report build",
                "dryRun": "changes describes the planned before/after state even when files are not written"
            },
            "artifactWriters": "Scaffold, normalize, profile, export, and other artifact writers keep family-specific success fields; inspect commands[].followUpFields."
        },
        "error": {
            "transport": "stderr",
            "topLevelRequiredFields": ["error"],
            "requiredFields": ["error.code", "error.exitCode", "error.message"],
            "optionalFields": ["error.hint", "error.suggestedCommands"],
            "shape": {
                "error": {
                    "code": "<diagnostic-code>",
                    "exitCode": "<integer>",
                    "message": "<text>",
                    "hint": "<optional-text>",
                    "suggestedCommands": ["<executable powerbi-cli command template>"]
                }
            }
        },
        "followUps": {
            "next": "Executable powerbi-cli command templates only.",
            "instructions": "Human or agent prose steps that are not executable commands.",
            "notes": "Explanatory context; never interpret as commands."
        }
    })
}

fn architecture_guardrails() -> Vec<&'static str> {
    vec![
        "Do not add new Power BI features to src/main.rs.",
        "Keep dispatch in cli.rs and the live agent contract in contract.rs.",
        "Put future model mutations, report inspection, Desktop oracle, PBIR, and TMDL logic in focused modules.",
        "Freeze visual binding expansion until Desktop-authored PBIR golden fixtures exist.",
    ]
}

fn design_rules() -> Vec<&'static str> {
    vec![
        "Author PBIP folder projects instead of attempting direct PBIX binary generation.",
        "Keep generated semantic models offline-safe by using dummy inline M tables until the work machine rebinds data.",
        "Do not include .pbi/cache.abf, localSettings.json, credentials, or real exported data in the home-authored project.",
        "Validate before moving the project back to the locked-down work machine.",
        "Do not create a monolithic implementation; split command contract, schema, PBIR, TMDL, project validation, and future mutation features into focused modules.",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn proof_level_vocabulary_is_ordered_and_closed_across_catalogs() {
        let names = PROOF_LEVELS
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "unit-smoke",
                "schema-golden",
                "desktop-golden-pending",
                "manual-desktop-canvas-refresh",
                "desktop-canvas-refresh",
            ]
        );
        let allowed = names.into_iter().collect::<BTreeSet<_>>();
        for (catalog, value) in [
            ("contract", capabilities(&[]).expect("capabilities")),
            (
                "feature_catalog",
                crate::feature_catalog::features_command(&["list".to_string()])
                    .expect("feature catalog"),
            ),
            (
                "visual_catalog",
                crate::visual_catalog::visual_catalog_command(&[]).expect("visual catalog"),
            ),
        ] {
            assert_proof_levels(&value, catalog, &allowed);
        }
    }

    fn assert_proof_levels(value: &Value, path: &str, allowed: &BTreeSet<&'static str>) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let child_path = format!("{path}.{key}");
                    if key == "proofLevel" {
                        let level = child
                            .as_str()
                            .unwrap_or_else(|| panic!("{child_path} must be a string"));
                        assert!(
                            allowed.contains(level),
                            "{child_path} contains out-of-vocabulary proof level {level}"
                        );
                    }
                    assert_proof_levels(child, &child_path, allowed);
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    assert_proof_levels(child, &format!("{path}[{index}]"), allowed);
                }
            }
            _ => {}
        }
    }
}
