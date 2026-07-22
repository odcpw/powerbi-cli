use crate::feature_catalog::{feature_catalog_schema_fields, feature_policy_json};
use crate::visual_catalog::{supported_visual_type_names, visual_type_contracts};
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
  powerbi-cli desktop open <project-dir-or.pbip-or.pbix> --json
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
  powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json
  powerbi-cli report visuals delete --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json
  powerbi-cli report visuals set-position --project <project-dir-or.pbip> --handle <visual-handle> --x <n> --y <n> --dry-run --json
  powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> --handle <visual-handle> --bindings-json <json> --dry-run --json
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
- Use `model measures list/show/add/update/delete` for DAX measure authoring; updates refuse unsupported Desktop-authored TMDL metadata, local validation proves file structure, and Power BI Desktop remains the DAX compatibility oracle.
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
- Use `desktop open` for one interactive CLI-owned Power BI Desktop session for a PBIP or PBIX document and always finish with idempotent `desktop close`; opening another managed session closes the prior owned session first. PBIP retains strict source validation/lint, while PBIX gets bounded native archive preflight and delegates rendering to Desktop. Use `desktop open-check` and `desktop screenshot` for one-shot evidence; they always attempt bounded identity-checked cleanup and report unresolved ownership. Launch/capture commands require an opt-in Windows oracle machine with `POWERBI_DESKTOP_ORACLE=1`; `desktop close` intentionally does not, so cleanup remains available. Default CI should treat oracle-unavailable as expected. `desktop-launch` and `desktop-window` are observation stages, not members of the closed proof-level ladder. Window/title signals and screenshots still do not prove canvas render or refresh.
- Use `report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir>` as the macro surface for generic dashboard generation; it compiles only supported spec features and returns proof/handoff follow-up commands.
- Use `report spec fields --schema <schema.json> [--profile <profile.json>]` to get exact column/measure binding references before writing a dashboard spec.
- Use `report plan --schema <schema.json> --profile <profile.json> --objective <goal> --out <dashboard.json>` to create a deterministic starter dashboard spec, then `report spec validate --schema <schema.json> --spec <dashboard.json>` before build.
- Use project-only `report design-plan --project <project>` to get visual opportunities from an already scaffolded project.
- Use `report tree/find/cat/query` for stable report-object navigation across pages, visuals, bindings, filters, slicers, bookmarks, and interactions. Use `--include-raw` only when you explicitly need raw PBIR JSON.
- Use `report audit` and `report sanitize plan/apply` before handoff when a Desktop-authored or template-derived report might contain persisted filter/slicer/bookmark state, literal values, or stale interaction references.
- Use `report pages list/show/add/update/reorder/set-active/delete-empty`, `report layout auto`, `report drilldown set-hierarchy`, `report drillthrough set/show/clear`, `report bookmarks list/show/set-display-name/reorder/delete`, `report filters list/show/add/update/delete/clear`, `report slicers list/show/clear`, `report interactions list/show/set/disable`, and `report visuals list/show/catalog/formatting list/formatting show/formatting conditional-formatting list/show/formatting extract/formatting apply/formatting set-text/formatting set-color/add/clone/delete/set-position/set-bindings` for PBIR layout navigation, deterministic visual arrangement, chart hierarchy axes, same-report drillthrough page bindings, bookmark/filter/slicer/interaction inventory and readback, guarded categorical/range/TopN/relative-date filter authoring, type-preserving filter updates, deletion and owner-scoped clear, guarded slicer selection clear, guarded interaction overrides, guarded page metadata/order edits, visual type/role discovery, safe visual formatting inventory and bundle portability, conditional-formatting readback, typed title/static-color formatting and rejected alt-text cleanup, safe visual creation/cloning/deletion, geometry edits, and field-well binding replacement.
- Use `report style inspect/extract/apply/diff` for master-style bundles that combine report themeCollection and per-visual formatting payloads. Review literal text before applying a style bundle with `--allow-literal-text`.
- Use `report themes show/extract/apply`, `report themes presets list/show`, and `report themes apply-preset` for report-level theme bundles and built-in registered-resource theme presets. Theme copy is not per-visual formatting copy.
- Run `handoff check <project>` for an offline/dummy project. For a canonical live-source PBIP going to its work network, use `handoff check <project> --target work`; recognized connectors are then accepted, while credentials, caches, binaries, embedded data, and unknown sources still fail.
- Start measure mutations with `--dry-run`; use `--in-place` or `--out-dir <dir>` only after the returned TMDL block looks right.
- Keep real data, credentials, gateway names, `.pbix`, `.pbit`, `.pbi/cache.abf`, and `localSettings.json` out of offline projects.
- Treat PBIR visual bindings as Desktop-proved only after a public Desktop oracle proof record exists; a deterministic local golden alone is not Desktop proof. The pie, donut, matrix, and slicer binding families have manual-desktop-canvas-refresh evidence in testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json, but current title-bearing generated visual bytes are desktop-golden-pending until re-verified. Same-report drillthrough currently has schema-golden proof; end-to-end Desktop interaction proof remains open.
- Bind measures, not raw columns, to card Values, chart Y, matrix Values, and scatter X/Y/Size roles. Bare-column aggregation semantics and repeated use of one field per visual are unsupported_feature until Desktop-authored fixtures prove their PBIR shapes.
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
            "desktopOpen": "powerbi-cli desktop open <project-dir-or.pbip-or.pbix> --json",
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
            "relationshipList": "powerbi-cli model relationships list --project <project-dir-or.pbip> --json",
            "relationshipAddDryRun": "powerbi-cli model relationships add --project <project-dir-or.pbip> --from-table <table> --from-column <column> --to-table <table> --to-column <column> --dry-run --json",
            "partitionList": "powerbi-cli model partitions list --project <project-dir-or.pbip> --json",
            "modelDaxBridgePlan": "powerbi-cli model dax bridge-plan --project <project-dir-or.pbip> --json",
            "modelDaxExecute": "POWERBI_DESKTOP_ORACLE=1 powerbi-cli model dax execute --project <project-dir-or.pbip-or.pbix> --query-file <query.dax> --allow-data-read --json",
            "modelLiveExportTmdl": "POWERBI_DESKTOP_ORACLE=1 powerbi-cli model live export-tmdl --document <project-dir-or.pbip-or.pbix> --out-dir <fresh-dir> --allow-model-read --json",
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
            "reportVisualCloneDryRun": "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json",
            "reportVisualDeleteDryRun": "powerbi-cli report visuals delete --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json",
            "reportVisualSetPositionDryRun": "powerbi-cli report visuals set-position --project <project-dir-or.pbip> --handle <visual-handle> --x 40 --y 40 --dry-run --json",
            "reportVisualSetBindingsDryRun": "powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> --handle <visual-handle> --bindings-json '[{\"role\":\"Values\",\"table\":\"FactSales\",\"measure\":\"Total Revenue\"}]' --dry-run --json",
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
    vec![
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
        json!({
            "path": "package inspect",
            "aliases": ["package info", "packages inspect"],
            "usage": "powerbi-cli package inspect <file.pbix|file.pbit|file.zip> --json",
            "summary": "Inspect a PBIX/PBIT ZIP-like package and classify source/metadata/cache entries without extracting opaque data caches",
            "tags": ["package", "pbix", "pbit", "inspect", "metadata", "no-fallback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.package.inspect.v1",
            "flags": ["<file.pbix|file.pbit|file.zip>", "--json", "--format json"],
            "examples": ["powerbi-cli package inspect template.pbit --json"],
            "followUpFields": ["package", "packageKind", "packageClass", "archive.entries", "sourceRoots", "support.canExtractSafeMetadata", "support.canImportSourceProject", "support.canWriteBinaryPackage", "entries[].category", "next"]
        }),
        json!({
            "path": "package extract",
            "aliases": ["package unpack"],
            "usage": "powerbi-cli package extract <file.pbix|file.pbit|file.zip> --out-dir <dir> [--include-unknown] [--max-entries <n>] [--max-entry-bytes <n>] [--max-total-bytes <n>] [--max-compression-ratio <n>] --json",
            "summary": "Extract selected source/metadata entries with streaming archive-bomb budgets and clean partial-output rollback",
            "tags": ["package", "pbix", "pbit", "extract", "metadata", "safe", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.package.extract.v1",
            "flags": ["<file.pbix|file.pbit|file.zip>", "--out-dir <dir>", "--include-unknown", "--include-unsafe", "--max-entries <n>", "--max-entry-bytes <n>", "--max-total-bytes <n>", "--max-compression-ratio <n>", "--json", "--format json"],
            "examples": ["powerbi-cli package extract template.pbit --out-dir extracted-template --json"],
            "limitations": ["Does not convert opaque PBIX internals into a PBIP project.", "Skips unsafe cache/binary/data-model entries by default.", "Defaults: 10000 entries, 256 MiB per entry, 2 GiB total uncompressed, and 200:1 compression ratio; overrides require explicit flags."],
            "followUpFields": ["package", "outDir", "policy.limits", "extracted[].name", "skipped[].skipReason", "next"]
        }),
        json!({
            "path": "package import",
            "usage": "powerbi-cli package import <file.pbix|file.pbit|file.zip> --out-dir <project-dir> [--max-entries <n>] [--max-entry-bytes <n>] [--max-total-bytes <n>] [--max-compression-ratio <n>] --json",
            "summary": "Import PBIP/PBIR/TMDL source entries only when they are actually present inside a package archive",
            "tags": ["package", "pbix", "pbit", "pbip", "import", "metadata", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.package.import.v1",
            "flags": ["<file.pbix|file.pbit|file.zip>", "--out-dir <project-dir>", "--source-root <archive-folder>", "--max-entries <n>", "--max-entry-bytes <n>", "--max-total-bytes <n>", "--max-compression-ratio <n>", "--json", "--format json"],
            "examples": ["powerbi-cli package import source-bearing-template.pbit --out-dir imported-project --json", "powerbi-cli package import report-source.zip --out-dir imported-project --json"],
            "limitations": ["Fails honestly when the package does not contain PBIP/PBIR/TMDL source files.", "Does not reverse-engineer opaque DataModel binaries."],
            "followUpFields": ["ok", "packageClass", "sourceRoot", "outDir", "validation", "next"]
        }),
        json!({
            "path": "package source-pack",
            "aliases": ["package source-package", "package source-zip"],
            "usage": "powerbi-cli package source-pack --project <project-dir-or.pbip> --out <archive.pbit|archive.pbix|archive.zip> [--force] [--dry-run] --json",
            "summary": "Write a deterministic, allowlisted source archive only after credential and PII-suspect content scans pass",
            "tags": ["package", "pbix", "pbit", "pbip", "pbir", "tmdl", "source", "handoff", "no-fallback", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.package.sourcePack.v1",
            "flags": ["--project <project-dir-or.pbip>", "--out <archive>", "--kind <pbit|pbix|zip>", "--force", "--dry-run", "--json", "--format json"],
            "examples": ["powerbi-cli package source-pack --project build/sales --out build/sales-source.pbit --json"],
            "limitations": ["Writes a ZIP-format source archive only; it does not synthesize Desktop's opaque imported-data model.", "Allows root .pbip, report PBIR/definition JSON, semantic-model PBISM/TMDL, registered/shared JSON resources, and generated .gitignore, POWERBI_HANDOFF.md, and powerbi-cli.manifest.copy.json sidecars only.", "Refuses all files in dot-directories and every unknown path before archive creation.", "Credential-like content, PII-suspect row literals, and non-dummy or unverified partition sources prevent archive creation."],
            "followUpFields": ["ok", "changed", "dryRun", "projectDir", "pbip", "package", "packageClass", "entries[].name", "validation", "next"]
        }),
        json!({
            "path": "package export-plan",
            "aliases": ["package pbit-plan", "package template-plan"],
            "usage": "powerbi-cli package export-plan --project <project-dir-or.pbip> --json",
            "summary": "Return the Desktop handoff plan for producing PBIX/PBIT because powerbi-cli does not write opaque binary package containers",
            "tags": ["package", "pbix", "pbit", "export", "plan", "desktop", "no-fallback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.package.exportPlan.v1",
            "flags": ["--project <project-dir-or.pbip>", "--json", "--format json"],
            "examples": ["powerbi-cli package export-plan --project build/sales --json"],
            "followUpFields": ["canWriteBinaryPackage", "reason", "desktopWorkflow", "next"]
        }),
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
        json!({
            "path": "workflow plan",
            "usage": "powerbi-cli workflow plan --project <project-dir-or.pbip> --profile <source-profile.json> --out <new-plan.json> --out-dir <new-project-dir> [--resource <name>=<path>] --json",
            "summary": "Create a fingerprinted deterministic plan for one selected PBIP closure and typed source-profile replacements",
            "tags": ["workflow", "source-profile", "plan", "pbip", "mcp", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "requiresOutput": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.workflow-plan.v1",
            "flags": ["--project <project-dir-or.pbip>", "--profile <source-profile.json>", "--out <new-plan.json>", "--out-dir <new-project-dir>", "--resource <name>=<path>", "--json", "--format json"],
            "examples": ["powerbi-cli workflow plan --project report.pbip --profile workflow/source-profile.json --out ../powerbi-build/report.plan.json --out-dir ../powerbi-build/report --json"],
            "limitations": ["Writes only a new plan file; it does not create the intended output directory. Plan and output paths must be outside the entire source project root, and credential-like canonical profile/template/resource/override/project/plan/output paths are refused before persistence.", "Profiles support only typed partition.replaceSource operations rooted at Excel.Workbook or PostgreSQL.Database. Resources require exact SHA-256 claims; unknown/dynamic connectors, computed/postfix invocations, and hard-coded file/URI paths are refused."],
            "followUpFields": ["profileId", "plan", "planFingerprint", "selectedFiles", "resources", "replacements", "outputDir", "next"]
        }),
        json!({
            "path": "workflow run",
            "usage": "powerbi-cli workflow run --plan <plan.json> --confirm <plan-fingerprint> --json",
            "summary": "Recheck a fingerprinted plan, build a fresh selected-artifact closure, apply exact local MCP model edits, validate, and write a checksummed receipt",
            "tags": ["workflow", "source-profile", "run", "pbip", "mcp", "official-validation", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "requiresOutput": true,
            "confirmRequired": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.workflow-receipt.v1",
            "flags": ["--plan <plan.json>", "--confirm <plan-fingerprint>", "--json", "--format json"],
            "examples": ["powerbi-cli workflow run --plan ../powerbi-build/report.plan.json --confirm sha256:<fingerprint> --json"],
            "limitations": ["Requires the exact installed modeling MCP and official report validator.", "Creates a narrow allowlisted output outside the source project; source projects are never edited and failed outputs retain an incomplete marker.", "Directory creation, create-only writes, copy readback, and marker removal are relative to the opened output-directory capability with no-follow component traversal; ambient path/FileId identity is checked separately before publication."],
            "followUpFields": ["planFingerprint", "receiptChecksum", "outputDir", "receipt", "validation", "childrenReaped", "pumpsJoined", "next"]
        }),
        json!({
            "path": "workflow verify",
            "usage": "powerbi-cli workflow verify --plan <plan.json> --json",
            "summary": "Reconstruct profile-derived plan and staged-model semantics, bind output and MCP readbacks, and rerun native/official validation without editing the workflow output",
            "tags": ["workflow", "source-profile", "verify", "integrity", "validation", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.workflow-verify.v1",
            "flags": ["--plan <plan.json>", "--json", "--format json"],
            "examples": ["powerbi-cli workflow verify --plan ../powerbi-build/report.plan.json --json"],
            "limitations": ["Requires the exact installed modeling MCP and official report validator.", "Creates a private temporary canonical MCP export to bind the complete staged model to evidence; all evidence TMDL is bounded UTF-8 and credential-scanned, and the workflow output remains read-only."],
            "followUpFields": ["planFingerprint", "receiptChecksum", "outputTreeSha256", "validation", "sourceInputsUnchanged", "receiptClaimsValid", "evidenceClaimsValid"]
        }),
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
        json!({
            "path": "desktop open",
            "usage": "powerbi-cli desktop open <project-dir-or.pbip-or.pbix> [--timeout-ms <ms>] [--desktop-path <PBIDesktop.exe>] --json",
            "summary": "Open the single CLI-owned interactive Power BI Desktop session, closing a prior owned session first",
            "tags": ["desktop", "session", "lifecycle", "window", "windows", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "stability": "alpha-oracle",
            "proofLevel": "unit-smoke",
            "observedStage": "desktop-window",
            "outputSchema": "powerbi-cli.desktop.open.v1",
            "platforms": ["windows session when POWERBI_DESKTOP_ORACLE=1", "linux unsupported_feature", "macos unsupported_feature"],
            "sessionContract": "exactly one CLI-owned session; ownership is persisted outside the project with the exact observed PID and creation time; opening another session first closes only that recorded process lineage",
            "flags": ["<project-dir-or.pbip-or.pbix>", "--project <project-dir-or.pbip-or.pbix>", "--timeout-ms <ms>", "--desktop-path <PBIDesktop.exe>", "--json", "--format json"],
            "examples": ["POWERBI_DESKTOP_ORACLE=1 powerbi-cli desktop open build/sales --json", "POWERBI_DESKTOP_ORACLE=1 powerbi-cli desktop open SourceProfile.pbix --json"],
            "followUpFields": ["ok", "exitCode", "document.kind", "document.path", "session.state", "session.owned", "session.desktopProcessId", "session.desktopProcessCreationTimeUtc", "session.receiptPath", "session.cleanupCommand", "session.priorSessionCleanup", "proof", "validation", "diagnostics", "next"]
        }),
        json!({
            "path": "desktop close",
            "usage": "powerbi-cli desktop close --json",
            "summary": "Idempotently close only the exact CLI-owned Power BI Desktop session and its verified descendants",
            "tags": ["desktop", "session", "lifecycle", "cleanup", "windows", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "stability": "alpha-oracle",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.desktop.close.v1",
            "platforms": ["windows", "linux unsupported_feature", "macos unsupported_feature"],
            "sessionContract": "idempotent; a missing, exited, or PID-reused receipt returns alreadyClosed without killing anything; failed verified cleanup retains the receipt for retry",
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli desktop close --json"],
            "followUpFields": ["ok", "exitCode", "session.state", "session.alreadyClosed", "session.document", "session.documentKind", "session.desktopProcessId", "session.receiptPath", "session.receiptRemoved", "cleanup.attempted", "cleanup.identityMatched", "cleanup.closed", "cleanup.targeted", "cleanup.remainingProcessIds", "cleanup.errors", "next"]
        }),
        json!({
            "path": "desktop open-check",
            "usage": "powerbi-cli desktop open-check <project-dir-or.pbip-or.pbix> [--timeout-ms <ms>] [--desktop-path <PBIDesktop.exe>] --json",
            "summary": "Attempt one-shot Power BI Desktop launch plus exact project-title observation, then clean up",
            "tags": ["desktop", "oracle", "proof", "window", "title", "windows", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-oracle",
            "proofLevel": "unit-smoke",
            "observedStage": "desktop-window",
            "outputSchema": "powerbi-cli.desktop.openCheck.v1",
            "platforms": ["windows observation when POWERBI_DESKTOP_ORACLE=1", "linux unsupported_feature", "macos unsupported_feature"],
            "ciPolicy": "never required in default CI; run only on Windows with POWERBI_DESKTOP_ORACLE=1 and Desktop installed; proof.observedStage reports launch/exact-title observations and is not a canvas/render/refresh compatibility claim",
            "timeoutContract": "timeout-ms is one watchdog budget for the bounded version probe, process baseline, file-association launch, and exact window/title polling; a timeout after confirmed launch returns exit 0 with observedStage=desktop-launch and timeout signals, while a launch timeout is oracle_failed",
            "flags": ["<project-dir-or.pbip-or.pbix>", "--project <project-dir-or.pbip-or.pbix>", "--timeout-ms <ms>", "--desktop-path <PBIDesktop.exe>", "--json", "--format json"],
            "examples": ["powerbi-cli desktop open-check build/sales --json", "POWERBI_DESKTOP_ORACLE=1 powerbi-cli desktop open-check build/sales --desktop-path \"C:\\\\Program Files\\\\Microsoft Power BI Desktop\\\\bin\\\\PBIDesktop.exe\" --json"],
            "followUpFields": ["ok", "exitCode", "changes", "oracle.available", "oracle.desktopVersion", "oracle.detection.requestedDesktopPath", "proof.level", "proof.observedStage", "proof.status", "proof.passed", "proof.signals.launchMethod", "proof.signals.detectionPathUsedForLaunch", "proof.signals.windowObserved", "proof.signals.titleMatched", "proof.signals.observedWindowTitle", "proof.signals.windowSelectionReason", "proof.signals.observation", "proof.signals.observation.exactTitleCandidateCount", "proof.signals.cleanup", "proof.signals.cleanup.targeted", "proof.claimedCompatibility", "proof.requiredCompatibilityLevel", "proof.unprovenSignals", "proof.manualReview", "validation", "diagnostics", "next"]
        }),
        json!({
            "path": "desktop screenshot",
            "usage": "powerbi-cli desktop screenshot <project-dir-or.pbip-or.pbix> --out <file.png> [--timeout-ms <ms>] [--desktop-path <PBIDesktop.exe>] [--allow-unverified-capture] --json",
            "summary": "Capture the primary display after exact Desktop title and foreground-PID verification for manual or agent review",
            "tags": ["desktop", "oracle", "proof", "window", "title", "screenshot", "evidence", "windows", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "writesEvidenceArtifact": true,
            "stability": "alpha-oracle",
            "proofLevel": "unit-smoke",
            "observedStage": "desktop-window",
            "outputSchema": "powerbi-cli.desktop.screenshot.v1",
            "platforms": ["windows evidence capture when POWERBI_DESKTOP_ORACLE=1", "linux unsupported_feature", "macos unsupported_feature"],
            "ciPolicy": "never required in default CI; screenshots are evidence for manual/screen-agent review and never an automated canvas/render/refresh compatibility claim",
            "timeoutContract": "timeout-ms is one watchdog budget for the bounded version probe, process baseline, file-association launch, and exact window/title polling; if launch succeeds but no exact project title appears, screenshot returns proof_incomplete exit 20 rather than oracle_failed",
            "outputPathPolicy": "--out must end in .png and resolve outside the PBIP project directory; capture uses a unique same-directory temporary file and creates/replaces the destination only after success",
            "captureSafety": "foreground PID must be the selected PBIDesktop process or one of its verified descendants; failure publishes no PNG. --allow-unverified-capture explicitly accepts the risk of capturing unrelated sensitive screen content",
            "flags": ["<project-dir-or.pbip-or.pbix>", "--project <project-dir-or.pbip-or.pbix>", "--out <file.png>", "--timeout-ms <ms>", "--desktop-path <PBIDesktop.exe>", "--allow-unverified-capture", "--json", "--format json"],
            "examples": ["powerbi-cli desktop screenshot build/sales --out proof/sales.png --json"],
            "followUpFields": ["ok", "exitCode", "changes", "oracle.available", "oracle.desktopVersion", "proof.level", "proof.observedStage", "proof.status", "proof.claimedCompatibility", "proof.signals.windowObserved", "proof.signals.titleMatched", "proof.signals.observedWindowTitle", "proof.signals.windowSelectionReason", "proof.signals.observation", "proof.signals.observation.exactTitleCandidateCount", "proof.signals.screenshotCaptured", "proof.signals.screenshotPath", "proof.signals.screenshotActivationSucceeded", "proof.signals.screenshotForegroundVerified", "proof.signals.screenshotForegroundProcessId", "proof.signals.cleanup", "proof.signals.cleanup.targeted", "screenshot.path", "screenshot.captured", "screenshot.activationSucceeded", "screenshot.foregroundVerified", "screenshot.foregroundProcessId", "screenshot.allowUnverifiedCapture", "screenshot.purpose", "screenshot.automatedCompatibilityProof", "diagnostics", "next"]
        }),
        json!({
            "path": "desktop bridge status",
            "usage": "powerbi-cli desktop bridge status [--pid <pid>] --json",
            "summary": "Inspect pinned Microsoft Desktop Bridge instances and their exact current-file, dirty-state, and PBIR page inventory",
            "tags": ["desktop", "bridge", "microsoft", "preview", "status", "windows", "agent"],
            "readOnly": true,
            "mutates": false,
            "networkRequired": false,
            "stability": "preview-output",
            "proofLevel": "unit-smoke",
            "observedStage": "state-inventory",
            "outputSchema": "powerbi-cli.desktop.bridge.status.v1",
            "platforms": ["windows", "linux unsupported_feature", "macos unsupported_feature"],
            "flags": ["--pid <pid>", "--json", "--format json"],
            "examples": ["powerbi-cli desktop bridge status --json", "powerbi-cli desktop bridge status --pid 1234 --json"],
            "limitations": ["Bridge state and page inventory do not prove report refresh, rendered correctness, save/reopen, interactions, drill behavior, issue-dialog absence, or semantic correctness."],
            "followUpFields": ["ok", "exitCode", "ready", "status", "instances[].pid", "instances[].bridgeStatus", "instances[].currentFilePath", "instances[].hasUnsavedChanges", "instances[].pages", "backend.version", "backend.stdoutSha256", "proof", "next"]
        }),
        json!({
            "path": "desktop bridge reload",
            "usage": "powerbi-cli desktop bridge reload --project <project-dir-or.pbip> --pid <pid> --json",
            "summary": "Reload the report definition only after exact canonical project/PID identity and a clean saved Desktop state are proven",
            "tags": ["desktop", "bridge", "microsoft", "preview", "reload", "windows", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "networkRequired": false,
            "stability": "preview-output",
            "proofLevel": "unit-smoke",
            "observedStage": "reload-request-completed",
            "outputSchema": "powerbi-cli.desktop.bridge.reload.v1",
            "platforms": ["windows", "linux unsupported_feature", "macos unsupported_feature"],
            "flags": ["--project <project-dir-or.pbip>", "--pid <pid>", "--json", "--format json"],
            "examples": ["powerbi-cli desktop bridge reload --project build/sales --pid 1234 --json"],
            "limitations": ["Reload uses reloadModelDefinition=false and is refused for dirty or non-exact instances. Completion does not prove refresh, save/reopen, rendered correctness, interactions, drill behavior, issue-dialog absence, or semantic correctness."],
            "followUpFields": ["ok", "exitCode", "project", "pid", "hasUnsavedChanges", "ownership.owned", "ownership.cleanupEligible", "desktop.desktopVersion", "backend.version", "changes", "proof", "next"]
        }),
        json!({
            "path": "desktop bridge screenshot-page",
            "usage": "powerbi-cli desktop bridge screenshot-page --project <project-dir-or.pbip> --pid <pid> --page <id> --out <new.png> --json",
            "summary": "Capture one exact inventoried PBIR page through the pinned Desktop Bridge into a new guarded PNG evidence file",
            "tags": ["desktop", "bridge", "microsoft", "preview", "screenshot", "evidence", "windows", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "writesEvidenceArtifact": true,
            "networkRequired": false,
            "stability": "preview-output",
            "proofLevel": "unit-smoke",
            "observedStage": "page-screenshot-captured",
            "outputSchema": "powerbi-cli.desktop.bridge.screenshotPage.v1",
            "platforms": ["windows", "linux unsupported_feature", "macos unsupported_feature"],
            "outputPathPolicy": "--out must be a new .png outside the PBIP project; existing evidence is never replaced or deleted",
            "flags": ["--project <project-dir-or.pbip>", "--pid <pid>", "--page <id>", "--out <new.png>", "--json", "--format json"],
            "examples": ["powerbi-cli desktop bridge screenshot-page --project build/sales --pid 1234 --page ReportSection --out proof/page.png --json"],
            "limitations": ["A dirty-instance capture is labeled diagnostic and cannot represent on-disk workflow output. PNG capture does not prove refresh, save/reopen, interactions, drill behavior, issue-dialog absence, or semantic correctness."],
            "followUpFields": ["ok", "exitCode", "project", "pid", "page", "hasUnsavedChanges", "screenshot.path", "screenshot.width", "screenshot.height", "screenshot.bytes", "screenshot.sha256", "ownership", "desktop.desktopVersion", "backend", "proof", "next"]
        }),
        json!({
            "path": "desktop bridge screenshot-all",
            "usage": "powerbi-cli desktop bridge screenshot-all --project <project-dir-or.pbip> --pid <pid> --out-dir <new-dir> --json",
            "summary": "Capture the exact bounded Desktop status page inventory through the pinned Bridge into a new guarded directory",
            "tags": ["desktop", "bridge", "microsoft", "preview", "screenshot", "pages", "evidence", "windows", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "writesEvidenceArtifact": true,
            "networkRequired": false,
            "stability": "preview-output",
            "proofLevel": "unit-smoke",
            "observedStage": "all-page-screenshots-captured",
            "outputSchema": "powerbi-cli.desktop.bridge.screenshotAll.v1",
            "platforms": ["windows", "linux unsupported_feature", "macos unsupported_feature"],
            "outputPathPolicy": "--out-dir must not exist and must be outside the PBIP project; existing evidence is never replaced or deleted",
            "flags": ["--project <project-dir-or.pbip>", "--pid <pid>", "--out-dir <new-dir>", "--json", "--format json"],
            "examples": ["powerbi-cli desktop bridge screenshot-all --project build/sales --pid 1234 --out-dir proof/pages --json"],
            "limitations": ["Capture inventory must equal the bounded status inventory. Dirty-instance captures are diagnostic and cannot represent on-disk workflow output. Screenshots do not prove refresh, save/reopen, interactions, drill behavior, issue-dialog absence, or semantic correctness."],
            "followUpFields": ["ok", "exitCode", "project", "pid", "hasUnsavedChanges", "pageInventory", "screenshots[].pageId", "screenshots[].file.path", "screenshots[].file.width", "screenshots[].file.height", "screenshots[].file.sha256", "outputDirectory", "ownership", "desktop.desktopVersion", "backend", "proof", "next"]
        }),
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
            "summary": "Run typed PBIP/PBIR/TMDL quality checks and return structured findings",
            "tags": ["pbip", "pbir", "tmdl", "validation", "lint", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "lintResult.v1",
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli lint build/sales --json"],
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
        json!({
            "path": "model tables add-static",
            "aliases": ["model tables add-selector"],
            "usage": "powerbi-cli model tables add-static --project <project-dir-or.pbip> --table <table> ((--column <column> --values-json <json-array-of-strings>) | (--columns-json <json-array-of-column-names> --rows-json <json-array-of-string-arrays>)) [--include-raw] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Add a small static selector table or multi-column lookup dimension backed by an inline M table",
            "tags": ["tmdl", "semantic-model", "table", "static-table", "selector", "parameter", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.tables.staticMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--table <table>", "--column <column>", "--values-json <json-array-of-strings>", "--columns-json <json-array-of-column-names>", "--rows-json <json-array-of-string-arrays>", "--include-raw", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli model tables add-static --project build/sales --table Metric --column Metric --values-json '[\"Count\",\"Cost\"]' --dry-run --json", "powerbi-cli model tables add-static --project build/sales --table DimSegment --columns-json '[\"Code\",\"Label\"]' --rows-json '[[\"A\",\"Alpha\"],[\"B\",\"Beta\"]]' --dry-run --json"],
            "limitations": ["Creates only a new table with 1-10 string columns and 1-100 short rows; the first column must be unique.", "Does not create relationships automatically; add the reviewed relationship separately with model relationships add.", "Refuses replacement, credentials, multiline cells, duplicate rows/keys, and arbitrary fact-table ingestion."],
            "followUpFields": ["dryRun", "projectModified", "target.handle", "target.columns", "tablePlan.columnCount", "tablePlan.rowCount", "changes[].after", "readbackCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "model calculated-columns list",
            "usage": "powerbi-cli model calculated-columns list --project <project-dir-or.pbip> [--table <table>] --json",
            "summary": "List semantic model DAX calculated columns with stable column handles",
            "tags": ["tmdl", "semantic-model", "calculated-column", "column", "dax", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelCalculatedColumnsList.v1",
            "flags": ["--project <project-dir-or.pbip>", "--table <table>", "--json", "--format json"],
            "examples": ["powerbi-cli model calculated-columns list --project build/sales --json", "powerbi-cli model calculated-columns list --project build/sales --table FactSales --json"],
            "followUpFields": ["calculatedColumns[].handle", "calculatedColumns[].expression", "next"]
        }),
        json!({
            "path": "model calculated-columns show",
            "usage": "powerbi-cli model calculated-columns show --project <project-dir-or.pbip> (--handle <column-handle> | --table <table> --name <column>) --json",
            "summary": "Show one semantic model DAX calculated column and its TMDL block",
            "tags": ["tmdl", "semantic-model", "calculated-column", "column", "dax", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelCalculatedColumnsShow.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <column-handle>", "--table <table>", "--name <column>", "--json", "--format json"],
            "examples": ["powerbi-cli model calculated-columns show --project build/sales --handle 'column:FactSales:Revenue Band' --json"],
            "followUpFields": ["calculatedColumn.handle", "calculatedColumn.expression", "block", "next"]
        }),
        json!({
            "path": "model calculated-columns add",
            "usage": "powerbi-cli model calculated-columns add --project <project-dir-or.pbip> --table <table> --name <column> (--expression <dax> | --expression-file <path|->) --data-type <type> [--format-string <fmt>] [--summarize-by <mode>] [--display-folder <folder>] [--description <text>] [--hidden] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Add a DAX calculated column to a TMDL table with guarded output semantics",
            "tags": ["tmdl", "semantic-model", "calculated-column", "column", "dax", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelCalculatedColumnsMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--table <table>", "--name <column>", "--expression <dax>", "--expression-file <path|->", "--data-type <type>", "--format-string <fmt>", "--summarize-by <mode>", "--display-folder <folder>", "--description <text>", "--hidden", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli model calculated-columns add --project build/sales --table FactSales --name 'Revenue Band' --expression 'IF(''FactSales''[Revenue] >= 10000, \"\"High\"\", \"\"Standard\"\")' --data-type string --dry-run --json"],
            "followUpFields": ["dryRun", "projectModified", "rollback.performed", "changes[].before", "changes[].after", "readbackCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "model calculated-columns update",
            "usage": "powerbi-cli model calculated-columns update --project <project-dir-or.pbip> (--handle <column-handle> | --table <table> --name <column>) [--expression <dax> | --expression-file <path|->] [--data-type <type>] [--format-string <fmt>] [--summarize-by <mode>] [--display-folder <folder>] [--description <text>] [--hidden|--visible] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Update a DAX calculated column expression or metadata; refuses unsupported Desktop-authored TMDL lines",
            "tags": ["tmdl", "semantic-model", "calculated-column", "column", "dax", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelCalculatedColumnsMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <column-handle>", "--table <table>", "--name <column>", "--expression <dax>", "--expression-file <path|->", "--data-type <type>", "--format-string <fmt>", "--summarize-by <mode>", "--display-folder <folder>", "--description <text>", "--hidden", "--visible", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli model calculated-columns update --project build/sales --handle 'column:FactSales:Revenue Band' --expression 'IF(''FactSales''[Revenue] >= 5000, \"\"High\"\", \"\"Standard\"\")' --dry-run --json"],
            "followUpFields": ["dryRun", "projectModified", "rollback.performed", "changes[].before", "changes[].after", "readbackCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "model calculated-columns delete",
            "usage": "powerbi-cli model calculated-columns delete --project <project-dir-or.pbip> (--handle <column-handle> | --table <table> --name <column>) (--dry-run | --in-place --confirm <column-handle> | --out-dir <dir>) --json",
            "summary": "Delete a DAX calculated column; in-place delete requires exact handle confirmation",
            "tags": ["tmdl", "semantic-model", "calculated-column", "column", "dax", "mutation", "delete", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelCalculatedColumnsMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <column-handle>", "--table <table>", "--name <column>", "--dry-run", "--in-place", "--confirm <column-handle>", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli model calculated-columns delete --project build/sales --handle 'column:FactSales:Revenue Band' --dry-run --json"],
            "followUpFields": ["dryRun", "projectModified", "rollback.performed", "changes[].before", "changes[].after", "readbackCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "model measures list",
            "usage": "powerbi-cli model measures list --project <project-dir-or.pbip> [--table <table>] --json",
            "summary": "List semantic model DAX measures with stable handles",
            "tags": ["tmdl", "semantic-model", "measure", "dax", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelMeasuresList.v1",
            "flags": ["--project <project-dir-or.pbip>", "--table <table>", "--json", "--format json"],
            "examples": ["powerbi-cli model measures list --project build/sales --json", "powerbi-cli model measures list --project build/sales --table FactSales --json"],
            "followUpFields": ["measures[].handle", "measures[].expression", "next"]
        }),
        json!({
            "path": "model measures show",
            "usage": "powerbi-cli model measures show --project <project-dir-or.pbip> (--handle <measure-handle> | --table <table> --name <measure>) --json",
            "summary": "Show one semantic model DAX measure and its TMDL block",
            "tags": ["tmdl", "semantic-model", "measure", "dax", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelMeasuresShow.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <measure-handle>", "--table <table>", "--name <measure>", "--json", "--format json"],
            "examples": ["powerbi-cli model measures show --project build/sales --handle 'measure:FactSales:Total Revenue' --json"],
            "followUpFields": ["measure.handle", "measure.expression", "block", "next"]
        }),
        json!({
            "path": "model measures add",
            "usage": "powerbi-cli model measures add --project <project-dir-or.pbip> --table <table> --name <measure> (--expression <dax> | --expression-file <path|->) [--format-string <fmt>] [--display-folder <folder>] [--description <text>] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Add a DAX measure to a TMDL table with guarded output semantics",
            "tags": ["tmdl", "semantic-model", "measure", "dax", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelMeasuresMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--table <table>", "--name <measure>", "--expression <dax>", "--expression-file <path|->", "--format-string <fmt>", "--display-folder <folder>", "--description <text>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli model measures add --project build/sales --table FactSales --name 'Average Revenue' --expression 'DIVIDE([Total Revenue], [Total Units])' --dry-run --json"],
            "followUpFields": ["dryRun", "projectModified", "rollback.performed", "changes[].before", "changes[].after", "readbackCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "model measures update",
            "usage": "powerbi-cli model measures update --project <project-dir-or.pbip> (--handle <measure-handle> | --table <table> --name <measure>) [--expression <dax> | --expression-file <path|->] [--format-string <fmt>] [--display-folder <folder>] [--description <text>] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Update a DAX measure expression or metadata; refuses unsupported Desktop-authored TMDL lines",
            "tags": ["tmdl", "semantic-model", "measure", "dax", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelMeasuresMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <measure-handle>", "--table <table>", "--name <measure>", "--expression <dax>", "--expression-file <path|->", "--format-string <fmt>", "--display-folder <folder>", "--description <text>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli model measures update --project build/sales --handle 'measure:FactSales:Total Revenue' --expression 'SUM(''FactSales''[Revenue])' --dry-run --json"],
            "followUpFields": ["dryRun", "projectModified", "rollback.performed", "changes[].before", "changes[].after", "readbackCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "model measures delete",
            "usage": "powerbi-cli model measures delete --project <project-dir-or.pbip> (--handle <measure-handle> | --table <table> --name <measure>) (--dry-run | --in-place --confirm <measure-handle> | --out-dir <dir>) --json",
            "summary": "Delete a DAX measure; in-place delete requires exact handle confirmation",
            "tags": ["tmdl", "semantic-model", "measure", "dax", "mutation", "delete", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelMeasuresMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <measure-handle>", "--table <table>", "--name <measure>", "--dry-run", "--in-place", "--confirm <measure-handle>", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli model measures delete --project build/sales --handle 'measure:FactSales:Average Revenue' --dry-run --json"],
            "followUpFields": ["dryRun", "projectModified", "rollback.performed", "changes[].before", "changes[].after", "readbackCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "model relationships list",
            "usage": "powerbi-cli model relationships list --project <project-dir-or.pbip> [--table <table>] --json",
            "summary": "List semantic model relationships with stable relationship handles and endpoint column handles",
            "tags": ["tmdl", "semantic-model", "relationship", "model", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelRelationshipsList.v1",
            "flags": ["--project <project-dir-or.pbip>", "--table <table>", "--json", "--format json"],
            "examples": ["powerbi-cli model relationships list --project build/sales --json", "powerbi-cli model relationships list --project build/sales --table FactSales --json"],
            "followUpFields": ["relationships[].handle", "relationships[].from.columnHandle", "relationships[].to.columnHandle", "next"]
        }),
        json!({
            "path": "model relationships show",
            "usage": "powerbi-cli model relationships show --project <project-dir-or.pbip> (--handle <relationship-handle> | --name <relationship-name>) --json",
            "summary": "Show one semantic model relationship, endpoints, properties, and its TMDL block",
            "tags": ["tmdl", "semantic-model", "relationship", "model", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelRelationshipsShow.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <relationship-handle>", "--name <relationship-name>", "--json", "--format json"],
            "examples": ["powerbi-cli model relationships show --project build/sales --handle <relationship-handle> --json"],
            "followUpFields": ["relationship.handle", "relationship.from", "relationship.to", "relationship.properties", "block", "next"]
        }),
        json!({
            "path": "model relationships add",
            "usage": "powerbi-cli model relationships add --project <project-dir-or.pbip> --from-table <table> --from-column <column> --to-table <table> --to-column <column> [--name <relationship-name>] [--cross-filtering-behavior <oneDirection|bothDirections|automatic>] [--inactive] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Add a semantic model relationship with explicit endpoints and guarded output semantics",
            "tags": ["tmdl", "semantic-model", "relationship", "model", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelRelationshipsMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--from-table <table>", "--from-column <column>", "--to-table <table>", "--to-column <column>", "--name <relationship-name>", "--cross-filtering-behavior <mode>", "--cross-filter <mode>", "--active", "--inactive", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli model relationships add --project build/sales --from-table FactSales --from-column DateKey --to-table DimDate --to-column DateKey --cross-filtering-behavior oneDirection --dry-run --json"],
            "followUpFields": ["dryRun", "projectModified", "rollback.performed", "changes[].before", "changes[].after", "readbackCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "model relationships update",
            "usage": "powerbi-cli model relationships update --project <project-dir-or.pbip> (--handle <relationship-handle> | --name <relationship-name>) [--cross-filtering-behavior <oneDirection|bothDirections|automatic>] [--active|--inactive] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Update relationship active state or cross-filtering behavior; endpoint rewiring is delete+add",
            "tags": ["tmdl", "semantic-model", "relationship", "model", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelRelationshipsMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <relationship-handle>", "--name <relationship-name>", "--cross-filtering-behavior <mode>", "--cross-filter <mode>", "--active", "--inactive", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli model relationships update --project build/sales --handle <relationship-handle> --cross-filtering-behavior bothDirections --dry-run --json"],
            "followUpFields": ["dryRun", "projectModified", "rollback.performed", "changes[].before", "changes[].after", "readbackCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "model relationships delete",
            "usage": "powerbi-cli model relationships delete --project <project-dir-or.pbip> (--handle <relationship-handle> | --name <relationship-name>) (--dry-run | --in-place --confirm <relationship-handle> | --out-dir <dir>) --json",
            "summary": "Delete a semantic model relationship; in-place delete requires exact handle confirmation",
            "tags": ["tmdl", "semantic-model", "relationship", "model", "mutation", "delete", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelRelationshipsMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <relationship-handle>", "--name <relationship-name>", "--dry-run", "--in-place", "--confirm <relationship-handle>", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli model relationships delete --project build/sales --handle <relationship-handle> --dry-run --json"],
            "followUpFields": ["dryRun", "projectModified", "rollback.performed", "changes[].before", "changes[].after", "readbackCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "model partitions list",
            "usage": "powerbi-cli model partitions list --project <project-dir-or.pbip> [--table <table>] --json",
            "summary": "List semantic model partitions with source kind and offline safety classification",
            "tags": ["tmdl", "semantic-model", "partition", "model", "handoff", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelPartitionsList.v1",
            "flags": ["--project <project-dir-or.pbip>", "--table <table>", "--json", "--format json"],
            "examples": ["powerbi-cli model partitions list --project build/sales --json", "powerbi-cli model partition list --project build/sales --table FactSales --json"],
            "followUpFields": ["partitions[].handle", "partitions[].sourceKind", "partitions[].offlineSafety", "next"]
        }),
        json!({
            "path": "model partitions show",
            "usage": "powerbi-cli model partitions show --project <project-dir-or.pbip> (--handle <partition-handle> | --table <table> --name <partition-name>) [--include-source] --json",
            "summary": "Show one semantic model partition with a redacted preview by default; raw source/block output requires --include-source and is refused unless safety is safe",
            "tags": ["tmdl", "semantic-model", "partition", "model", "handoff", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "modelPartitionsShow.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <partition-handle>", "--table <table>", "--name <partition-name>", "--partition <partition-name>", "--include-source", "--json", "--format json"],
            "examples": ["powerbi-cli model partitions show --project build/sales --handle partition:FactSales:FactSales --json"],
            "followUpFields": ["partition.handle", "partition.sourcePreview", "partition.source", "partition.sourceIncluded", "partition.offlineSafety", "block", "next"]
        }),
        json!({
            "path": "model dax bridge-plan",
            "aliases": ["model dax plan", "model dax validate-plan"],
            "usage": "powerbi-cli model dax bridge-plan --project <project-dir-or.pbip> [--engine desktop|xmla|tabular-editor] --json",
            "summary": "Inventory DAX measures/calculated columns and return the external validation bridge boundary without fake local DAX compatibility claims",
            "tags": ["tmdl", "semantic-model", "dax", "measure", "calculated-column", "validation", "bridge", "oracle", "agent", "no-fallback"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.dax.bridgePlan.v1",
            "flags": ["--project <project-dir-or.pbip>", "--engine desktop|xmla|tabular-editor", "--json", "--format json"],
            "examples": ["powerbi-cli model dax bridge-plan --project build/sales --json", "powerbi-cli model dax bridge-plan --project build/sales --engine desktop --json"],
            "followUpFields": ["ok", "projectDir", "counts.measures", "counts.calculatedColumns", "daxInventory.measures[].handle", "daxInventory.calculatedColumns[].handle", "bridge.required", "bridge.supportedEngines", "bridge.noFakeFallbacks", "validationBridge.offlineDaxParser.available", "next"]
        }),
        json!({
            "path": "model dax dependencies",
            "aliases": ["model dax references", "model dax refs"],
            "usage": "powerbi-cli model dax dependencies --project <project-dir-or.pbip> --json",
            "summary": "Extract static DAX table/column and measure references for dependency graphing without claiming DAX-engine validation",
            "tags": ["tmdl", "semantic-model", "dax", "dependencies", "references", "lint", "agent", "no-fallback"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.dax.dependencies.v1",
            "flags": ["--project <project-dir-or.pbip>", "--json", "--format json"],
            "examples": ["powerbi-cli model dax dependencies --project build/sales --json"],
            "limitations": ["Static reference extraction only; not a full DAX parser.", "Use model dax bridge-plan for Desktop/XMLA/Tabular Editor validation boundaries."],
            "followUpFields": ["analysisBoundary.daxEngineValidated", "counts", "expressions[].tableColumns", "expressions[].measureReferences", "graph.edges", "findings", "next"]
        }),
        json!({
            "path": "model dax lint",
            "aliases": ["model dax check"],
            "usage": "powerbi-cli model dax lint --project <project-dir-or.pbip> --json",
            "summary": "Run static DAX reference lint for missing columns/measures, ambiguous measure names, self references, and measure dependency cycles",
            "tags": ["tmdl", "semantic-model", "dax", "lint", "bpa", "agent", "no-fallback"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.dax.lint.v1",
            "flags": ["--project <project-dir-or.pbip>", "--json", "--format json"],
            "examples": ["powerbi-cli model dax lint --project build/sales --json"],
            "limitations": ["Static reference lint only; Power BI Desktop remains the compatibility oracle for DAX syntax and semantics."],
            "followUpFields": ["ok", "analysisBoundary", "counts.errors", "counts.warnings", "findings[].code", "validation", "next"]
        }),
        json!({
            "path": "model dax execute",
            "aliases": ["model dax query"],
            "usage": "POWERBI_DESKTOP_ORACLE=1 powerbi-cli model dax execute --project <project-dir-or.pbip-or.pbix> (--query <dax> | --query-file <path|->) --allow-data-read [--max-rows <1..100000>] [--max-cell-chars <1..1000000>] [--timeout-ms <1000..300000>] --json",
            "summary": "Execute a bounded read-only DAX EVALUATE query against the exact already-open Power BI Desktop semantic model",
            "tags": ["tmdl", "semantic-model", "dax", "query", "desktop", "oracle", "read-only", "data-read", "agent", "no-fallback"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "returnsModelData": true,
            "explicitOptIn": ["POWERBI_DESKTOP_ORACLE=1", "--allow-data-read"],
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.dax.execute.v2",
            "timeoutContract": "timeout-ms is one end-to-end watchdog for exact Desktop/model discovery, in-bridge PID/creation/workspace/port revalidation, connection, and bounded query execution",
            "flags": ["--project <project-dir-or.pbip-or.pbix>", "--query <dax>", "--query-file <path|->", "--allow-data-read", "--max-rows <1..100000>", "--max-cell-chars <1..1000000>", "--timeout-ms <1000..300000>", "--json", "--format json"],
            "examples": ["POWERBI_DESKTOP_ORACLE=1 powerbi-cli model dax execute --project build/sales --query-file checks/total-revenue.dax --allow-data-read --json", "POWERBI_DESKTOP_ORACLE=1 powerbi-cli model dax query --project build/sales --query \"EVALUATE ROW(\\\"Value\\\", 1)\" --allow-data-read --max-rows 10 --json"],
            "limitations": ["Windows and an already-open exact PBIP or PBIX document match are required; the command never launches Desktop.", "Only EVALUATE or DEFINE ... EVALUATE query forms are accepted; XMLA/model mutations are refused.", "Returned rows can contain sensitive model data and are bounded but not redacted.", "The local Desktop Analysis Services endpoint and bundled ADOMD client are implementation details and may change with Desktop."],
            "followUpFields": ["ok", "exitCode", "document.kind", "document.path", "query.fingerprint", "query.textReturned", "safety", "limits", "stage", "engine.desktopProcessId", "engine.modelProcessId", "columns", "rows", "counts", "truncation", "runtime.temporaryFilesRemoved", "diagnostics", "validation", "next"]
        }),
        json!({
            "path": "model live export-tmdl",
            "usage": "POWERBI_DESKTOP_ORACLE=1 powerbi-cli model live export-tmdl --document <project-dir-or.pbip-or.pbix> --out-dir <fresh-dir> --allow-model-read [--timeout-ms <1000..300000>] --json",
            "summary": "Export the semantic model of one exact already-open Desktop PBIP/PBIX document to a fresh validated TMDL definition through the pinned local Microsoft MCP",
            "tags": ["tmdl", "semantic-model", "pbix", "pbip", "desktop", "modeling-mcp", "read-only", "model-read", "agent", "no-fallback"],
            "readOnly": true,
            "mutates": false,
            "writesOutput": true,
            "writesDataCache": false,
            "returnsModelData": false,
            "returnsModelMetadata": true,
            "explicitOptIn": ["POWERBI_DESKTOP_ORACLE=1", "--allow-model-read"],
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.live.export-tmdl.v1",
            "timeoutContract": "timeout-ms is one end-to-end watchdog for discovery, exact endpoint revalidation, MCP handshake/connect/export, bounded output validation, publication, and a reserved MCP cleanup budget",
            "flags": ["--document <project-dir-or.pbip-or.pbix>", "--out-dir <fresh-dir>", "--allow-model-read", "--timeout-ms <1000..300000>", "--json", "--format json"],
            "examples": ["POWERBI_DESKTOP_ORACLE=1 powerbi-cli model live export-tmdl --document SourceProfile.pbix --out-dir build/source-profile-model --allow-model-read --json"],
            "limitations": ["Windows and an already-open exact PBIP or PBIX document match are required; the command never launches Desktop.", "The pinned local Microsoft Modeling MCP integration must be installed and pass its exact handshake.", "The output is semantic-model TMDL only, not report pages and not a full PBIX-to-PBIP conversion.", "TMDL contains DAX, Power Query source expressions, and possibly small static table values; explicit model-read consent and review are required.", "The destination must be a fresh child of an existing ordinary directory; export occurs in a private sibling quarantine and is published only after bounded shape, UTF-8, link/reparse, and credential-like-text validation."],
            "followUpFields": ["ok", "exitCode", "document.kind", "document.path", "output.kind", "output.root", "output.definition", "output.fileCount", "output.totalBytes", "output.sha256", "engine.desktopProcessId", "engine.modelProcessId", "integration.server", "integration.cleanup", "safety", "limits", "validation", "instructions", "next"]
        }),
        json!({
            "path": "model advanced inventory",
            "aliases": ["model inventory"],
            "usage": "powerbi-cli model advanced inventory --project <project-dir-or.pbip> --json",
            "summary": "Inventory advanced TMDL folders for roles, perspectives, cultures, and named expressions",
            "tags": ["tmdl", "semantic-model", "advanced", "roles", "rls", "perspectives", "cultures", "expressions", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.advanced.inventory.v1",
            "flags": ["--project <project-dir-or.pbip>", "--json", "--format json"],
            "examples": ["powerbi-cli model advanced inventory --project build/sales --json"],
            "followUpFields": ["families[].family", "families[].records[].handle", "validation", "next"]
        }),
        json!({
            "path": "model roles list",
            "aliases": ["model rls list", "model role list"],
            "usage": "powerbi-cli model roles list --project <project-dir-or.pbip> [--include-raw] --json",
            "summary": "List role/RLS TMDL blocks by stable handle",
            "tags": ["tmdl", "semantic-model", "roles", "rls", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.roles.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli model roles list --project build/sales --json"],
            "followUpFields": ["records[].handle", "records[].summary", "validation", "next"]
        }),
        json!({
            "path": "model perspectives list",
            "usage": "powerbi-cli model perspectives list --project <project-dir-or.pbip> [--include-raw] --json",
            "summary": "List perspective TMDL blocks by stable handle",
            "tags": ["tmdl", "semantic-model", "perspectives", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.perspectives.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli model perspectives list --project build/sales --json"],
            "followUpFields": ["records[].handle", "records[].summary", "validation", "next"]
        }),
        json!({
            "path": "model cultures list",
            "aliases": ["model translations list"],
            "usage": "powerbi-cli model cultures list --project <project-dir-or.pbip> [--include-raw] --json",
            "summary": "List culture/translation TMDL blocks by stable handle",
            "tags": ["tmdl", "semantic-model", "cultures", "translations", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.cultures.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli model cultures list --project build/sales --json"],
            "followUpFields": ["records[].handle", "records[].summary", "validation", "next"]
        }),
        json!({
            "path": "model expressions list",
            "aliases": ["model named-expressions list"],
            "usage": "powerbi-cli model expressions list --project <project-dir-or.pbip> [--include-raw] --json",
            "summary": "List named expression TMDL blocks by stable handle",
            "tags": ["tmdl", "semantic-model", "expressions", "named-expressions", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.expressions.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli model expressions list --project build/sales --json"],
            "followUpFields": ["records[].handle", "records[].summary", "validation", "next"]
        }),
        json!({
            "path": "model roles show",
            "aliases": ["model rls show", "model role show"],
            "usage": "powerbi-cli model roles show --project <project-dir-or.pbip> (--handle <role-handle> | --name <role-name>) [--include-raw] --json",
            "summary": "Show one role/RLS TMDL block by stable handle or exact name",
            "tags": ["tmdl", "semantic-model", "roles", "rls", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.roles.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <role-handle>", "--name <role-name>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli model roles show --project build/sales --handle role:Safety --json"],
            "followUpFields": ["record.handle", "record.summary", "record.block", "validation", "next"]
        }),
        json!({
            "path": "model perspectives show",
            "usage": "powerbi-cli model perspectives show --project <project-dir-or.pbip> (--handle <perspective-handle> | --name <perspective-name>) [--include-raw] --json",
            "summary": "Show one perspective TMDL block by stable handle or exact name",
            "tags": ["tmdl", "semantic-model", "perspectives", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.perspectives.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <perspective-handle>", "--name <perspective-name>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli model perspectives show --project build/sales --handle perspective:Executive --json"],
            "followUpFields": ["record.handle", "record.summary", "record.block", "validation", "next"]
        }),
        json!({
            "path": "model cultures show",
            "aliases": ["model translations show"],
            "usage": "powerbi-cli model cultures show --project <project-dir-or.pbip> (--handle <culture-handle> | --name <culture-name>) [--include-raw] --json",
            "summary": "Show one culture/translation TMDL block by stable handle or exact name",
            "tags": ["tmdl", "semantic-model", "cultures", "translations", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.cultures.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <culture-handle>", "--name <culture-name>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli model cultures show --project build/sales --handle culture:de-CH --json"],
            "followUpFields": ["record.handle", "record.summary", "record.block", "validation", "next"]
        }),
        json!({
            "path": "model expressions show",
            "aliases": ["model named-expressions show"],
            "usage": "powerbi-cli model expressions show --project <project-dir-or.pbip> (--handle <expression-handle> | --name <expression-name>) [--include-raw] --json",
            "summary": "Show one named expression TMDL block by stable handle or exact name",
            "tags": ["tmdl", "semantic-model", "expressions", "named-expressions", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.model.expressions.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <expression-handle>", "--name <expression-name>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli model expressions show --project build/sales --handle expression:RefreshDate --json"],
            "followUpFields": ["record.handle", "record.summary", "record.block", "validation", "next"]
        }),
        json!({
            "path": "source-template list",
            "aliases": ["source-templates list", "sourceTemplate list", "sourceTemplates list", "source-template ls"],
            "usage": "powerbi-cli source-template list --project <project-dir-or.pbip> [--table <table>] [--kind <sql|postgres|odbc|excel>] --json",
            "summary": "List credential-free sidecar source templates used by handoff rebind plans",
            "tags": ["source-template", "source", "handoff", "rebind", "partition", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.source-template.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--table <table>", "--kind <sql|postgres|odbc|excel>", "--json", "--format json"],
            "examples": ["powerbi-cli source-template list --project build/sales --json"],
            "followUpFields": ["templates[].handle", "templates[].partitionHandle", "templates[].mTemplate", "templates[].safety", "next"]
        }),
        json!({
            "path": "source-template show",
            "aliases": ["source-templates show", "sourceTemplate show", "source-template get"],
            "usage": "powerbi-cli source-template show --project <project-dir-or.pbip> (--handle <source-template-handle> | --name <template-name>) --json",
            "summary": "Show one source template, its partition mapping, M template preview, and safety findings",
            "tags": ["source-template", "source", "handoff", "rebind", "partition", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.source-template.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <source-template-handle>", "--name <template-name>", "--json", "--format json"],
            "examples": ["powerbi-cli source-template show --project build/sales --handle source-template:FactSales:FactSales --json"],
            "followUpFields": ["sourceTemplate.handle", "sourceTemplate.partitionHandle", "sourceTemplate.mTemplate", "sourceTemplate.requirements", "sourceTemplate.safety", "next"]
        }),
        json!({
            "path": "source-template add",
            "aliases": ["source-templates add", "sourceTemplate add", "source-template create"],
            "usage": "powerbi-cli source-template add --project <project-dir-or.pbip> (--handle <partition-handle> | --table <table> [--partition <partition-name>]) [--name <template-name>] --kind <sql|postgres|odbc|excel> [--server <placeholder> | --dsn <placeholder> | --file <workbook-or-placeholder>] [--database <placeholder>] [--schema <schema>] [--object <table-or-view>] [--item <sheet-or-table>] [--item-kind <Sheet|Table>] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Add or replace a credential-free SQL Server, PostgreSQL, ODBC, or Excel source template sidecar without changing executable partitions",
            "tags": ["source-template", "source", "handoff", "rebind", "partition", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.source-template.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <partition-handle>", "--table <table>", "--partition <partition-name-or-handle>", "--name <template-name>", "--kind <sql|postgres|odbc|excel>", "--server <placeholder>", "--dsn <placeholder>", "--database <placeholder>", "--schema <schema>", "--sql-schema <schema>", "--object <table-or-view>", "--file <workbook-or-placeholder>", "--path <workbook-or-placeholder>", "--item <sheet-or-table>", "--sheet <worksheet>", "--item-kind <Sheet|Table>", "--description <text>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": [
                "powerbi-cli source-template add --project build/sales --table FactSales --kind sql --server <server> --database <database> --schema dbo --object FactSales --dry-run --json",
                "powerbi-cli source-template add --project build/sales --table FactSales --kind postgres --server <server> --database <database> --schema public --object <object> --dry-run --json",
                "powerbi-cli source-template add --project build/sales --table FactSales --kind odbc --dsn <dsn> --database <database> --schema <schema> --object <object> --dry-run --json",
                "powerbi-cli source-template add --project build/sales --table FactSales --kind excel --file <workbook.xlsx> --sheet FactSales --dry-run --json"
            ],
            "limitations": ["ODBC --dsn accepts only a bare DSN name; semicolon/equal connection attributes and embedded credentials are refused.", "Excel apply materializes an absolute workbook path; move-safe packages should reapply or patch that path on the target machine."],
            "followUpFields": ["dryRun", "changes[].before", "changes[].after", "readbackCommand", "rebindPlanCommand", "handoffCheckCommand", "validateCommand"]
        }),
        json!({
            "path": "source-template apply",
            "aliases": ["source-template materialize", "source-templates apply", "sourceTemplate apply"],
            "usage": "powerbi-cli source-template apply --project <project-dir-or.pbip> (--handle <source-template-handle> | --name <template-name>) [--server <server> | --dsn <dsn> | --file <workbook.xlsx>] [--database <database>] [--schema <schema>] [--object <table-or-view>] [--item <sheet-or-table>] [--item-kind <Sheet|Table>] [--replace-existing --confirm <partition-handle>] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Materialize one credential-free source template into a generated dummy partition, or explicitly retarget a confirmed existing credential-free partition",
            "tags": ["source-template", "source", "handoff", "rebind", "partition", "mutation", "work-machine", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.source-template.apply.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <source-template-handle>", "--name <template-name>", "--server <server>", "--dsn <dsn>", "--database <database>", "--schema <schema>", "--sql-schema <schema>", "--object <table-or-view>", "--file <workbook.xlsx>", "--path <workbook.xlsx>", "--item <sheet-or-table>", "--sheet <worksheet>", "--item-kind <Sheet|Table>", "--replace-existing", "--confirm <partition-handle>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": [
                "powerbi-cli source-template apply --project build/sales --handle source-template:FactSales:FactSales --server sql.example.internal --database Sales --dry-run --json",
                "powerbi-cli source-template apply --project build/sales --handle source-template:FactSales:FactSales --server pg.example.internal:5432 --database Sales --out-dir build/sales-live --json",
                "powerbi-cli source-template materialize --project build/sales --handle source-template:DimCustomer:DimCustomer --dsn CorpWarehouse --database Sales --in-place --json",
                "powerbi-cli source-template apply --project build/sales --handle source-template:FactSales:FactSales --file C:\\\\data\\\\sales.xlsx --sheet FactSales --replace-existing --confirm partition:FactSales:FactSales --dry-run --json"
            ],
            "limitations": ["Applies one template per command.", "Existing source replacement requires --replace-existing plus the exact --confirm partition handle and is limited to recognized credential-free SQL, PostgreSQL, ODBC, and external-file sources.", "Unknown, web, and credential-bearing sources are refused.", "Credentials cannot be supplied or embedded; Power BI Desktop performs database authentication after the PBIP opens."],
            "followUpFields": ["projectModified", "credentialsEmbedded", "requiresDesktopAuthentication", "connection.parameters", "changes[].afterSource", "readbackCommand", "validateCommand", "instructions"]
        }),
        json!({
            "path": "report build",
            "usage": "powerbi-cli report build --schema <schema.json> [--profile <profile.json>] [--spec <dashboard.json>] (--dry-run | --out-dir <project-dir> [--force]) --json",
            "summary": "Compile a data schema plus optional profile/dashboard spec into an offline-safe PBIP/PBIR/TMDL project using supported primitives only",
            "tags": ["report", "dashboard", "build", "schema", "profile", "spec", "agent", "offline"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.build.v1",
            "flags": ["--schema <schema.json>", "--profile <profile.json>", "--spec <dashboard.json>", "--dry-run", "--out-dir <project-dir>", "--out <project-dir>", "--force", "--json", "--format json"],
            "examples": [
                "powerbi-cli report build --schema examples/sales.schema.json --out-dir build/sales --json",
                "powerbi-cli report build --schema examples/sales.schema.json --profile build/sales.profile.json --spec examples/sales.dashboard.json --out-dir build/sales --force --json"
            ],
            "followUpFields": ["projectDir", "compiled.counts", "changes[].kind", "changes[].action", "changes[].path", "changes[].before", "changes[].after", "executedPrimitives", "inspectCommand", "validateCommand", "handoffCheckCommand", "fixtureNormalizeCommand", "desktopOpenCheckCommand", "proof", "next"]
        }),
        json!({
            "path": "report spec validate",
            "usage": "powerbi-cli report spec validate [--schema <schema.json>] --spec <dashboard.json> [--profile <profile.json>] --json",
            "summary": "Validate a dashboard spec shape, and compile-check it against a schema/profile when --schema is supplied before report build",
            "tags": ["report", "dashboard", "spec", "schema", "profile", "validation", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.spec.validate.v1",
            "flags": ["--schema <schema.json>", "--profile <profile.json>", "--spec <dashboard.json>", "<dashboard.json>", "--json", "--format json"],
            "examples": ["powerbi-cli report spec validate --schema examples/sales.schema.json --spec examples/sales.dashboard.json --json", "powerbi-cli report spec validate --spec examples/sales.dashboard.json --json"],
            "followUpFields": ["ok", "exitCode", "validationLevel", "compiled.counts", "warnings", "errors", "next"],
            "validationLevels": [
                {"level": "shape-only", "ok": null, "meaning": "Checks JSON/spec shape only; cannot prove field references, visual roles, measures, or build compatibility."},
                {"level": "compiled", "ok": "boolean", "meaning": "Compiles the spec against a schema and enforces generated visual role contracts."}
            ]
        }),
        json!({
            "path": "report spec fields",
            "usage": "powerbi-cli report spec fields --schema <schema.json> [--profile <profile.json>] --json",
            "summary": "List exact column and measure binding references for writing dashboard specs without guessing raw schema JSON",
            "tags": ["report", "dashboard", "spec", "fields", "schema", "profile", "bindings", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.spec.fields.v1",
            "flags": ["--schema <schema.json>", "--profile <profile.json>", "--json", "--format json"],
            "examples": ["powerbi-cli report spec fields --schema examples/sales.schema.json --profile examples/sales.profile.json --json"],
            "followUpFields": ["ok", "exitCode", "supportedVisualTypes", "tables[].columns[].reference", "tables[].measures[].reference", "tables[].columns[].structuredBinding", "tables[].measures[].structuredBinding", "fields[]", "examples", "next"]
        }),
        json!({
            "path": "report plan",
            "usage": "powerbi-cli report plan --schema <schema.json> --profile <profile.json> --objective <goal> --out <dashboard.json> --json",
            "summary": "Create a deterministic starter dashboard spec from schema/profile candidates and an explicit dashboard objective",
            "tags": ["report", "dashboard", "plan", "intent", "spec", "agent"],
            "readOnly": false,
            "mutates": true,
            "writesDataCache": false,
            "requiresOutput": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.plan.v1",
            "flags": ["--schema <schema.json>", "--profile <profile.json>", "--intent <intent.md|text>", "--objective <goal>", "--out <dashboard.json>", "--force", "--json", "--format json"],
            "examples": ["powerbi-cli report plan --schema examples/sales.schema.json --profile build/sales.profile.json --objective \"Executive overview with trends and segment breakdown\" --out build/sales.dashboard.json --json"],
            "followUpFields": ["ok", "schemaPath", "profilePath", "specPath", "spec", "compiled.counts", "decisions", "warnings", "next"]
        }),
        json!({
            "path": "report design-plan",
            "aliases": ["report design plan", "report designplan"],
            "usage": "powerbi-cli report design-plan --project <project-dir-or.pbip> --json",
            "summary": "Profile a model/report and return agent-ready visual, layout, drilldown, and style authoring opportunities with exact next commands",
            "tags": ["pbir", "report", "design", "planning", "visuals", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.designPlan.v1",
            "flags": ["--project <project-dir-or.pbip>", "--json", "--format json"],
            "examples": ["powerbi-cli report design-plan --project build/sales --json"],
            "followUpFields": ["profile.counts", "candidates.dateColumns", "candidates.categoryColumns", "candidates.measures", "opportunities[].command", "recommendedWorkflow"]
        }),
        json!({
            "path": "report tree",
            "aliases": ["report objects tree"],
            "usage": "powerbi-cli report tree --project <project-dir-or.pbip> [--include-raw] --json",
            "summary": "Return a stable navigable report object tree across pages, visuals, bindings, filters, slicers, bookmarks, and interactions",
            "tags": ["pbir", "report", "objects", "tree", "inspect", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.objects.tree.v1",
            "flags": ["--project <project-dir-or.pbip>", "--json", "--format json"],
            "examples": ["powerbi-cli report tree --project build/sales --json", "powerbi-cli report objects tree --project build/sales --json"],
            "followUpFields": ["ok", "projectDir", "counts", "tree.handle", "tree.children[].handle", "objects[].handle", "objects[].kind", "objects[].path", "next"]
        }),
        json!({
            "path": "report find",
            "aliases": ["report objects find"],
            "usage": "powerbi-cli report find --project <project-dir-or.pbip> [--kind <kind>] [--name-contains <text>] [--title-contains <text>] [--visual-type <type>] [--path-contains <text>] [--include-raw] --json",
            "summary": "Search report objects by stable metadata instead of guessing PBIR file paths",
            "tags": ["pbir", "report", "objects", "find", "search", "inspect", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.objects.find.v1",
            "flags": ["--project <project-dir-or.pbip>", "--kind <kind>", "--name-contains <text>", "--title-contains <text>", "--visual-type <type>", "--path-contains <text>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report find --project build/sales --kind visual --json", "powerbi-cli report find --project build/sales --title-contains Revenue --json"],
            "followUpFields": ["ok", "predicates", "objects[].handle", "objects[].kind", "objects[].path", "counts.matched", "next"]
        }),
        json!({
            "path": "report cat",
            "aliases": ["report objects cat", "report object show"],
            "usage": "powerbi-cli report cat --project <project-dir-or.pbip> --handle <object-handle> [--include-raw] --json",
            "summary": "Show one report object by stable handle; raw PBIR content is returned only with --include-raw",
            "tags": ["pbir", "report", "objects", "cat", "show", "raw", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.objects.cat.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <object-handle>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report cat --project build/sales --handle visual:ReportSectionOverview:VisualContainerSalesKpi --json", "powerbi-cli report cat --project build/sales --handle <object-handle> --include-raw --json"],
            "followUpFields": ["ok", "object.handle", "object.kind", "object.path", "raw", "rawIncluded", "next"]
        }),
        json!({
            "path": "report query",
            "aliases": ["report objects query"],
            "usage": "powerbi-cli report query --project <project-dir-or.pbip> --selector <selector> [--include-raw] --json",
            "summary": "Run a constrained stable-selector query over report objects for agent automation",
            "tags": ["pbir", "report", "objects", "query", "selector", "inspect", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.objects.query.v1",
            "flags": ["--project <project-dir-or.pbip>", "--selector handle:<handle>|kind:<kind>|visualType:<type>|title~:<text>|name~:<text>|path~:<text>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report query --project build/sales --selector kind:visual --json", "powerbi-cli report query --project build/sales --selector title~:Revenue --json"],
            "followUpFields": ["ok", "selector", "objects[].handle", "objects[].kind", "counts.matched", "next"]
        }),
        json!({
            "path": "report audit",
            "usage": "powerbi-cli report audit --project <project-dir-or.pbip> [--profile agent-safe|handoff] [--include-raw] --json",
            "summary": "Audit report PBIR state for persisted values, raw-literal risks, stale references, and handoff hygiene issues",
            "tags": ["pbir", "report", "audit", "sanitize", "handoff", "safety", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.audit.v1",
            "flags": ["--project <project-dir-or.pbip>", "--profile agent-safe|handoff", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report audit --project build/sales --json", "powerbi-cli report audit --project build/sales --profile handoff --json"],
            "followUpFields": ["ok", "profile", "counts.findings", "findings[].ruleId", "findings[].severity", "findings[].handle", "findings[].supportedAction", "sanitizePlanCommand", "next"]
        }),
        json!({
            "path": "report sanitize plan",
            "usage": "powerbi-cli report sanitize plan --project <project-dir-or.pbip> [--profile agent-safe|handoff] --json",
            "summary": "Create a deterministic sanitize plan before clearing persisted report filter/slicer state or flagging plan-only manual review items",
            "tags": ["pbir", "report", "sanitize", "plan", "handoff", "safety", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.sanitize.plan.v1",
            "flags": ["--project <project-dir-or.pbip>", "--profile agent-safe|handoff", "--json", "--format json"],
            "examples": ["powerbi-cli report sanitize plan --project build/sales --json", "powerbi-cli report sanitize plan --project build/sales --profile handoff --json"],
            "followUpFields": ["ok", "planFingerprint", "confirmToken", "actions[].kind", "actions[].handles", "actions[].applySupported", "actions[].blockedReason", "next"]
        }),
        json!({
            "path": "report sanitize apply",
            "usage": "powerbi-cli report sanitize apply --project <project-dir-or.pbip> [--profile agent-safe|handoff] (--dry-run | --out-dir <dir> | --in-place --confirm sanitize:<planFingerprint>) --json",
            "summary": "Apply only supported sanitize actions under guarded dry-run/out-dir/in-place semantics",
            "tags": ["pbir", "report", "sanitize", "apply", "handoff", "safety", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.sanitize.apply.v1",
            "flags": ["--project <project-dir-or.pbip>", "--profile agent-safe|handoff", "--dry-run", "--out-dir <dir>", "--in-place", "--confirm sanitize:<planFingerprint>", "--json", "--format json"],
            "examples": ["powerbi-cli report sanitize apply --project build/sales --dry-run --json", "powerbi-cli report sanitize apply --project build/sales --out-dir build/sales-sanitized --json"],
            "followUpFields": ["ok", "dryRun", "mode", "planFingerprint", "actions[].kind", "changes[].path", "changes[].jsonPointer", "postAudit", "validateCommand", "readbackCommand", "next"]
        }),
        json!({
            "path": "report wireframe export",
            "usage": "powerbi-cli report wireframe export <project-dir-or.pbip> --json",
            "summary": "Export report pages, visual geometry, bindings, and report handles as JSON without Power BI Desktop",
            "tags": ["pbir", "report", "wireframe", "layout", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "reportWireframe.v1",
            "flags": ["--json", "--format json"],
            "examples": ["powerbi-cli report wireframe export build/sales --json"],
            "followUpFields": ["handles", "pages", "counts", "next", "warnings", "errors"]
        }),
        json!({
            "path": "report layout auto",
            "aliases": ["report layouts auto", "report layout arrange"],
            "usage": "powerbi-cli report layout auto --project <project-dir-or.pbip> [--page <page-name-or-handle>] [--preset overview|analysis|detail|grid] [--margin <n>] [--gap <n>] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Reposition existing visuals into deterministic responsive canvas slots without changing bindings or formatting",
            "tags": ["pbir", "report", "layout", "visual", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.layout.autoMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--page <page-name-or-handle>", "--handle <page-handle>", "--preset overview|analysis|detail|grid", "--margin <n>", "--gap <n>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report layout auto --project build/sales --page page:ReportSectionOverview --preset overview --dry-run --json"],
            "followUpFields": ["dryRun", "mode", "layoutPlan.pages", "layoutPlan.changedVisuals", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report pages list",
            "usage": "powerbi-cli report pages list --project <project-dir-or.pbip> --json",
            "summary": "List PBIR report pages with stable page handles and visual counts",
            "tags": ["pbir", "report", "page", "layout", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "reportPagesList.v1",
            "flags": ["--project <project-dir-or.pbip>", "--json", "--format json"],
            "examples": ["powerbi-cli report pages list --project build/sales --json"],
            "followUpFields": ["pages[].handle", "pages[].visualHandles", "next"]
        }),
        json!({
            "path": "report pages show",
            "usage": "powerbi-cli report pages show --project <project-dir-or.pbip> (--handle <page-handle> | --page <page-name-or-handle>) --json",
            "summary": "Show one PBIR report page with visual geometry and bindings",
            "tags": ["pbir", "report", "page", "visual", "layout", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "reportPagesShow.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <page-handle>", "--page <page-name-or-handle>", "--name <page-name>", "--json", "--format json"],
            "examples": ["powerbi-cli report pages show --project build/sales --handle page:ReportSectionOverview --json"],
            "followUpFields": ["page.handle", "page.visuals[].handle", "page.visuals[].position", "next"]
        }),
        json!({
            "path": "report pages add",
            "aliases": ["report pages create"],
            "usage": "powerbi-cli report pages add --project <project-dir-or.pbip> --display-name <name> [--name <pbir-page-name>] [--width <n>] [--height <n>] [--display-option <mode>] [--before <page-handle>|--after <page-handle>] [--set-active] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Add an empty PBIR report page and update pageOrder with guarded output semantics",
            "tags": ["pbir", "report", "page", "layout", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.pages.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--display-name <name>", "--name <pbir-page-name>", "--width <n>", "--height <n>", "--display-option <mode>", "--before <page-handle>", "--after <page-handle>", "--set-active", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report pages add --project build/sales --display-name \"Executive Summary\" --after page:ReportSectionOverview --set-active --dry-run --json"],
            "followUpFields": ["dryRun", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report pages update",
            "aliases": ["report pages patch"],
            "usage": "powerbi-cli report pages update --project <project-dir-or.pbip> (--handle <page-handle> | --page <page-name-or-handle>) [--display-name <name>] [--width <n>] [--height <n>] [--display-option <mode>] [--allow-visuals-outside-page] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Patch PBIR page display metadata without renaming the internal page handle",
            "tags": ["pbir", "report", "page", "layout", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.pages.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <page-handle>", "--page <page-name-or-handle>", "--display-name <name>", "--width <n>", "--height <n>", "--display-option <mode>", "--allow-visuals-outside-page", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report pages update --project build/sales --handle page:ReportSectionOverview --display-name \"Operations\" --dry-run --json"],
            "followUpFields": ["dryRun", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report pages reorder",
            "aliases": ["report pages order"],
            "usage": "powerbi-cli report pages reorder --project <project-dir-or.pbip> --order <page-handle,...> (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Replace PBIR pageOrder after resolving every page handle exactly once",
            "tags": ["pbir", "report", "page", "layout", "order", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.pages.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--order <page-handle,...>", "--page <page-handle>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report pages reorder --project build/sales --order page:ReportSectionOverview,page:ReportSectionExecutiveSummary --dry-run --json"],
            "followUpFields": ["dryRun", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report pages set-active",
            "aliases": ["report pages activate"],
            "usage": "powerbi-cli report pages set-active --project <project-dir-or.pbip> (--handle <page-handle> | --page <page-name-or-handle>) (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Set pages.json activePageName to an existing PBIR page",
            "tags": ["pbir", "report", "page", "layout", "active", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.pages.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <page-handle>", "--page <page-name-or-handle>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report pages set-active --project build/sales --handle page:ReportSectionOverview --dry-run --json"],
            "followUpFields": ["dryRun", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report pages delete-empty",
            "aliases": ["report pages delete"],
            "usage": "powerbi-cli report pages delete-empty --project <project-dir-or.pbip> (--handle <page-handle> | --page <page-name-or-handle>) (--dry-run | --in-place --confirm <page-handle> | --out-dir <dir>) --json",
            "summary": "Delete only a simple empty PBIR page; pages with visuals or unknown files are refused",
            "tags": ["pbir", "report", "page", "layout", "delete", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.pages.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <page-handle>", "--page <page-name-or-handle>", "--confirm <page-handle>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report pages delete-empty --project build/sales --handle page:ReportSectionScratch --dry-run --json"],
            "followUpFields": ["dryRun", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report drilldown set-hierarchy",
            "aliases": ["report drilldown hierarchy", "report drilldown set", "report drill-down set-hierarchy"],
            "usage": "powerbi-cli report drilldown set-hierarchy --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-title>) --field <table[column]> --field <table[column]>... (--dry-run | --in-place | --out-dir <dir>) [--include-raw] --json",
            "summary": "Replace a category-axis chart's Category projections with a multi-column hierarchy and enable its Desktop drill controls",
            "tags": ["pbir", "report", "drilldown", "hierarchy", "visual", "binding", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.drilldown.hierarchyMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-title>", "--field <table[column]>", "--target <table[column]>", "--category <table[column]>", "--dry-run", "--in-place", "--out-dir <dir>", "--include-raw", "--json", "--format json"],
            "limitations": ["Requires an existing line, area, bar, column, or combo chart with its numeric field wells already bound.", "Requires at least two model columns.", "Scatter Category is intentionally refused because Microsoft Report Authoring permits only one projection in that role.", "Sets the first hierarchy field active as the initial level; later end-user drill position and expanded data state remain transient."],
            "examples": ["powerbi-cli report drilldown set-hierarchy --project build/sales --handle <visual-handle> --field 'DimDate[FiscalYear]' --field 'DimDate[Month]' --dry-run --json"],
            "followUpFields": ["dryRun", "mode", "target.handle", "hierarchyPlan.fields", "hierarchyPlan.before", "hierarchyPlan.after", "hierarchyPlan.controls.before", "hierarchyPlan.controls.after", "changes[].jsonPointer", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report drillthrough set",
            "aliases": ["report drillthrough add", "report drillthrough create"],
            "usage": "powerbi-cli report drillthrough set --project <project-dir-or.pbip> (--page <page-name-or-handle> | --handle <page-handle>) (--target <table[column]> | --table <table> --column <column>) [--keep-all-filters true|false] [--keep-visible] (--dry-run | --in-place | --out-dir <dir>) [--include-raw] --json",
            "summary": "Mark an existing PBIR page as a same-report drillthrough target using linked pageBinding and filterConfig metadata",
            "tags": ["pbir", "report", "drillthrough", "page", "binding", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "schema-golden",
            "outputSchema": "powerbi-cli.report.drillthrough.setMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--page <page-name-or-handle>", "--handle <page-handle>", "--target <table[column]>", "--table <table>", "--column <column>", "--keep-all-filters true|false", "--no-keep-all-filters", "--keep-visible", "--dry-run", "--in-place", "--out-dir <dir>", "--include-raw", "--json", "--format json"],
            "limitations": ["Same-report drillthrough only.", "One model column target only.", "Does not author visual drillthrough action links.", "Does not author cross-report drillthrough."],
            "examples": ["powerbi-cli report drillthrough set --project build/sales --page page:ReportSectionOverview --target 'DimCustomer[Segment]' --dry-run --json"],
            "followUpFields": ["dryRun", "mode", "page.handle", "target", "drillthroughPlan.before", "drillthroughPlan.after", "drillthroughPlan.filterName", "drillthroughPlan.after.binding.parameters[].boundFilter", "drillthroughPlan.after.binding.parameters[].fieldExpr", "changes[].jsonPointer", "readbackCommand", "pageReadbackCommand", "filterReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report drillthrough show",
            "aliases": ["report drillthrough get"],
            "usage": "powerbi-cli report drillthrough show --project <project-dir-or.pbip> (--page <page-name-or-handle> | --handle <page-handle>) [--include-raw] --json",
            "summary": "Read linked drillthrough pageBinding parameters and paired Drillthrough filters",
            "tags": ["pbir", "report", "drillthrough", "page", "binding", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "schema-golden",
            "outputSchema": "powerbi-cli.report.drillthrough.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--page <page-name-or-handle>", "--handle <page-handle>", "--include-raw", "--no-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report drillthrough show --project build/sales --page page:ReportSectionOverview --json"],
            "followUpFields": ["page.handle", "drillthrough.enabled", "drillthrough.binding.parameters[].boundFilter", "drillthrough.binding.parameters[].fieldExpr", "drillthrough.binding.parameters[].target", "drillthrough.filters", "readbackCommand", "pageReadbackCommand", "validateCommand"]
        }),
        json!({
            "path": "report drillthrough clear",
            "aliases": ["report drillthrough remove", "report drillthrough delete"],
            "usage": "powerbi-cli report drillthrough clear --project <project-dir-or.pbip> (--page <page-name-or-handle> | --handle <page-handle>) [--restore-visible] (--dry-run | --in-place --confirm <page-handle> | --out-dir <dir>) [--include-raw] --json",
            "summary": "Remove drillthrough page type, pageBinding, and existing drillthrough-created page filters with guarded output semantics",
            "tags": ["pbir", "report", "drillthrough", "page", "binding", "clear", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "schema-golden",
            "outputSchema": "powerbi-cli.report.drillthrough.clearMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--page <page-name-or-handle>", "--handle <page-handle>", "--restore-visible", "--dry-run", "--in-place", "--confirm <page-handle>", "--out-dir <dir>", "--include-raw", "--json", "--format json"],
            "limitations": ["Does not remove visual action links.", "Does not infer whether a hidden page should become visible unless --restore-visible is passed."],
            "examples": ["powerbi-cli report drillthrough clear --project build/sales --page page:ReportSectionOverview --dry-run --json"],
            "followUpFields": ["dryRun", "mode", "page.handle", "drillthroughPlan.before", "drillthroughPlan.after", "drillthroughPlan.removedFilters", "changes[].jsonPointer", "readbackCommand", "pageReadbackCommand", "filterReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report bookmarks list",
            "aliases": ["report bookmark list", "report bookmarks ls"],
            "usage": "powerbi-cli report bookmarks list --project <project-dir-or.pbip> [--include-raw] --json",
            "summary": "List raw PBIR bookmark files with stable handles, bookmark order/group metadata, and data-value safety warnings",
            "tags": ["pbir", "report", "bookmark", "bookmarks", "readback", "state", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.bookmarks.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report bookmarks list --project build/sales --json", "powerbi-cli report bookmarks list --project build/sales --include-raw --json"],
            "followUpFields": ["bookmarks[].handle", "bookmarks[].state", "bookmarks[].options", "bookmarks[].safety", "bookmarksMetadata", "bookmarkDiagnostics", "next"]
        }),
        json!({
            "path": "report bookmarks show",
            "aliases": ["report bookmark show", "report bookmarks get", "report bookmark get"],
            "usage": "powerbi-cli report bookmarks show --project <project-dir-or.pbip> --handle <bookmark-handle> [--no-raw] --json",
            "summary": "Show one raw PBIR bookmark by stable handle, including captured state summary and persisted-value safety metadata",
            "tags": ["pbir", "report", "bookmark", "bookmarks", "readback", "state", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.bookmarks.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <bookmark-handle>", "--include-raw", "--no-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report bookmarks show --project build/sales --handle bookmark:BookmarkExecutive --json"],
            "followUpFields": ["bookmark.handle", "bookmark.raw", "bookmark.state", "bookmark.options", "bookmark.safety", "bookmarkDiagnostics", "readbackCommand", "validateCommand", "next"]
        }),
        json!({
            "path": "report bookmarks set-display-name",
            "aliases": ["report bookmarks rename", "report bookmark set-display-name"],
            "usage": "powerbi-cli report bookmarks set-display-name --project <project-dir-or.pbip> --handle <bookmark-handle> --display-name <text> (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Patch only bookmark displayName metadata without capturing or changing bookmark state",
            "tags": ["pbir", "report", "bookmark", "metadata", "display-name", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.bookmarks.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <bookmark-handle>", "--display-name <text>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "limitations": ["Does not capture or modify explorationState.", "Grouped metadata child display names remain in the bookmark file."],
            "examples": ["powerbi-cli report bookmarks set-display-name --project build/sales --handle bookmark:Executive --display-name \"Executive View\" --dry-run --json"],
            "followUpFields": ["dryRun", "mode", "action", "target.bookmark", "changes", "readbackCommand", "validateCommand"]
        }),
        json!({
            "path": "report bookmarks reorder",
            "aliases": ["report bookmarks order", "report bookmark reorder"],
            "usage": "powerbi-cli report bookmarks reorder --project <project-dir-or.pbip> --order <bookmark-handle,...> (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Reorder flat bookmark metadata without changing captured bookmark state",
            "tags": ["pbir", "report", "bookmark", "metadata", "order", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.bookmarks.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--order <bookmark-handle,...>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "limitations": ["Grouped bookmark metadata is read-only until Desktop-authored fixtures prove safe reorder semantics.", "Order must include every bookmark exactly once."],
            "examples": ["powerbi-cli report bookmarks reorder --project build/sales --order bookmark:A,bookmark:B --dry-run --json"],
            "followUpFields": ["dryRun", "mode", "action", "target.beforeOrder", "target.afterOrder", "changes", "readbackCommand", "validateCommand"]
        }),
        json!({
            "path": "report bookmarks delete",
            "aliases": ["report bookmarks remove", "report bookmark delete"],
            "usage": "powerbi-cli report bookmarks delete --project <project-dir-or.pbip> --handle <bookmark-handle> (--dry-run | --in-place --confirm <bookmark-handle> | --out-dir <dir>) --json",
            "summary": "Delete one bookmark file and remove it from bookmark metadata with guarded output semantics",
            "tags": ["pbir", "report", "bookmark", "delete", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.bookmarks.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <bookmark-handle>", "--dry-run", "--in-place", "--confirm <bookmark-handle>", "--out-dir <dir>", "--json", "--format json"],
            "limitations": ["Does not capture replacement bookmark state.", "In-place delete requires exact handle confirmation."],
            "examples": ["powerbi-cli report bookmarks delete --project build/sales --handle bookmark:OldView --dry-run --json"],
            "followUpFields": ["dryRun", "mode", "action", "target.bookmark", "changes", "readbackCommand", "validateCommand"]
        }),
        json!({
            "path": "report filters list",
            "aliases": ["report filter list", "report filters ls"],
            "usage": "powerbi-cli report filters list --project <project-dir-or.pbip> [--scope all|report|page|visual] [--page <page-name-or-handle>] [--visual <visual-name-or-handle>] [--include-raw] --json",
            "summary": "List raw PBIR report, page, and visual filters with stable handles and data-value safety warnings",
            "tags": ["pbir", "report", "filter", "page-filter", "visual-filter", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.filters.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--scope all|report|page|visual", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report filters list --project build/sales --json", "powerbi-cli report filters list --project build/sales --scope visual --json"],
            "limitations": ["Handles use filter names when present and content fingerprints for nameless legacy entries.", "Legacy /filters entries carry #legacy; duplicate identities are listed with deterministic suffixes but marked handleAmbiguous and cannot be mutated by handle.", "Ordinal handle formats are rejected; re-list after upgrading."],
            "followUpFields": ["filters[].handle", "filters[].scope", "filters[].owner", "filters[].target", "filters[].safety", "next"]
        }),
        json!({
            "path": "report filters show",
            "aliases": ["report filter show", "report filters get", "report filter get"],
            "usage": "powerbi-cli report filters show --project <project-dir-or.pbip> --handle <filter-handle> [--no-raw] --json",
            "summary": "Show one raw PBIR filter by stable handle, including owner readback and persisted-value safety metadata",
            "tags": ["pbir", "report", "filter", "page-filter", "visual-filter", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.filters.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <filter-handle>", "--include-raw", "--no-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report filters show --project build/sales --handle filter:report:main:ReportRegionFilter --json"],
            "limitations": ["Ordinal filter handles from earlier releases are rejected with a re-list hint."],
            "followUpFields": ["filter.handle", "filter.raw", "filter.safety", "readbackCommand", "validateCommand", "next"]
        }),
        json!({
            "path": "report filters add",
            "aliases": ["report filter add", "report filters create", "report filter create"],
            "usage": "powerbi-cli report filters add --project <project-dir-or.pbip> [--scope report|page|visual] [--page <page-name-or-handle>] [--visual <visual-name-or-handle>] (--target <table[column]> | --table <table> --column <column>) [--condition-type categorical|range|topn|relative-date] ((--value <text> | --value-json <json> | --values-json <json-array>)... | [--min <number>] [--max <number>] | (--top <N> | --bottom <N>) --by <measure> | --relative last|next|this --unit days|weeks|months|years|calendar-weeks|calendar-months|calendar-years --span <N>) (--dry-run | --in-place | --out-dir <dir>) [--name <filter-name>] [--display-name <label>] [--include-raw] --json",
            "summary": "Add one categorical, numeric range, TopN, or relative-date PBIR filter with TMDL type checks and guarded output semantics",
            "tags": ["pbir", "report", "filter", "page-filter", "visual-filter", "mutation", "add", "categorical", "range", "topn", "relative-date", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "schema-golden",
            "outputSchema": "powerbi-cli.report.filters.addMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--scope report|page|visual", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--target <table[column]>", "--table <table>", "--column <column>", "--condition-type categorical|range|topn|relative-date", "--name <filter-name>", "--display-name <label>", "--value <text>", "--value-json <json>", "--values-json <json-array>", "--min <number>", "--max <number>", "--top <N>", "--bottom <N>", "--by <measure>", "--relative last|next|this", "--unit days|weeks|months|years|calendar-weeks|calendar-months|calendar-years", "--span <N>", "--dry-run", "--in-place", "--out-dir <dir>", "--include-raw", "--json", "--format json"],
            "limitations": ["TopN is visual-only; report/page TopN returns unsupported_feature.", "Numeric range, TopN, and relative-date shapes are schema-golden and still await Desktop canvas/open-save validation.", "Stores categorical values and numeric thresholds in PBIR; review them before sharing outside the work environment.", "Generated names include raw target/type and condition hashes, remain at most 50 characters, and allow distinct conditions on one field; exact duplicate conditions still collide loudly.", "Writes only /filterConfig/filters.", "No tuple, arbitrary advanced-expression, filter sort, or type-changing update helper."],
            "examples": ["powerbi-cli report filters add --project build/sales --scope report --target 'DimCustomer[Segment]' --value Enterprise --dry-run --json", "powerbi-cli report filters add --project build/sales --page page:ReportSectionOverview --target 'FactSales[Revenue]' --min 1000 --max 5000 --out-dir build/sales-range-filter --json", "powerbi-cli report filters add --project build/sales --scope visual --visual <visual-handle> --target 'DimCustomer[CustomerName]' --top 10 --by 'Total Revenue' --dry-run --json", "powerbi-cli report filters add --project build/sales --target 'DimDate[Date]' --relative last --unit months --span 12 --dry-run --json"],
            "followUpFields": ["dryRun", "mode", "target.handle", "target.target", "owner", "filterPlan.beforeCount", "filterPlan.afterCount", "changes[].jsonPointer", "readbackCommand", "filterReadbackCommand", "ownerReadbackCommand", "rawReviewCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report filters update",
            "aliases": ["report filter update", "report filters edit", "report filter edit"],
            "usage": "powerbi-cli report filters update --project <project-dir-or.pbip> --handle <filter-handle> (--display-name <label> | (--value <text> | --value-json <json> | --values-json <json-array>)...) [--condition-type categorical|range|topn|relative-date] (--dry-run | --in-place | --out-dir <dir>) [--include-raw] --json",
            "summary": "Update one filter by stable handle: change any display name or replace categorical values while preserving filter type",
            "tags": ["pbir", "report", "filter", "page-filter", "visual-filter", "mutation", "update", "categorical", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.filters.updateMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <filter-handle>", "--display-name <label>", "--condition-type categorical|range|topn|relative-date", "--value <text>", "--value-json <json>", "--values-json <json-array>", "--dry-run", "--in-place", "--out-dir <dir>", "--include-raw", "--json", "--format json"],
            "limitations": ["Refuses every filter type change with unsupported_feature.", "Only categorical In-filter values can be replaced; range bounds, TopN ranking, and relative windows require reviewed delete/add operations.", "Dry-run always returns exact raw before/after filter JSON; applied output includes raw only with --include-raw."],
            "examples": ["powerbi-cli report filters update --project build/sales --handle filter:report:main:ReportSegmentFilter --values-json '[\"Enterprise\",\"SMB\"]' --display-name 'Customer segment' --dry-run --json", "powerbi-cli report filters update --project build/sales --handle filter:visual:ReportSectionOverview:VisualContainerCompanies:CompanyFilter --display-name 'Top companies' --out-dir build/sales-filter-renamed --json"],
            "followUpFields": ["dryRun", "mode", "target.handle", "filterPlan.before", "filterPlan.after", "filterPlan.rawIncluded", "changes[].jsonPointer", "changes[].before", "changes[].after", "readbackCommand", "filterReadbackCommand", "ownerReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report filters delete",
            "aliases": ["report filter delete", "report filters remove", "report filter remove"],
            "usage": "powerbi-cli report filters delete --project <project-dir-or.pbip> --handle <filter-handle> (--dry-run | --in-place --confirm <filter-handle> | --out-dir <dir>) [--include-raw] --json",
            "summary": "Delete one existing report, page, or visual PBIR filter by stable handle with guarded output semantics",
            "tags": ["pbir", "report", "filter", "page-filter", "visual-filter", "mutation", "delete", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.filters.deleteMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <filter-handle>", "--dry-run", "--in-place", "--confirm <filter-handle>", "--out-dir <dir>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report filters delete --project build/sales --handle filter:report:main:ReportSegmentFilter --dry-run --json", "powerbi-cli report filters delete --project build/sales --handle filter:page:ReportSectionOverview:PageSegmentFilter --out-dir build/sales-filter-deleted --json"],
            "followUpFields": ["dryRun", "mode", "target.handle", "filterPlan.before", "filterPlan.after", "changes[].jsonPointer", "readbackCommand", "ownerReadbackCommand", "rawReviewCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report filters clear",
            "aliases": ["report filter clear", "report filters reset", "report filter reset"],
            "usage": "powerbi-cli report filters clear --project <project-dir-or.pbip> (--handle <filter-handle> | --scope report | --page <page-name-or-handle> | --visual <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle> | --all) (--dry-run | --in-place --confirm <confirm-token> | --out-dir <dir>) [--include-raw] --json",
            "summary": "Clear existing PBIR filters by exact filter handle, report scope, one page owner, one visual owner, or explicit --all with guarded output semantics",
            "tags": ["pbir", "report", "filter", "page-filter", "visual-filter", "mutation", "clear", "reset", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.filters.clearMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <filter-handle>", "--scope report|page|visual", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--all", "--dry-run", "--in-place", "--confirm <confirm-token>", "--out-dir <dir>", "--include-raw", "--json", "--format json"],
            "limitations": ["No implicit full clear; use --all explicitly.", "--page clears only page-owned filters, not visual filters on that page.", "Does not author new filter expressions or partially edit filter conditions."],
            "examples": ["powerbi-cli report filters clear --project build/sales --page page:ReportSectionOverview --dry-run --json", "powerbi-cli report filters clear --project build/sales --visual <visual-handle> --out-dir build/sales-visual-filters-cleared --json"],
            "followUpFields": ["dryRun", "mode", "selector.kind", "confirmToken", "counts.matchedFilters", "targets[].handle", "filterPlan.before", "filterPlan.after", "filterPlan.arrayEdits", "changes[].jsonPointer", "readbackCommand", "ownerReadbackCommand", "rawReviewCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report slicers list",
            "aliases": ["report slicer list", "report slicers ls"],
            "usage": "powerbi-cli report slicers list --project <project-dir-or.pbip> [--page <page-name-or-handle>] [--include-raw] --json",
            "summary": "List PBIR slicer visuals with stable slicer handles, visual handles, bindings, state summaries, and persisted-value safety warnings",
            "tags": ["pbir", "report", "slicer", "slicers", "visual", "filter", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.slicers.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--page <page-name-or-handle>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report slicers list --project build/sales --json", "powerbi-cli report slicers list --project build/sales --page page:ReportSectionOverview --json"],
            "followUpFields": ["slicers[].handle", "slicers[].visualHandle", "slicers[].target", "slicers[].state", "slicers[].safety", "next"]
        }),
        json!({
            "path": "report slicers show",
            "aliases": ["report slicer show", "report slicers get", "report slicer get"],
            "usage": "powerbi-cli report slicers show --project <project-dir-or.pbip> --handle <slicer-handle> [--no-raw] --json",
            "summary": "Show one PBIR slicer visual by slicer or visual handle, including raw visual state and persisted-value safety metadata",
            "tags": ["pbir", "report", "slicer", "slicers", "visual", "filter", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.slicers.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <slicer-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--include-raw", "--no-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report slicers show --project build/sales --handle slicer:ReportSectionOverview:VisualContainerRegionSlicer --json", "powerbi-cli report slicers show --project build/sales --handle visual:ReportSectionOverview:VisualContainerRegionSlicer --json"],
            "followUpFields": ["slicer.handle", "slicer.visualHandle", "slicer.raw", "slicer.target", "slicer.state", "slicer.safety", "visualReadbackCommand", "validateCommand", "next"]
        }),
        json!({
            "path": "report slicers clear",
            "aliases": ["report slicer clear"],
            "usage": "powerbi-cli report slicers clear --project <project-dir-or.pbip> (--handle <slicer-or-visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) (--dry-run | --in-place --confirm <confirm-token> | --out-dir <dir>) [--include-raw] --json",
            "summary": "Clear persisted PBIR slicer selection/filter state for one slicer visual without changing bindings, layout, or formatting",
            "tags": ["pbir", "report", "slicer", "slicers", "visual", "filter", "mutation", "clear", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.slicers.clearMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <slicer-or-visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--dry-run", "--in-place", "--confirm <confirm-token>", "--out-dir <dir>", "--include-raw", "--json", "--format json"],
            "limitations": ["Clears persisted slicer filter entries matching the slicer binding in known filter arrays only; it does not author new slicer bindings or conditions.", "Rejects non-slicer visuals.", "Does not remove formatting or orientation objects."],
            "examples": ["powerbi-cli report slicers clear --project build/sales --handle slicer:ReportSectionOverview:VisualContainerRegionSlicer --dry-run --json", "powerbi-cli report slicers clear --project build/sales --page page:ReportSectionOverview --visual 'Region Slicer' --out-dir build/sales-slicer-cleared --json"],
            "followUpFields": ["dryRun", "mode", "target.handle", "confirmToken", "counts.clearedFilterEntries", "slicerPlan.beforeState", "slicerPlan.afterState", "slicerPlan.arrayEdits", "changes[].jsonPointer", "changes[].parentJsonPointer", "readbackCommand", "visualReadbackCommand", "rawReviewCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report interactions list",
            "aliases": ["report interaction list", "report interactions ls"],
            "usage": "powerbi-cli report interactions list --project <project-dir-or.pbip> [--page <page-name-or-handle>] [--source <visual-name-or-handle>] [--target <visual-name-or-handle>] [--type Default|DataFilter|HighlightFilter|NoFilter] [--include-raw] --json",
            "summary": "List explicit PBIR page visualInteraction overrides with stable handles, source/target visual resolution, and default-interaction semantics",
            "tags": ["pbir", "report", "interaction", "interactions", "visual", "page", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.interactions.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--page <page-name-or-handle>", "--source <visual-name-or-handle>", "--target <visual-name-or-handle>", "--type Default|DataFilter|HighlightFilter|NoFilter", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report interactions list --project build/sales --json", "powerbi-cli report interactions list --project build/sales --type NoFilter --json"],
            "followUpFields": ["interactions[].handle", "interactions[].source", "interactions[].target", "interactions[].interactionType", "interactions[].semantics", "counts.staleVisualReferences", "next"]
        }),
        json!({
            "path": "report interactions show",
            "aliases": ["report interaction show", "report interactions get", "report interaction get"],
            "usage": "powerbi-cli report interactions show --project <project-dir-or.pbip> --handle <interaction-handle> [--no-raw] --json",
            "summary": "Show one explicit PBIR page visualInteraction override by handle or page/source/target selector",
            "tags": ["pbir", "report", "interaction", "interactions", "visual", "page", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.interactions.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <interaction-handle>", "--page <page-name-or-handle>", "--source <visual-name-or-handle>", "--target <visual-name-or-handle>", "--include-raw", "--no-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report interactions show --project build/sales --handle interaction:ReportSectionOverview:0 --json", "powerbi-cli report interactions show --project build/sales --page page:ReportSectionOverview --source <visual-handle> --target <visual-handle> --json"],
            "followUpFields": ["interaction.handle", "interaction.raw", "interaction.source", "interaction.target", "interaction.semantics", "pageReadbackCommand", "sourceVisualReadbackCommand", "targetVisualReadbackCommand", "validateCommand", "next"]
        }),
        json!({
            "path": "report interactions set",
            "aliases": ["report interaction set", "report interactions update", "report interaction update"],
            "usage": "powerbi-cli report interactions set --project <project-dir-or.pbip> (--handle <interaction-handle> | --page <page-name-or-handle> --source <visual-name-or-handle> --target <visual-name-or-handle>) --type DataFilter|HighlightFilter|NoFilter (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Upsert one explicit PBIR page visualInteraction override for a source/target visual pair; Default authoring remains Desktop-fixture gated",
            "tags": ["pbir", "report", "interaction", "interactions", "visual", "page", "mutation", "authoring", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.interactions.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <interaction-handle>", "--page <page-name-or-handle>", "--source <visual-name-or-handle>", "--target <visual-name-or-handle>", "--type DataFilter|HighlightFilter|NoFilter", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report interactions set --project build/sales --page page:ReportSectionOverview --source <visual-handle> --target <visual-handle> --type HighlightFilter --dry-run --json", "powerbi-cli report interactions set --project build/sales --handle interaction:ReportSectionOverview:0 --type DataFilter --out-dir build/sales-interactions --json"],
            "followUpFields": ["target.handle", "interactionPlan.before", "interactionPlan.after", "interactionPlan.changed", "changes[].path", "readbackCommand", "validateCommand", "next"]
        }),
        json!({
            "path": "report interactions disable",
            "aliases": ["report interaction disable", "report interactions no-filter", "report interaction no-filter"],
            "usage": "powerbi-cli report interactions disable --project <project-dir-or.pbip> (--handle <interaction-handle> | --page <page-name-or-handle> --source <visual-name-or-handle> --target <visual-name-or-handle>) (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Upsert an explicit NoFilter visualInteraction row so the target visual does not react to the source visual",
            "tags": ["pbir", "report", "interaction", "interactions", "visual", "page", "mutation", "disable", "nofilter", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.interactions.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <interaction-handle>", "--page <page-name-or-handle>", "--source <visual-name-or-handle>", "--target <visual-name-or-handle>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report interactions disable --project build/sales --page page:ReportSectionOverview --source <visual-handle> --target <visual-handle> --dry-run --json", "powerbi-cli report interactions disable --project build/sales --handle interaction:ReportSectionOverview:0 --out-dir build/sales-disabled --json"],
            "followUpFields": ["target.handle", "interactionPlan.after.type", "changes[].jsonPointer", "readbackCommand", "validateCommand", "next"]
        }),
        json!({
            "path": "report themes show",
            "aliases": ["report theme show", "report styles show", "report style show", "report themes get"],
            "usage": "powerbi-cli report themes show --project <project-dir-or.pbip> --json",
            "summary": "Show raw report-level theme state, fingerprint, themeCollection, and registered theme JSON resources",
            "tags": ["pbir", "report", "theme", "style", "raw-bundle", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.themes.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--json", "--format json"],
            "examples": ["powerbi-cli report themes show --project build/sales --json"],
            "followUpFields": ["theme.state", "theme.fingerprint", "theme.themeCollection", "theme.registeredThemes", "next"]
        }),
        json!({
            "path": "report themes extract",
            "aliases": ["report theme extract", "report styles extract", "report style extract", "report themes export", "report themes clone"],
            "usage": "powerbi-cli report themes extract --project <source-project-or.pbip> [--out <theme-bundle.json>] --json",
            "summary": "Extract a deterministic raw report theme bundle from themeCollection and already-present registered theme JSON resources",
            "tags": ["pbir", "report", "theme", "style", "extract", "raw-bundle", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "writesArtifactWhenOutProvided": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.themes.extract.v1",
            "flags": ["--project <project-dir-or.pbip>", "--out <theme-bundle.json>", "--out-file <theme-bundle.json>", "--json", "--format json"],
            "examples": ["powerbi-cli report themes extract --project corp/template.pbip --out theme-bundle.json --json"],
            "followUpFields": ["bundle.schema", "bundle.sourceFingerprint", "bundle.themeCollection", "bundle.registeredThemes", "next"]
        }),
        json!({
            "path": "report themes apply",
            "aliases": ["report theme apply", "report styles apply", "report style apply", "report themes import"],
            "usage": "powerbi-cli report themes apply --project <target-project-or.pbip> --bundle <theme-bundle.json> (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Apply a raw report theme bundle by replacing themeCollection and copied registered theme JSON resources; does not copy per-visual formatting",
            "tags": ["pbir", "report", "theme", "style", "mutation", "raw-bundle", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.themes.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--bundle <theme-bundle.json>", "--style-bundle <theme-bundle.json>", "--theme-bundle <theme-bundle.json>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report themes apply --project build/generated --bundle theme-bundle.json --dry-run --json", "powerbi-cli report themes apply --project build/generated --bundle theme-bundle.json --out-dir build/generated-themed --json"],
            "followUpFields": ["dryRun", "source.fingerprint", "changes[].before", "changes[].after", "resourceChanges", "readbackCommand", "validateCommand", "handoffCheckCommand"]
        }),
        json!({
            "path": "report themes presets",
            "aliases": ["report theme presets", "report styles presets", "report style presets", "report themes preset"],
            "usage": "powerbi-cli report themes presets list --json | powerbi-cli report themes presets show --preset <preset-id> [--include-bundle] --json",
            "summary": "List or show built-in report theme presets that apply as registered theme JSON resources",
            "tags": ["pbir", "report", "theme", "style", "preset", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.themes.presets.v1",
            "flags": ["list", "show", "--preset <preset-id>", "--include-bundle", "--json", "--format json"],
            "examples": ["powerbi-cli report themes presets list --json", "powerbi-cli report themes presets show --preset risk-dashboard --include-bundle --json"],
            "followUpFields": ["presets[].id", "presets[].command", "preset.bundle", "next"]
        }),
        json!({
            "path": "report themes apply-preset",
            "aliases": ["report theme apply-preset", "report style apply-preset", "report styles apply-preset", "report themes applyPreset"],
            "usage": "powerbi-cli report themes apply-preset --project <target-project-or.pbip> [--preset risk-dashboard|neutral-ops] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Apply a built-in registered-resource theme preset to a report with guarded output semantics",
            "tags": ["pbir", "report", "theme", "style", "preset", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.themes.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--preset <preset-id>", "--theme <preset-id>", "--style <preset-id>", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report themes apply-preset --project build/sales --preset risk-dashboard --dry-run --json"],
            "followUpFields": ["dryRun", "source.preset", "changes[].before", "changes[].after", "resourceChanges", "readbackCommand", "validateCommand", "handoffCheckCommand"]
        }),
        json!({
            "path": "report style inspect",
            "aliases": ["report style show", "report styles inspect"],
            "usage": "powerbi-cli report style inspect --project <project-dir-or.pbip> --json",
            "summary": "Inspect a combined report style bundle: report themeCollection plus per-visual formatting payload summaries",
            "tags": ["pbir", "report", "style", "theme", "formatting", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.style.inspect.v1",
            "flags": ["--project <project-dir-or.pbip>", "--json", "--format json"],
            "examples": ["powerbi-cli report style inspect --project build/sales --json"],
            "followUpFields": ["summary", "style.themeCollection", "style.visualStyles[].safety", "validation", "next"]
        }),
        json!({
            "path": "report style extract",
            "aliases": ["report styles extract", "report style export"],
            "usage": "powerbi-cli report style extract --project <project-dir-or.pbip> [--out <style-bundle.json>] [--include-literal-text] --json",
            "summary": "Extract a portable master-style bundle containing report themeCollection and per-visual formatting payloads",
            "tags": ["pbir", "report", "style", "theme", "formatting", "bundle", "extract", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "writesArtifactWhenOutProvided": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.style.extract.v1",
            "flags": ["--project <project-dir-or.pbip>", "--out <style-bundle.json>", "--out-file <style-bundle.json>", "--include-literal-text", "--json", "--format json"],
            "examples": ["powerbi-cli report style extract --project corp/template --out master-style.json --json"],
            "followUpFields": ["summary", "bundle.schema", "bundle.policy", "bundle.visualStyles[].formatting", "bundle.visualStyles[].safety", "next"]
        }),
        json!({
            "path": "report style apply",
            "aliases": ["report styles apply", "report style import"],
            "usage": "powerbi-cli report style apply --project <target-project-or.pbip> --bundle <style-bundle.json> [--allow-literal-text] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Apply a master-style bundle by replacing report themeCollection and matching visual formatting payloads by visualType+ordinal",
            "tags": ["pbir", "report", "style", "theme", "formatting", "bundle", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.style.apply.v1",
            "flags": ["--project <project-dir-or.pbip>", "--bundle <style-bundle.json>", "--allow-literal-text", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "limitations": ["Matches visual styles by visualType plus ordinal, not semantic intent.", "Does not copy bindings/data queries.", "Literal text in styles requires explicit --allow-literal-text."],
            "examples": ["powerbi-cli report style apply --project build/generated --bundle master-style.json --dry-run --json"],
            "followUpFields": ["dryRun", "mode", "summary", "applied", "skipped", "changes", "readbackCommand", "validateCommand", "handoffCheckCommand"]
        }),
        json!({
            "path": "report style diff",
            "usage": "powerbi-cli report style diff <before-style.json> <after-style.json> --json",
            "summary": "Compare two extracted report style bundles by fingerprint, themeCollection, and visual-style counts",
            "tags": ["pbir", "report", "style", "diff", "bundle", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.style.diff.v1",
            "flags": ["<before-style.json>", "<after-style.json>", "--json", "--format json"],
            "examples": ["powerbi-cli report style diff old-style.json new-style.json --json"],
            "followUpFields": ["left", "right", "diff.sameFingerprint", "diff.themeCollectionChanged", "diff.visualStyleCountDelta"]
        }),
        json!({
            "path": "report visuals list",
            "usage": "powerbi-cli report visuals list --project <project-dir-or.pbip> [--page <page-name-or-handle>] --json",
            "summary": "List PBIR report visuals with stable handles, page context, geometry, and binding counts",
            "tags": ["pbir", "report", "visual", "layout", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "reportVisualsList.v1",
            "flags": ["--project <project-dir-or.pbip>", "--page <page-name-or-handle>", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals list --project build/sales --json", "powerbi-cli report visuals list --project build/sales --page page:ReportSectionOverview --json"],
            "followUpFields": ["visuals[].handle", "visuals[].page.handle", "visuals[].position", "next"]
        }),
        json!({
            "path": "report visuals show",
            "usage": "powerbi-cli report visuals show --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) --json",
            "summary": "Show one PBIR visual with page context, geometry, type, and field bindings",
            "tags": ["pbir", "report", "visual", "layout", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "reportVisualsShow.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals show --project build/sales --handle visual:ReportSectionOverview:VisualContainerSalesKpi --json"],
            "followUpFields": ["visual.handle", "visual.position", "visual.bindings", "next"]
        }),
        json!({
            "path": "report visuals formatting list",
            "aliases": ["report visuals format list"],
            "usage": "powerbi-cli report visuals formatting list --project <project-dir-or.pbip> [--page <page-name-or-handle>] [--include-raw] --json",
            "summary": "Inventory per-visual PBIR formatting object containers and property names without raw formatting payloads by default",
            "tags": ["pbir", "report", "visual", "formatting", "style", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.formatting.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--page <page-name-or-handle>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals formatting list --project build/sales --json", "powerbi-cli report visuals format list --project build/sales --page page:ReportSectionOverview --json"],
            "followUpFields": ["visuals[].handle", "visuals[].formatting.objectNames", "visuals[].formatting.containers[].propertyNames", "counts.formatProperties", "next"]
        }),
        json!({
            "path": "report visuals formatting show",
            "aliases": ["report visuals format show", "report visuals formatting get", "report visuals format get"],
            "usage": "powerbi-cli report visuals formatting show --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) [--include-raw] --json",
            "summary": "Show one visual's PBIR formatting object inventory; raw PBIR objects require explicit --include-raw",
            "tags": ["pbir", "report", "visual", "formatting", "style", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.formatting.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals formatting show --project build/sales --handle visual:ReportSectionOverview:VisualContainerSalesKpi --json", "powerbi-cli report visuals format show --project build/sales --handle <visual-handle> --include-raw --json"],
            "followUpFields": ["visual.handle", "formatting.objectNames", "formatting.containers[].propertyNames", "formatting.safety.rawIncluded", "next"]
        }),
        json!({
            "path": "report visuals formatting conditional-formatting list",
            "aliases": ["report visuals formatting cf list", "report visuals formatting conditional list"],
            "usage": "powerbi-cli report visuals formatting conditional-formatting list --project <project-dir-or.pbip> [--page <page-name-or-handle>] [--include-raw] --json",
            "summary": "Inventory conditional-formatting/rule/gradient PBIR signals across visuals without authoring new rules",
            "tags": ["pbir", "report", "visual", "formatting", "conditional-formatting", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.conditionalFormatting.list.v1",
            "flags": ["--project <project-dir-or.pbip>", "--page <page-name-or-handle>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals formatting conditional-formatting list --project build/sales --json"],
            "limitations": ["Readback/static scan only; conditional formatting authoring is Desktop-fixture gated."],
            "followUpFields": ["counts.conditionalFormattingSignals", "visuals[].conditionalFormatting.signalTypes", "visuals[].conditionalFormatting.signals", "contract", "next"]
        }),
        json!({
            "path": "report visuals formatting conditional-formatting show",
            "aliases": ["report visuals formatting cf show", "report visuals formatting conditional show"],
            "usage": "powerbi-cli report visuals formatting conditional-formatting show --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) [--include-raw] --json",
            "summary": "Show conditional-formatting/rule/gradient PBIR signals for one visual",
            "tags": ["pbir", "report", "visual", "formatting", "conditional-formatting", "readback", "agent"],
            "readOnly": true,
            "mutates": false,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.conditionalFormatting.show.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--include-raw", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals formatting conditional-formatting show --project build/sales --handle <visual-handle> --include-raw --json"],
            "limitations": ["Readback/static scan only; conditional formatting authoring is Desktop-fixture gated."],
            "followUpFields": ["visual.handle", "conditionalFormatting.signalCount", "conditionalFormatting.signals[].pointer", "contract", "next"]
        }),
        json!({
            "path": "report visuals formatting extract",
            "aliases": ["report visuals format extract", "report visuals formatting export", "report visuals format export"],
            "usage": "powerbi-cli report visuals formatting extract --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) [--out <formatting-bundle.json>] --json",
            "summary": "Extract one visual's raw PBIR formatting objects into an auditable bundle for style portability",
            "tags": ["pbir", "report", "visual", "formatting", "style", "bundle", "extract", "agent"],
            "readOnly": false,
            "mutates": true,
            "mutatesProject": false,
            "writesArtifactWhenOutProvided": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.formatting.extract.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--out <formatting-bundle.json>", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals formatting extract --project corp/template --handle <visual-handle> --out visual-formatting-bundle.json --json"],
            "followUpFields": ["bundle.schema", "bundle.formatting.visualObjects", "bundle.formatting.topLevelObjects", "bundle.summary", "bundle.safety", "next"]
        }),
        json!({
            "path": "report visuals formatting apply",
            "aliases": ["report visuals format apply", "report visuals formatting import", "report visuals format import"],
            "usage": "powerbi-cli report visuals formatting apply --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) --bundle <formatting-bundle.json> [--allow-literal-text] [--allow-cross-type] [--include-raw] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Apply a visual formatting bundle by replacing only /visual/objects and /objects on the target visual",
            "tags": ["pbir", "report", "visual", "formatting", "style", "bundle", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.formatting.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--bundle <formatting-bundle.json>", "--allow-literal-text", "--allow-cross-type", "--include-raw", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals formatting apply --project build/generated --handle <visual-handle> --bundle visual-formatting-bundle.json --dry-run --json", "powerbi-cli report visuals format apply --project build/generated --handle <visual-handle> --bundle visual-formatting-bundle.json --allow-literal-text --out-dir build/generated-styled --json"],
            "followUpFields": ["dryRun", "mode", "source.fingerprint", "target.handle", "formattingPlan.before", "formattingPlan.after", "changes[].jsonPointers", "changes[].before", "changes[].after", "readbackCommand", "rawReviewCommand", "visualReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report visuals formatting set-text",
            "aliases": ["report visuals format set-text", "report visuals formatting title", "report visuals format title", "report visuals formatting set-title"],
            "usage": "powerbi-cli report visuals formatting set-text --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) [--title <text>] [--show-title true|false] [--clear-alt-text] [--include-raw] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Patch typed PBIR visual title visibility/text or remove validator-rejected alt-text metadata without replacing sibling formatting objects",
            "tags": ["pbir", "report", "visual", "formatting", "title", "alt-text", "accessibility", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.formatting.textMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--title <text>", "--show-title true|false", "--clear-alt-text", "--include-raw", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals formatting set-text --project build/sales --handle <visual-handle> --title \"Revenue Overview\" --dry-run --json", "powerbi-cli report visuals format title --project build/sales --handle <visual-handle> --clear-alt-text --out-dir build/sales-clean --json"],
            "followUpFields": ["dryRun", "mode", "target.handle", "textPlan.requested", "textPlan.before", "textPlan.after", "changes[].jsonPointers", "changes[].before", "changes[].after", "readbackCommand", "rawReviewCommand", "visualReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
            "limitations": ["Patches title properties in the existing /visual/visualContainerObjects/title and /visual/objects/title containers, keeps an existing powerbi-cli.placeholderTitle annotation synchronized, and removes altText from both known general containers while preserving sibling properties. --alt-text authoring returns unsupported_feature because Microsoft powerbi-report-authoring-cli v0.1.4 rejects both known placements. Other typed formatting properties remain bundle- or Desktop-fixture gated."]
        }),
        json!({
            "path": "report visuals formatting set-color",
            "aliases": ["report visuals format set-color", "report visuals formatting color", "report visuals format color", "report visuals formatting set-colour", "report visuals format colour"],
            "usage": "powerbi-cli report visuals formatting set-color --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) (--slot title.fontColor|dataPoint.fill --color <#RRGGBB|#AARRGGBB> | --title-font-color <hex> | --data-point-fill <hex>) [--include-raw] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Patch static PBIR visual title font color or wildcard data point fill without replacing other formatting objects",
            "tags": ["pbir", "report", "visual", "formatting", "color", "colour", "title", "data-point", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.formatting.colorMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--slot title.fontColor|dataPoint.fill", "--color <hex>", "--title-font-color <hex>", "--title-font-colour <hex>", "--data-point-fill <hex>", "--fill-color <hex>", "--include-raw", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals formatting set-color --project build/sales --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json", "powerbi-cli report visuals format color --project build/sales --handle <visual-handle> --data-point-fill '#E87722' --out-dir build/sales-colored --json"],
            "followUpFields": ["dryRun", "mode", "target.handle", "colorPlan.requested", "colorPlan.before", "colorPlan.after", "changes[].jsonPointers", "changes[].before", "changes[].after", "readbackCommand", "rawReviewCommand", "visualReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"],
            "supportedSlots": ["title.fontColor", "dataPoint.fill"],
            "limitations": ["Patches only static literal colors at /visual/objects/title/0/properties/fontColor and /visual/objects/dataPoint/0/properties/fill.", "dataPoint.fill is refused when the existing formatting card has data-bound selectors; conditional formatting remains Desktop-fixture gated."]
        }),
        json!({
            "path": "report visuals catalog",
            "aliases": ["report visuals types", "report visuals visual-types"],
            "usage": "powerbi-cli report visuals catalog [--visual-type <type-or-alias>] --json",
            "summary": "Return generated visual types, aliases, binding roles, template-only visual types, and planned visual families for agent-safe report authoring",
            "tags": ["pbir", "report", "visual", "catalog", "chart", "binding", "pie", "donut", "matrix", "slicer", "agent"],
            "readOnly": true,
            "mutates": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.catalog.v1",
            "flags": ["--visual-type <type-or-alias>", "--type <type-or-alias>", "--json", "--format json"],
            "supportedVisualTypes": supported_visual_type_names(),
            "examples": ["powerbi-cli report visuals catalog --json", "powerbi-cli report visuals catalog --visual-type line --json"],
            "limitations": ["Raw columns are supported only in proven categorical/detail roles and table value lists; card Values, chart Y, matrix Values, and scatter X/Y/Size require measures.", "A model field may appear only once per generated visual until Desktop-authored duplicate queryRef numbering is available."],
            "followUpFields": ["supportedVisualTypes", "visualTypes[].proofLevel", "visualTypes[].bindingProofLevel", "visualTypes[].proofNote", "visualTypes[].roles", "templateOnlyVisualTypes", "plannedVisualTypes", "next"]
        }),
        json!({
            "path": "report visuals add",
            "aliases": ["report visuals create"],
            "usage": "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-name-or-handle> --title <title> [--visual-type <type>] [--mode basic|dropdown|between] [--name <visual-name>] [--x <n>] [--y <n>] [--width <n>] [--height <n>] [--z <n>] [--tab-order <n>] (--binding <key=value,...> | --bindings-json <json> | --bindings-file <file>) [--allow-outside-page] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Create a PBIR visual container on an existing page using the same minimal generated patterns as scaffold",
            "tags": ["pbir", "report", "visual", "layout", "binding", "pie", "donut", "matrix", "slicer", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "desktop-golden-pending",
            "outputSchema": "powerbi-cli.report.visuals.mutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--page <page-name-or-handle>", "--title <title>", "--visual-type <type>", "--type <type>", "--mode basic|dropdown|between", "--name <visual-name>", "--x <n>", "--y <n>", "--width <n>", "--height <n>", "--z <n>", "--tab-order <n>", "--binding <key=value,...>", "--bindings-json <json>", "--bindings-file <file>", "--allow-outside-page", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "supportedVisualTypes": supported_visual_type_names(),
            "examples": ["powerbi-cli report visuals add --project build/sales --page page:ReportSectionOverview --title \"Revenue Card\" --binding \"role=Values,table=FactSales,measure=Total Revenue\" --dry-run --json", "powerbi-cli report visuals create --project build/sales --page page:ReportSectionOverview --title \"Scratch Card\" --binding \"role=Values,table=FactSales,measure=Total Revenue\" --out-dir build/sales-visual --json"],
            "limitations": ["Generated --title emits a literal title with show=true under /visual/visualContainerObjects/title and keeps annotation readback metadata. Generated visuals omit validator-rejected general.altText; the Desktop-authored title shape and schema goldens exist, but the changed generated bytes await Desktop open/refresh/save re-verification.", "Raw columns in measure/value roles and repeated use of one field return unsupported_feature instead of guessed PBIR."],
            "followUpFields": ["dryRun", "target.handle", "visualPlan.after", "bindingPlan.after", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report visuals clone",
            "aliases": ["report visuals duplicate", "report visuals copy"],
            "usage": "powerbi-cli report visuals clone --project <project-dir-or.pbip> (--handle <source-visual-handle> | --from-page <page-name-or-handle> --visual <visual-name-or-handle>) [--target-page <page-name-or-handle>] [--name <new-visual-name>] [--title <title>] [--x <n>] [--y <n>] [--width <n>] [--height <n>] [--z <n>] [--tab-order <n>] [--allow-outside-page] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Clone one simple PBIR visual container by copying visual.json and patching only name, position, and clone annotations",
            "tags": ["pbir", "report", "visual", "clone", "duplicate", "template", "layout", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.cloneMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <source-visual-handle>", "--from-page <page-name-or-handle>", "--source-page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--target-page <page-name-or-handle>", "--to-page <page-name-or-handle>", "--page <target-page-or-source-page>", "--name <new-visual-name>", "--title <title>", "--x <n>", "--y <n>", "--width <n>", "--height <n>", "--z <n>", "--tab-order <n>", "--allow-outside-page", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "limitations": ["First slice copies only simple visual containers whose directory contains visual.json and no sidecars.", "Clone preserves raw PBIR inside visual.json; use set-bindings or formatting apply for subsequent edits."],
            "examples": ["powerbi-cli report visuals clone --project build/sales --handle <visual-handle> --title \"Revenue Copy\" --dry-run --json", "powerbi-cli report visuals duplicate --project build/sales --handle <visual-handle> --target-page page:ReportSectionOverview --out-dir build/sales-cloned --json"],
            "followUpFields": ["dryRun", "mode", "source.handle", "target.handle", "clonePlan.sourcePath", "clonePlan.targetPath", "clonePlan.position.before", "clonePlan.position.after", "changes[].path", "changes[].after", "readbackCommand", "slicerReadbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report visuals delete",
            "aliases": ["report visuals remove"],
            "usage": "powerbi-cli report visuals delete --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) (--dry-run | --in-place --confirm <visual-handle> | --out-dir <dir>) --json",
            "summary": "Delete one PBIR visual container directory after proving it contains only visual.json; in-place delete requires exact handle confirmation",
            "tags": ["pbir", "report", "visual", "layout", "mutation", "delete", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "confirmRequiredForInPlace": true,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.deleteMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--dry-run", "--in-place", "--confirm <visual-handle>", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals delete --project build/sales --handle <visual-handle> --dry-run --json", "powerbi-cli report visuals delete --project build/sales --handle <visual-handle> --out-dir build/sales-without-visual --json"],
            "followUpFields": ["dryRun", "mode", "target.handle", "target.page.handle", "deletePlan.before", "deletePlan.after", "changes[].kind", "changes[].action", "changes[].path", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report visuals set-position",
            "usage": "powerbi-cli report visuals set-position --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) [--x <n>] [--y <n>] [--width <n>] [--height <n>] [--z <n>] [--tab-order <n>] [--allow-outside-page] (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Patch only a PBIR visual position object with guarded output semantics",
            "tags": ["pbir", "report", "visual", "layout", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.positionMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--x <n>", "--y <n>", "--width <n>", "--height <n>", "--z <n>", "--tab-order <n>", "--allow-outside-page", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals set-position --project build/sales --handle <visual-handle> --x 40 --y 40 --width 320 --height 180 --dry-run --json"],
            "followUpFields": ["dryRun", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
        json!({
            "path": "report visuals set-bindings",
            "aliases": ["report visuals bind"],
            "usage": "powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) (--binding <key=value,...> | --bindings-json <json> | --bindings-file <file> | --clear-bindings) (--dry-run | --in-place | --out-dir <dir>) --json",
            "summary": "Replace or clear PBIR field-well bindings for an existing visual using canonical TMDL table, column, and measure names",
            "tags": ["pbir", "report", "visual", "binding", "queryState", "mutation", "agent"],
            "readOnly": false,
            "mutates": true,
            "requiresOutput": true,
            "writesDataCache": false,
            "stability": "alpha-output",
            "proofLevel": "unit-smoke",
            "outputSchema": "powerbi-cli.report.visuals.bindingMutation.v1",
            "flags": ["--project <project-dir-or.pbip>", "--handle <visual-handle>", "--page <page-name-or-handle>", "--visual <visual-name-or-handle>", "--binding <key=value,...>", "--bindings-json <json>", "--bindings-file <file>", "--clear-bindings", "--dry-run", "--in-place", "--out-dir <dir>", "--json", "--format json"],
            "examples": ["powerbi-cli report visuals set-bindings --project build/sales --handle <visual-handle> --bindings-json '[{\"role\":\"Values\",\"table\":\"FactSales\",\"measure\":\"Total Revenue\"}]' --dry-run --json", "powerbi-cli report visuals set-bindings --project build/sales --handle <visual-handle> --clear-bindings --dry-run --json"],
            "limitations": ["Raw columns in card Values, chart Y, matrix Values, and scatter X/Y/Size roles return unsupported_feature pending aggregation-binding proof.", "Repeated use of one model field returns unsupported_feature pending Desktop-authored duplicate queryRef numbering."],
            "followUpFields": ["dryRun", "bindingPlan.before", "bindingPlan.after", "changes[].before", "changes[].after", "readbackCommand", "wireframeCommand", "inspectCommand", "validateCommand"]
        }),
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
    ]
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
        "columnFields": ["name", "dataType", "description", "formatString", "sourceColumn", "isHidden", "isKey", "summarizeBy"],
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
        "partitionSourceKinds": ["dummyMTable", "sqlDatabase", "postgresqlDatabase", "odbcDataSource", "webContents", "externalFile", "unknown", "missing"],
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
        "dashboardSpecFields": ["schema", "report.name", "report.displayName", "report.audience", "report.questions", "model.measures", "pages[].id", "pages[].displayName", "pages[].size", "pages[].visuals", "pages[].visuals[].type", "pages[].visuals[].bindings", "pages[].visuals[].bindings[].field"],
        "reportSpecFieldsInventoryFields": ["ok", "exitCode", "supportedVisualTypes", "tables[].name", "tables[].profileRole", "tables[].rowCount", "tables[].columns[].reference", "tables[].columns[].roles", "tables[].columns[].structuredBinding", "tables[].measures[].reference", "tables[].measures[].structuredBinding", "fields[].reference", "examples", "next"],
        "reportBuildFields": ["ok", "changed", "dryRun", "projectDir", "inputs", "compiled.counts", "changes[].kind", "changes[].action", "changes[].path", "changes[].before", "changes[].after", "profileSummary", "executedPrimitives", "operations", "warnings", "inspectCommand", "validateCommand", "handoffCheckCommand", "fixtureNormalizeCommand", "desktopOpenCheckCommand", "proof", "next"],
        "desktopOpenFields": ["ok", "exitCode", "document", "session.state", "session.owned", "session.desktopProcessId", "session.desktopProcessCreationTimeUtc", "session.desktopExecutablePath", "session.receiptPath", "session.cleanupCommand", "session.priorSessionCleanup", "oracle", "validation", "proof", "diagnostics", "next"],
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
        "visualBindingFields": ["role", "table", "column", "measure", "displayName", "formatString"],
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
        "summary": "Generated dashboard specs and report visuals add use this small visual role contract. Pie, donut, matrix, and slicer bindings retain manual Desktop canvas/refresh evidence recorded in testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json. Current title-bearing generated bytes are desktop-golden-pending until open/refresh/save re-verification. Automated desktop-canvas-refresh proof and broader formatting coverage remain open. Use clone/template workflows for visuals outside the catalog.",
        "supportedVisualTypes": supported_visual_type_names(),
        "visualTypes": visual_type_contracts(),
        "desktopGoldenPendingVisualTypes": supported_visual_type_names(),
        "bindingManualDesktopCanvasRefreshVisualTypes": ["pieChart", "donutChart", "pivotTable", "slicer"],
        "slicerModes": ["Basic", "Dropdown", "Between"],
        "bindingFields": ["role", "field", "table", "column", "measure", "displayName", "formatString"],
        "bindingRules": [
            "Prefer structured bindings with table plus column or measure to avoid ambiguity.",
            "Legacy field strings use Table[Name] and fail when a column and measure share the same name.",
            "Category, Series (including scatter color grouping), Rows, Columns, scatter Category, and slicer Values bindings must resolve to columns.",
            "Card Values, chart Y, matrix Values, and scatter X/Y/Size require measures; table Values and Tooltips may resolve to columns or measures.",
            "One model field may appear only once per visual until Desktop-authored duplicate queryRef numbering is available.",
            "Pie and donut require exactly one Category plus one or more Y measures and emit a default descending sort by the first Y binding.",
            "Slicer mode is Basic by default; Basic, Dropdown, and Between are generated, and generated slicers contain no persisted selection filter. Between is intended for numeric or date range columns."
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
