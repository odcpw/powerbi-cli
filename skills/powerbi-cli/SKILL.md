---
name: powerbi-cli
description: >-
  Use powerbi-cli to scaffold, inspect, validate, and iteratively author
  offline-safe Power BI PBIP/PBIR/TMDL projects for agent-driven dashboard
  work. Use for schema-first Power BI report authoring, dummy-data handoff,
  semantic model metadata, report pages, visuals, measures, relationships,
  themes, validation, and Desktop oracle proof.
---

# Power BI CLI Workbench

Use this skill when an agent needs to create or edit a Power BI dashboard
project through `powerbi-cli`. Treat the local binary as the source of truth.
This skill gives the operating loop; `powerbi-cli --json capabilities` gives the
live command contract.

```text
build or resolve powerbi-cli
-> read focused capabilities
-> validate schema/profile/spec inputs
-> build, scaffold, or inspect a PBIP project
-> use CLI-returned handles and commands
-> mutate with explicit outputs or dry-runs
-> validate, inspect, handoff-check, and Desktop-proof when available
```

## Product State

- Rust is the product path.
- Core commands must run on Windows, Linux, and macOS.
- Power BI Desktop is a Windows-only compatibility oracle, not a dependency for
  offline authoring.
- The CLI authors PBIP/PBIR/TMDL folders, not `.pbix` or `.pbit` binaries.
  Package commands can inspect/extract/import safe metadata/source entries from
  PBIX/PBIT archives, and `source-pack` writes only a scanned strict allowlist.
  On Windows, `model live export-tmdl` can read the semantic model of one exact
  already-open PBIP/PBIX document through the pinned local Microsoft Modeling
  MCP and publish a guarded TMDL-only export; it does not export report pages or
  claim full PBIX-to-PBIP conversion. Binary export is a Desktop handoff.
- Generated home/offline projects must not contain credentials, real exported
  data, `.pbi/cache.abf`, `localSettings.json`, `.pbix`, or `.pbit`.
- Dummy Power Query M partitions preserve schema shape until the work machine
  rebinds to real corporate sources.
- If docs, memory, and live capabilities disagree, trust the freshly built
  binary and its `capabilities` output.
- Before changing PBIR report definitions, filters, visuals, or Desktop proof
  logic, read `docs/pbir-desktop-oracle.md`. It records Desktop-discovered PBIR
  constraints, source links, proof commands, and the current implementation
  backlog.
- For report repair or multi-page dashboard work, also read
  `references/desktop-runtime-regression.md`. It captures the shortest
  source-mirroring, DAX, scatter, selector, and live-Desktop regression loop.
- Implementation must stay modular. Do not add new command families to
  `src/main.rs`; use focused modules for CLI dispatch, live contract, schema
  manifests, PBIR, TMDL, project validation, Desktop oracle proof, and future
  mutation kernels.

## Cold Start

Inside the repo, build or run the local Rust binary. Do not rely on a stale
installed `powerbi-cli` found on `PATH`.

PowerShell:

```powershell
$env:CARGO_TARGET_DIR = "$env:TEMP\powerbi-cli-target"
cargo build --bin powerbi-cli
$targetDir = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
$env:POWERBI_CLI_BIN = Join-Path $targetDir "debug\powerbi-cli.exe"
function pbi { & $env:POWERBI_CLI_BIN @args }
pbi --json capabilities
pbi features list --json
pbi --json doctor
```

Bash:

```bash
export CARGO_TARGET_DIR="${TMPDIR:-/tmp}/powerbi-cli-target"
cargo build --bin powerbi-cli
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
POWERBI_CLI_BIN="$TARGET_DIR/debug/powerbi-cli"
pbi() { "$POWERBI_CLI_BIN" "$@"; }
pbi --json capabilities
pbi --json doctor
```

## Discovery

<!-- powerbi-cli:commands:start -->
### Commands (generated from `capabilities --json`)

This list is generated; edit the live command catalog in `src/contract/` rather than this region.

- `powerbi-cli --robot-triage` — Return quick reference, recommended next steps, health, and command catalog in one call _(proof: `unit-smoke`)_
- `powerbi-cli --json capabilities [--for <filter> [--compact]]` — List the agent-facing command contract; focused queries omit unrelated large catalogs and exact compact queries return one minimal command record _(proof: `unit-smoke`)_
- `powerbi-cli desktop bridge reload --project <project-dir-or.pbip> --pid <pid> --json` — Reload the report definition only after exact canonical project/PID identity and a clean saved Desktop state are proven _(proof: `unit-smoke`)_
- `powerbi-cli desktop bridge screenshot-all --project <project-dir-or.pbip> --pid <pid> --out-dir <new-dir> --json` — Capture the exact bounded Desktop status page inventory through the pinned Bridge into a new guarded directory _(proof: `unit-smoke`)_
- `powerbi-cli desktop bridge screenshot-page --project <project-dir-or.pbip> --pid <pid> --page <id> --out <new.png> --json` — Capture one exact inventoried PBIR page through the pinned Desktop Bridge into a new guarded PNG evidence file _(proof: `unit-smoke`)_
- `powerbi-cli desktop bridge status [--pid <pid>] --json` — Inspect pinned Microsoft Desktop Bridge instances and their exact current-file, dirty-state, and PBIR page inventory _(proof: `unit-smoke`)_
- `powerbi-cli desktop canvas-check <project-dir-or.pbip-or.pbix> --page <page> --expect <values.json> [--timeout-ms <ms>] [--desktop-path <PBIDesktop.exe>] [--enable-oracle] --json` — Planned Windows Desktop canvas proof that will assert expected values and reject a blank rendered page _(proof: `unit-smoke`)_
- `powerbi-cli desktop close --json` — Idempotently close only the exact CLI-owned Power BI Desktop session and its verified descendants _(proof: `unit-smoke`)_
- `powerbi-cli desktop harvest-reference --project <saved.pbip> --visual <visual:<page>:<name>|page:<name>|report:main> --out docs/reference/desktop-authored-visuals/<name>.json [--desktop-version <version>] [--license-note <text>] [--dry-run] --json` — Archive one Desktop-saved visual, page, or report fragment with source fingerprint, date, license note, and honest pending proof _(proof: `desktop-golden-pending`)_
- `powerbi-cli desktop open <project-dir-or.pbip-or.pbix> [--preflight strict|normal|skip] [--timeout-ms <ms>] [--desktop-path <PBIDesktop.exe>] [--enable-oracle] --json` — Open the single CLI-owned interactive Power BI Desktop session, closing a prior owned session first _(proof: `unit-smoke`)_
- `powerbi-cli desktop open-check <project-dir-or.pbip-or.pbix> [--timeout-ms <ms>] [--desktop-path <PBIDesktop.exe>] [--enable-oracle] --json` — Attempt one-shot Power BI Desktop launch plus exact project-title observation, then clean up _(proof: `unit-smoke`)_
- `powerbi-cli desktop refresh-check <project-dir-or.pbip-or.pbix> [--timeout-ms <ms>] [--desktop-path <PBIDesktop.exe>] [--enable-oracle] --json` — Planned Windows Desktop refresh proof that will verify dummy-partition refresh and issue-dialog absence _(proof: `unit-smoke`)_
- `powerbi-cli desktop screenshot <project-dir-or.pbip-or.pbix> --out <file.png> [--timeout-ms <ms>] [--desktop-path <PBIDesktop.exe>] [--allow-unverified-capture] [--enable-oracle] --json` — Capture the primary display after exact Desktop title and foreground-PID verification for manual or agent review _(proof: `unit-smoke`)_
- `powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> [--scope model.tables|model.columns|model.measures|model.calculatedColumns|model.relationships] --json` — Compare two PBIP projects using normalized semantic summaries and stable handles _(proof: `unit-smoke`)_
- `powerbi-cli --json doctor` — Report local Power BI Desktop detection and format assumptions _(proof: `unit-smoke`)_
- `powerbi-cli features list [--for <feature-filter>] --json` — List supported, fixture-gated, planned, and explicitly refused Power BI feature surfaces _(proof: `unit-smoke`)_
- `powerbi-cli fixture normalize <project-dir-or.pbip> [--out <summary.json>] --json` — Emit a deterministic path-free summary for generated or Desktop-authored PBIP golden fixtures _(proof: `unit-smoke`)_
- `powerbi-cli fixture verify <project-dir-or.pbip> --expected <summary.json> [--write-actual <path>] --json` — Compare a project against a committed normalized fixture summary, returning the actual JSON and pointer differences without writing by default _(proof: `unit-smoke`)_
- `powerbi-cli guid [--count <1..100>] --json` — Generate lowercase UUIDv4 values for TMDL lineageTag authoring when hand-adding columns or measures _(proof: `unit-smoke`)_
- `powerbi-cli handoff check <project-dir-or.pbip> [--target offline|work] --json` — Classify an offline/dummy or work-network/live-source PBIP handoff after partition-shape, credential, PII-suspect text, cache, binary, and embedded-data checks _(proof: `unit-smoke`)_
- `powerbi-cli handoff rebind-check <project-dir-or.pbip> [--project <project-dir-or.pbip>] [--table <table>] [--partition <partition-handle-or-name>] --json` — Verify every selected partition resolves to a materialized credential-free source without opening a connection _(proof: `unit-smoke`)_
- `powerbi-cli handoff rebind-plan <project-dir-or.pbip> [--project <project-dir-or.pbip>] [--templates <source-templates.json|->] [--table <table>] [--partition <partition-handle>] [--allow-unmapped] [--out <file.md>] [--force] --json` — Generate a redacted work-machine rebind plan and suppress runbook materialization when a template or partition contains credentials _(proof: `unit-smoke`)_
- `powerbi-cli --json inspect [--deep] <project-dir-or.pbip>` — Summarize a PBIP project and, with --deep, return stable handles for report/model objects _(proof: `unit-smoke`)_
- `powerbi-cli integrations install --allow-network --json` — Install and atomically activate the committed exact Microsoft Power BI npm graph _(proof: `unit-smoke`)_
- `powerbi-cli integrations status [--deep] [--component modeling-mcp|report-authoring|desktop-bridge] --json` — Inspect the exact optional Microsoft Power BI toolchain without installation or registry access _(proof: `unit-smoke`)_
- `powerbi-cli lint (<project-dir-or.pbip> | --rules | --explain <rule-id>) --json` — Run typed PBIP/PBIR/TMDL quality checks or inspect the canonical lint and audit rule registry _(proof: `unit-smoke`)_
- `powerbi-cli model advanced inventory --project <project-dir-or.pbip> --json` — Inventory advanced TMDL folders for roles, perspectives, cultures, and named expressions _(proof: `unit-smoke`)_
- `powerbi-cli model calculated-columns add --project <project-dir-or.pbip> --table <table> --name <column> (--expression <dax> | --expression-file <path|->) --data-type <type> [--format-string <fmt>] [--summarize-by <mode>] [--display-folder <folder>] [--description <text>] [--hidden] (--dry-run | --in-place | --out-dir <dir>) --json` — Add a DAX calculated column to a TMDL table with guarded output semantics _(proof: `unit-smoke`)_
- `powerbi-cli model calculated-columns delete --project <project-dir-or.pbip> (--handle <column-handle> | --table <table> --name <column>) (--dry-run | --in-place --confirm <column-handle> | --out-dir <dir>) --json` — Delete a DAX calculated column; in-place delete requires exact handle confirmation _(proof: `unit-smoke`)_
- `powerbi-cli model calculated-columns list --project <project-dir-or.pbip> [--table <table>] --json` — List semantic model DAX calculated columns with stable column handles _(proof: `unit-smoke`)_
- `powerbi-cli model calculated-columns show --project <project-dir-or.pbip> (--handle <column-handle> | --table <table> --name <column>) --json` — Show one semantic model DAX calculated column and its TMDL block _(proof: `unit-smoke`)_
- `powerbi-cli model calculated-columns update --project <project-dir-or.pbip> (--handle <column-handle> | --table <table> --name <column>) [--expression <dax> | --expression-file <path|->] [--data-type <type>] [--format-string <fmt>] [--summarize-by <mode>] [--display-folder <folder>] [--description <text>] [--hidden|--visible] (--dry-run | --in-place | --out-dir <dir>) --json` — Update a DAX calculated column expression or metadata; refuses unsupported Desktop-authored TMDL lines _(proof: `unit-smoke`)_
- `powerbi-cli model columns add --project <project-dir-or.pbip> --table <table> --name <column> [--expression <dax> | --expression-file <path|->] [--data-type <type>] [--source-column <column>] [--format-string <fmt>] [--summarize-by <mode>] [--sort-by <column>] [--display-folder <folder>] [--description <text>] [--hidden] [--key] (--dry-run | --in-place | --out-dir <dir>) --json` — Add a base or calculated column to a TMDL table _(proof: `unit-smoke`)_
- `powerbi-cli model columns delete --project <project-dir-or.pbip> (--handle <column-handle> | --table <table> --name <column>) (--dry-run | --in-place --confirm <column-handle> | --out-dir <dir>) --json` — Delete a semantic-model column with guarded in-place confirmation _(proof: `unit-smoke`)_
- `powerbi-cli model columns list --project <project-dir-or.pbip> [--table <table>] --json` — List base and calculated semantic-model columns with stable handles _(proof: `unit-smoke`)_
- `powerbi-cli model columns set-sort-by --project <project-dir-or.pbip> --table <table> --column <column> (--by <sort-column> | --clear) (--dry-run | --in-place | --out-dir <dir>) --json` — Set or remove one column's same-table TMDL sortByColumn property with guarded output semantics _(proof: `unit-smoke`)_
- `powerbi-cli model columns show --project <project-dir-or.pbip> (--handle <column-handle> | --table <table> --name <column>) --json` — Show one base or calculated column and its raw TMDL block _(proof: `unit-smoke`)_
- `powerbi-cli model columns update --project <project-dir-or.pbip> (--handle <column-handle> | --table <table> --name <column>) [--expression <dax> | --expression-file <path|->] [--data-type <type>] [--source-column <column>] [--format-string <fmt>] [--summarize-by <mode>] [--sort-by <column> | --clear-sort-by] [--display-folder <folder>] [--description <text>] [--hidden|--visible] [--key|--not-key] (--dry-run | --in-place | --out-dir <dir>) --json` — Update a base or calculated column while refusing lossy unknown TMDL metadata _(proof: `unit-smoke`)_
- `powerbi-cli model cultures list --project <project-dir-or.pbip> [--include-raw] --json` — List culture/translation TMDL blocks by stable handle _(proof: `unit-smoke`)_
- `powerbi-cli model cultures show --project <project-dir-or.pbip> (--handle <culture-handle> | --name <culture-name>) [--include-raw] --json` — Show one culture/translation TMDL block by stable handle or exact name _(proof: `unit-smoke`)_
- `powerbi-cli model dax bridge-plan --project <project-dir-or.pbip> [--engine desktop|xmla|tabular-editor] --json` — Inventory DAX measures/calculated columns and return the external validation bridge boundary without fake local DAX compatibility claims _(proof: `unit-smoke`)_
- `powerbi-cli model dax dependencies --project <project-dir-or.pbip> --json` — Extract static DAX table/column and measure references for dependency graphing without claiming DAX-engine validation _(proof: `unit-smoke`)_
- `POWERBI_DESKTOP_ORACLE=1 powerbi-cli model dax execute --project <project-dir-or.pbip-or.pbix> (--query <dax> | --query-file <path|->) --allow-data-read [--enable-oracle] [--max-rows <1..100000>] [--max-cell-chars <1..1000000>] [--timeout-ms <1000..300000>] --json` — Execute a bounded read-only DAX EVALUATE query against the exact already-open Power BI Desktop semantic model _(proof: `unit-smoke`)_
- `powerbi-cli model dax lint --project <project-dir-or.pbip> --json` — Run static DAX and measure-format lint for missing references, ambiguous names, self references, dependency cycles, and malformed or absent display formats _(proof: `unit-smoke`)_
- `powerbi-cli model expressions add --project <project-dir-or.pbip> --name <expression-name> (--expression <m> | --expression-file <path|->) (--dry-run | --in-place | --out-dir <dir>) --json` — Add a named M expression to definition/expressions.tmdl _(proof: `unit-smoke`)_
- `powerbi-cli model expressions delete --project <project-dir-or.pbip> (--handle <expression-handle> | --name <expression-name>) (--dry-run | --in-place --confirm <expression-handle> | --out-dir <dir>) --json` — Delete one named M expression with guarded in-place confirmation _(proof: `unit-smoke`)_
- `powerbi-cli model expressions list --project <project-dir-or.pbip> [--include-raw] --json` — List named expression TMDL blocks by stable handle _(proof: `unit-smoke`)_
- `powerbi-cli model expressions show --project <project-dir-or.pbip> (--handle <expression-handle> | --name <expression-name>) [--include-raw] --json` — Show one named expression TMDL block by stable handle or exact name _(proof: `unit-smoke`)_
- `powerbi-cli model expressions update --project <project-dir-or.pbip> (--handle <expression-handle> | --name <expression-name>) (--expression <m> | --expression-file <path|->) (--dry-run | --in-place | --out-dir <dir>) --json` — Replace one named M expression while refusing unknown Desktop metadata _(proof: `unit-smoke`)_
- `POWERBI_DESKTOP_ORACLE=1 powerbi-cli model live export-tmdl --document <project-dir-or.pbip-or.pbix> --out-dir <fresh-dir> --allow-model-read [--enable-oracle] [--timeout-ms <1000..300000>] --json` — Export the semantic model of one exact already-open Desktop PBIP/PBIX document to a fresh validated TMDL definition through the pinned local Microsoft MCP _(proof: `unit-smoke`)_
- `powerbi-cli model measures add --project <project-dir-or.pbip> --table <table> --name <measure> (--expression <dax> | --expression-file <path|->) [--format-string <fmt> | --format-string-definition <dax>] [--display-folder <folder>] [--description <text>] (--dry-run | --in-place | --out-dir <dir>) --json` — Add a DAX measure from inline text or a UTF-8 expression file to a TMDL table with guarded output semantics _(proof: `unit-smoke`)_
- `powerbi-cli model measures delete --project <project-dir-or.pbip> (--handle <measure-handle> | --table <table> --name <measure>) (--dry-run | --in-place --confirm <measure-handle> | --out-dir <dir>) --json` — Delete a DAX measure; in-place delete requires exact handle confirmation _(proof: `unit-smoke`)_
- `powerbi-cli model measures list --project <project-dir-or.pbip> [--table <table>] --json` — List semantic model DAX measures with stable handles _(proof: `unit-smoke`)_
- `powerbi-cli model measures show --project <project-dir-or.pbip> (--handle <measure-handle> | --table <table> --name <measure>) --json` — Show one semantic model DAX measure and its TMDL block _(proof: `unit-smoke`)_
- `powerbi-cli model measures update --project <project-dir-or.pbip> (--handle <measure-handle> | --table <table> --name <measure>) [--expression <dax> | --expression-file <path|->] [--format-string <fmt> | --format-string-definition <dax>] [--display-folder <folder>] [--description <text>] (--dry-run | --in-place | --out-dir <dir>) --json` — Update a DAX measure from inline text or a UTF-8 expression file; refuses unsupported Desktop-authored TMDL lines _(proof: `unit-smoke`)_
- `powerbi-cli model partitions add-grouped-rank --project <project-dir-or.pbip> --table <table> --group-by <column> [--group-by <column> ...] --order-by <column> [--desc] --rank-column <int64-column> --eligible-when <M-predicate> (--dry-run | --out-dir <dir> | --in-place) --json` — Append a deterministic refresh-time grouped-rank M chain to one safe generated dummy partition, assigning zero to ineligible rows and explicitly retyping the rank _(proof: `schema-golden`)_
- `powerbi-cli model partitions list --project <project-dir-or.pbip> [--table <table>] --json` — List semantic model partitions with source kind and offline safety classification _(proof: `unit-smoke`)_
- `powerbi-cli model partitions show --project <project-dir-or.pbip> (--handle <partition-handle> | --table <table> --name <partition-name>) [--include-source] --json` — Show one semantic model partition with a redacted preview by default; raw source/block output requires --include-source and is refused unless safety is safe _(proof: `unit-smoke`)_
- `powerbi-cli model perspectives list --project <project-dir-or.pbip> [--include-raw] --json` — List perspective TMDL blocks by stable handle _(proof: `unit-smoke`)_
- `powerbi-cli model perspectives show --project <project-dir-or.pbip> (--handle <perspective-handle> | --name <perspective-name>) [--include-raw] --json` — Show one perspective TMDL block by stable handle or exact name _(proof: `unit-smoke`)_
- `powerbi-cli model relationships add --project <project-dir-or.pbip> --from-table <table> --from-column <column> --to-table <table> --to-column <column> [--name <relationship-name>] [--from-cardinality <one|many>] [--to-cardinality <one|many>] [--cross-filtering-behavior <oneDirection|bothDirections|automatic>] [--inactive] (--dry-run | --in-place | --out-dir <dir>) --json` — Add a semantic model relationship with explicit endpoints and guarded output semantics _(proof: `unit-smoke`)_
- `powerbi-cli model relationships delete --project <project-dir-or.pbip> (--handle <relationship-handle> | --name <relationship-name>) (--dry-run | --in-place --confirm <relationship-handle> | --out-dir <dir>) --json` — Delete a semantic model relationship; in-place delete requires exact handle confirmation _(proof: `unit-smoke`)_
- `powerbi-cli model relationships list --project <project-dir-or.pbip> [--table <table>] --json` — List semantic model relationships with stable relationship handles and endpoint column handles _(proof: `unit-smoke`)_
- `powerbi-cli model relationships show --project <project-dir-or.pbip> (--handle <relationship-handle> | --name <relationship-name>) --json` — Show one semantic model relationship, endpoints, properties, and its TMDL block _(proof: `unit-smoke`)_
- `powerbi-cli model relationships update --project <project-dir-or.pbip> (--handle <relationship-handle> | --name <relationship-name>) [--from-cardinality <one|many>] [--to-cardinality <one|many>] [--cross-filtering-behavior <oneDirection|bothDirections|automatic>] [--active|--inactive] (--dry-run | --in-place | --out-dir <dir>) --json` — Update relationship active state, cardinality, or cross-filtering behavior; endpoint rewiring is delete+add _(proof: `unit-smoke`)_
- `powerbi-cli model roles list --project <project-dir-or.pbip> [--include-raw] --json` — List role/RLS TMDL blocks by stable handle _(proof: `unit-smoke`)_
- `powerbi-cli model roles show --project <project-dir-or.pbip> (--handle <role-handle> | --name <role-name>) [--include-raw] --json` — Show one role/RLS TMDL block by stable handle or exact name _(proof: `unit-smoke`)_
- `powerbi-cli model tables add --project <project-dir-or.pbip> --table <table> [--column <column> ... | --columns-json <json-array>] [--data-type <type>] (--dry-run | --in-place | --out-dir <dir>) --json` — Add a typed offline-safe TMDL table with a generated dummy partition _(proof: `unit-smoke`)_
- `powerbi-cli model tables add-calculated --project <project-dir-or.pbip> --table <table> (--expression <dax> | --expression-file <path|->) [--column <column> ... | --columns-json <json-array>] (--dry-run | --in-place | --out-dir <dir>) --json` — Add a calculated TMDL table backed by an offline-safe DAX partition _(proof: `unit-smoke`)_
- `powerbi-cli model tables add-static --project <project-dir-or.pbip> --table <table> ((--column <column> --values-json <json-array-of-strings>) | (--columns-json <json-array-of-column-names> --rows-json <json-array-of-string-arrays>)) [--include-raw] (--dry-run | --in-place | --out-dir <dir>) --json` — Add a small static selector table or multi-column lookup dimension backed by an inline M table _(proof: `unit-smoke`)_
- `powerbi-cli model tables delete --project <project-dir-or.pbip> (--handle <table-handle> | --table <table>) (--dry-run | --in-place --confirm <table-handle> | --out-dir <dir>) --json` — Delete an unreferenced TMDL table with guarded in-place confirmation _(proof: `unit-smoke`)_
- `powerbi-cli model tables list --project <project-dir-or.pbip> --json` — List semantic-model tables with stable table handles and child counts _(proof: `unit-smoke`)_
- `powerbi-cli model tables rename --project <project-dir-or.pbip> (--handle <table-handle> | --table <table>) --new-name <table> [--rename-references] (--dry-run | --in-place | --out-dir <dir>) --json` — Rename a TMDL table and optionally rewrite relationship, DAX, and variation references _(proof: `unit-smoke`)_
- `powerbi-cli model tables show --project <project-dir-or.pbip> (--handle <table-handle> | --table <table>) --json` — Show one semantic-model table, child inventory, and raw TMDL block _(proof: `unit-smoke`)_
- `powerbi-cli package export-plan --project <project-dir-or.pbip> --json` — Return the Desktop handoff plan for producing PBIX/PBIT because powerbi-cli does not write opaque binary package containers _(proof: `unit-smoke`)_
- `powerbi-cli package extract <file.pbix|file.pbit|file.zip> --out-dir <dir> [--include-unknown] [--max-entries <n>] [--max-entry-bytes <n>] [--max-total-bytes <n>] [--max-compression-ratio <n>] --json` — Extract selected source/metadata entries with streaming archive-bomb budgets and clean partial-output rollback _(proof: `unit-smoke`)_
- `powerbi-cli package import <file.pbix|file.pbit|file.zip> --out-dir <project-dir> [--max-entries <n>] [--max-entry-bytes <n>] [--max-total-bytes <n>] [--max-compression-ratio <n>] --json` — Import PBIP/PBIR/TMDL source entries only when they are actually present inside a package archive _(proof: `unit-smoke`)_
- `powerbi-cli package inspect <file.pbix|file.pbit|file.zip> --json` — Inspect a PBIX/PBIT ZIP-like package and classify source/metadata/cache entries without extracting opaque data caches _(proof: `unit-smoke`)_
- `powerbi-cli package source-pack --project <project-dir-or.pbip> --out <archive.pbit|archive.pbix|archive.zip> [--force] [--dry-run] --json` — Write a deterministic, allowlisted source archive only after credential and PII-suspect content scans pass _(proof: `unit-smoke`)_
- `powerbi-cli package work-pack --project <project-dir-or.pbip> [--out <archive.pbit|archive.pbix|archive.zip>] [--force] [--dry-run] --json` — Write a deterministic credential-free work-machine archive containing only recognized materialized live connectors _(proof: `unit-smoke`)_
- `powerbi-cli profile infer --schema <schema.json> [--rows <rows.csv|rows.json>] [--out <profile.json>] [--include-data-values] [--redact] --json` — Infer an advisory profile from schema metadata and bounded CSV/JSON rows; top values are redacted unless explicitly opted in _(proof: `unit-smoke`)_
- `powerbi-cli profile summarize <profile.json> --json` — Return a compact summary of a dashboard data profile _(proof: `unit-smoke`)_
- `powerbi-cli profile validate <profile.json> --json` — Validate a data profile document used by dashboard planning/build flows _(proof: `unit-smoke`)_
- `powerbi-cli report audit --project <project-dir-or.pbip> [--profile agent-safe|handoff] [--include-raw] --json` — Audit report PBIR state for persisted values, raw-literal risks, stale references, and handoff hygiene issues _(proof: `unit-smoke`)_
- `powerbi-cli report bookmarks delete --project <project-dir-or.pbip> --handle <bookmark-handle> (--dry-run | --in-place --confirm <bookmark-handle> | --out-dir <dir>) --json` — Delete one bookmark file and remove it from bookmark metadata with guarded output semantics _(proof: `unit-smoke`)_
- `powerbi-cli report bookmarks list --project <project-dir-or.pbip> [--include-raw] --json` — List raw PBIR bookmark files with stable handles, bookmark order/group metadata, and data-value safety warnings _(proof: `unit-smoke`)_
- `powerbi-cli report bookmarks reorder --project <project-dir-or.pbip> --order <bookmark-handle,...> (--dry-run | --in-place | --out-dir <dir>) --json` — Reorder flat bookmark metadata without changing captured bookmark state _(proof: `unit-smoke`)_
- `powerbi-cli report bookmarks set-display-name --project <project-dir-or.pbip> --handle <bookmark-handle> --display-name <text> (--dry-run | --in-place | --out-dir <dir>) --json` — Patch only bookmark displayName metadata without capturing or changing bookmark state _(proof: `unit-smoke`)_
- `powerbi-cli report bookmarks show --project <project-dir-or.pbip> --handle <bookmark-handle> [--no-raw] --json` — Show one raw PBIR bookmark by stable handle, including captured state summary and persisted-value safety metadata _(proof: `unit-smoke`)_
- `powerbi-cli report build --schema <schema.json> [--profile <profile.json>] [--spec <dashboard.json>] (--dry-run | --out-dir <project-dir> [--force]) [--trace] --json` — Compile a data schema plus optional strict v1/v2 dashboard spec into an offline-safe PBIP/PBIR/TMDL project using supported primitives only, including aggregated operation changes, stable-handle readback, scorecard, and side-effect-free proofPlan commands _(proof: `unit-smoke`)_
- `powerbi-cli report cat --project <project-dir-or.pbip> --handle <object-handle> [--include-raw] --json` — Show one report object by stable handle; raw PBIR content is returned only with --include-raw _(proof: `unit-smoke`)_
- `powerbi-cli report design-plan --project <project-dir-or.pbip> --json` — Profile a model/report and return agent-ready visual, layout, drilldown, and style authoring opportunities with exact next commands _(proof: `unit-smoke`)_
- `powerbi-cli report drilldown set-hierarchy --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-title>) --field <table[column]> --field <table[column]>... (--dry-run | --in-place | --out-dir <dir>) [--include-raw] --json` — Replace a category-axis chart's Category projections with a multi-column hierarchy and enable its Desktop drill controls _(proof: `unit-smoke`)_
- `powerbi-cli report drillthrough clear --project <project-dir-or.pbip> (--page <page-name-or-handle> | --handle <page-handle>) [--restore-visible] (--dry-run | --in-place --confirm <page-handle> | --out-dir <dir>) [--include-raw] --json` — Remove drillthrough page type, pageBinding, and existing drillthrough-created page filters with guarded output semantics _(proof: `schema-golden`)_
- `powerbi-cli report drillthrough set --project <project-dir-or.pbip> (--page <page-name-or-handle> | --handle <page-handle>) (--target <table[column]> | --table <table> --column <column>) [--keep-all-filters true|false] [--keep-visible] (--dry-run | --in-place | --out-dir <dir>) [--include-raw] --json` — Mark an existing PBIR page as a same-report drillthrough target using linked pageBinding and filterConfig metadata _(proof: `schema-golden`)_
- `powerbi-cli report drillthrough show --project <project-dir-or.pbip> (--page <page-name-or-handle> | --handle <page-handle>) [--include-raw] --json` — Read linked drillthrough pageBinding parameters and paired Drillthrough filters _(proof: `schema-golden`)_
- `powerbi-cli report filters add --project <project-dir-or.pbip> [--scope report|page|visual] [--page <page-name-or-handle>] [--visual <visual-name-or-handle>] (--target <table[column]> | --table <table> --column <column>) [--condition-type categorical|range|topn|relative-date] ((--value <text> | --value-json <json> | --values-json <json-array>)... | [--min <number>] [--max <number>] | (--top <N> | --bottom <N>) --by <measure> | --relative last|next|this --unit days|weeks|months|years|calendar-weeks|calendar-months|calendar-years --span <N>) (--dry-run | --in-place | --out-dir <dir>) [--name <filter-name>] [--display-name <label>] [--include-raw] --json` — Add one categorical, numeric range, TopN, or relative-date PBIR filter with TMDL type checks and guarded output semantics _(proof: `schema-golden`)_
- `powerbi-cli report filters clear --project <project-dir-or.pbip> (--handle <filter-handle> | --scope report | --page <page-name-or-handle> | --visual <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle> | --all) (--dry-run | --in-place --confirm <confirm-token> | --out-dir <dir>) [--include-raw] --json` — Clear existing PBIR filters by exact filter handle, report scope, one page owner, one visual owner, or explicit --all with guarded output semantics _(proof: `unit-smoke`)_
- `powerbi-cli report filters delete --project <project-dir-or.pbip> --handle <filter-handle> (--dry-run | --in-place --confirm <filter-handle> | --out-dir <dir>) [--include-raw] --json` — Delete one existing report, page, or visual PBIR filter by stable handle with guarded output semantics _(proof: `unit-smoke`)_
- `powerbi-cli report filters list --project <project-dir-or.pbip> [--scope all|report|page|visual] [--page <page-name-or-handle>] [--visual <visual-name-or-handle>] [--include-raw] --json` — List raw PBIR report, page, and visual filters with stable handles and data-value safety warnings _(proof: `unit-smoke`)_
- `powerbi-cli report filters show --project <project-dir-or.pbip> --handle <filter-handle> [--no-raw] --json` — Show one raw PBIR filter by stable handle, including owner readback and persisted-value safety metadata _(proof: `unit-smoke`)_
- `powerbi-cli report filters update --project <project-dir-or.pbip> --handle <filter-handle> (--display-name <label> | (--value <text> | --value-json <json> | --values-json <json-array>)...) [--condition-type categorical|range|topn|relative-date] (--dry-run | --in-place | --out-dir <dir>) [--include-raw] --json` — Update one filter by stable handle: change any display name or replace categorical values while preserving filter type _(proof: `unit-smoke`)_
- `powerbi-cli report find --project <project-dir-or.pbip> [--kind <kind>] [--name-contains <text>] [--title-contains <text>] [--visual-type <type>] [--path-contains <text>] [--include-raw] --json` — Search report objects by stable metadata instead of guessing PBIR file paths _(proof: `unit-smoke`)_
- `powerbi-cli report interactions disable --project <project-dir-or.pbip> (--handle <interaction-handle> | --page <page-name-or-handle> --source <visual-name-or-handle> --target <visual-name-or-handle>) (--dry-run | --in-place | --out-dir <dir>) --json` — Upsert an explicit NoFilter visualInteraction row so the target visual does not react to the source visual _(proof: `unit-smoke`)_
- `powerbi-cli report interactions list --project <project-dir-or.pbip> [--page <page-name-or-handle>] [--source <visual-name-or-handle>] [--target <visual-name-or-handle>] [--type Default|DataFilter|HighlightFilter|NoFilter] [--include-raw] --json` — List explicit PBIR page visualInteraction overrides with stable handles, source/target visual resolution, and default-interaction semantics _(proof: `unit-smoke`)_
- `powerbi-cli report interactions reset --project <project-dir-or.pbip> --page <page-name-or-handle> --source <visual-name-or-handle> --target <visual-name-or-handle> (--dry-run | --in-place | --out-dir <dir>) --json` — Remove one explicit PBIR visualInteractions row so the target visual returns to its documented default interaction behavior _(proof: `unit-smoke`)_
- `powerbi-cli report interactions set --project <project-dir-or.pbip> (--handle <interaction-handle> | --page <page-name-or-handle> --source <visual-name-or-handle> --target <visual-name-or-handle>) --type DataFilter|HighlightFilter|NoFilter (--dry-run | --in-place | --out-dir <dir>) --json` — Upsert one explicit PBIR page visualInteraction override for a source/target visual pair; Default authoring remains Desktop-fixture gated _(proof: `unit-smoke`)_
- `powerbi-cli report interactions show --project <project-dir-or.pbip> --handle <interaction-handle> [--no-raw] --json` — Show one explicit PBIR page visualInteraction override by handle or page/source/target selector _(proof: `unit-smoke`)_
- `powerbi-cli report layout auto --project <project-dir-or.pbip> [--page <page-name-or-handle>] [--template <name> | --preset overview|analysis|detail|grid] [--page-size 1280x720|1920x1080] [--grid columns=12,gutter=16,margin=24,rowUnit=8] [--margin <n>] [--gap <n>] [--row-unit <n>] (--dry-run | --in-place | --out-dir <dir>) --json` — Resolve named twelve-column design-system slots and reposition existing visuals into deterministic canvas coordinates without changing bindings or formatting _(proof: `unit-smoke`)_
- `powerbi-cli report pages add --project <project-dir-or.pbip> --display-name <name> [--name <pbir-page-name>] [--width <n>] [--height <n>] [--display-option <mode>] [--before <page-handle>|--after <page-handle>] [--set-active] (--dry-run | --in-place | --out-dir <dir>) --json` — Add an empty PBIR report page and update pageOrder with guarded output semantics _(proof: `unit-smoke`)_
- `powerbi-cli report pages clone --project <project-dir-or.pbip> --from <page-name-or-handle> --new-name <ReportSectionX> [--display-name <text>] [--visual-prefix <Prefix>] (--dry-run | --in-place | --out-dir <dir>) --json` — Clone a complete PBIR page, regenerate page/visual/filter identities, prune stale visual interactions, and append pageOrder _(proof: `schema-golden`)_
- `powerbi-cli report pages delete-empty --project <project-dir-or.pbip> (--handle <page-handle> | --page <page-name-or-handle>) (--dry-run | --in-place --confirm <page-handle> | --out-dir <dir>) --json` — Delete only a simple empty PBIR page; pages with visuals or unknown files are refused _(proof: `unit-smoke`)_
- `powerbi-cli report pages list --project <project-dir-or.pbip> --json` — List PBIR report pages with stable page handles and visual counts _(proof: `unit-smoke`)_
- `powerbi-cli report pages reorder --project <project-dir-or.pbip> --order <page-handle,...> (--dry-run | --in-place | --out-dir <dir>) --json` — Replace PBIR pageOrder after resolving every page handle exactly once _(proof: `unit-smoke`)_
- `powerbi-cli report pages set-active --project <project-dir-or.pbip> (--handle <page-handle> | --page <page-name-or-handle>) (--dry-run | --in-place | --out-dir <dir>) --json` — Set pages.json activePageName to an existing PBIR page _(proof: `unit-smoke`)_
- `powerbi-cli report pages show --project <project-dir-or.pbip> (--handle <page-handle> | --page <page-name-or-handle>) --json` — Show one PBIR report page with visual geometry and bindings _(proof: `unit-smoke`)_
- `powerbi-cli report pages update --project <project-dir-or.pbip> (--handle <page-handle> | --page <page-name-or-handle>) [--display-name <name>] [--width <n>] [--height <n>] [--display-option <mode>] [--allow-visuals-outside-page] (--dry-run | --in-place | --out-dir <dir>) --json` — Patch PBIR page display metadata without renaming the internal page handle _(proof: `unit-smoke`)_
- `powerbi-cli report plan --schema <schema.json> --profile <profile.json> (--intent <intent.md|intent.json> | --objective <goal>) [--out <dashboard.json>] --json` — Create a deterministic starter dashboard spec from schema/profile candidates and a typed JSON or Markdown report intent (with backward-compatible objective text) _(proof: `unit-smoke`)_
- `powerbi-cli report query --project <project-dir-or.pbip> --selector <selector> [--include-raw] --json` — Run a constrained stable-selector query over report objects for agent automation _(proof: `unit-smoke`)_
- `powerbi-cli report sanitize apply --project <project-dir-or.pbip> [--profile agent-safe|handoff] (--dry-run | --out-dir <dir> | --in-place --confirm sanitize:<planFingerprint>) --json` — Apply only supported sanitize actions under guarded dry-run/out-dir/in-place semantics _(proof: `unit-smoke`)_
- `powerbi-cli report sanitize plan --project <project-dir-or.pbip> [--profile agent-safe|handoff] --json` — Create a deterministic sanitize plan before clearing persisted report filter/slicer state or flagging plan-only manual review items _(proof: `unit-smoke`)_
- `powerbi-cli report slicers clear --project <project-dir-or.pbip> (--handle <slicer-or-visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) (--dry-run | --in-place --confirm <confirm-token> | --out-dir <dir>) [--include-raw] --json` — Clear persisted PBIR slicer selection/filter state for one slicer visual without changing bindings, layout, or formatting _(proof: `unit-smoke`)_
- `powerbi-cli report slicers list --project <project-dir-or.pbip> [--page <page-name-or-handle>] [--include-raw] --json` — List PBIR slicer visuals with stable slicer handles, visual handles, bindings, state summaries, and persisted-value safety warnings _(proof: `unit-smoke`)_
- `powerbi-cli report slicers show --project <project-dir-or.pbip> --handle <slicer-handle> [--no-raw] --json` — Show one PBIR slicer visual by slicer or visual handle, including raw visual state and persisted-value safety metadata _(proof: `unit-smoke`)_
- `powerbi-cli report spec explain --schema <schema.json> [--profile <profile.json>] --spec <dashboard.json> --json` — Compile a strict dashboard spec to a deterministic staged operation plan without writing a project _(proof: `unit-smoke`)_
- `powerbi-cli report spec fields [--schema <schema.json>] [--profile <profile.json>] --json` — List the strict dashboard-spec key catalog and, when a schema is supplied, exact column and measure binding references _(proof: `unit-smoke`)_
- `powerbi-cli report spec normalize [--spec <dashboard.json> | <dashboard.json>] --out <canonical.json> --json` — Resolve supported dashboard-spec includes and write one deterministic canonical JSON document _(proof: `unit-smoke`)_
- `powerbi-cli report spec schema [--version v1|v2|all] --json` — Emit the draft 2020-12 JSON Schema generated from the strict v1/v2 dashboard-spec key catalog _(proof: `unit-smoke`)_
- `powerbi-cli report spec upgrade --spec <v1.json> (--dry-run | --out <v2.json> [--force]) --json` — Losslessly rewrite a strict powerbi-cli.dashboard.v1 spec as normalized powerbi-cli.dashboard.v2 JSON _(proof: `unit-smoke`)_
- `powerbi-cli report spec validate [--schema <schema.json>] --spec <dashboard.json> [--profile <profile.json>] --json` — Validate a strict powerbi-cli.dashboard.v1 or v2 shape, and compile-check it against a schema/profile when --schema is supplied before report build _(proof: `unit-smoke`)_
- `powerbi-cli report style apply --project <target-project-or.pbip> --bundle <style-bundle.json> [--allow-literal-text] (--dry-run | --in-place | --out-dir <dir>) --json` — Apply a master-style bundle by replacing report themeCollection and matching visual formatting payloads by visualType+ordinal _(proof: `unit-smoke`)_
- `powerbi-cli report style diff <before-style.json> <after-style.json> --json` — Compare two extracted report style bundles by fingerprint, themeCollection, and visual-style counts _(proof: `unit-smoke`)_
- `powerbi-cli report style extract --project <project-dir-or.pbip> [--out <style-bundle.json>] [--include-literal-text] --json` — Extract a portable master-style bundle containing report themeCollection and per-visual formatting payloads _(proof: `unit-smoke`)_
- `powerbi-cli report style inspect --project <project-dir-or.pbip> --json` — Inspect a combined report style bundle: report themeCollection plus per-visual formatting payload summaries _(proof: `unit-smoke`)_
- `powerbi-cli report themes apply --project <target-project-or.pbip> --bundle <theme-bundle.json> (--dry-run | --in-place | --out-dir <dir>) --json` — Apply a raw report theme bundle by replacing themeCollection and copied registered theme JSON resources; does not copy per-visual formatting _(proof: `unit-smoke`)_
- `powerbi-cli report themes apply-preset --project <target-project-or.pbip> [--preset risk-dashboard|neutral-ops] (--dry-run | --in-place | --out-dir <dir>) --json` — Apply a built-in registered-resource theme preset to a report with guarded output semantics _(proof: `unit-smoke`)_
- `powerbi-cli report themes extract --project <source-project-or.pbip> [--out <theme-bundle.json>] --json` — Extract a deterministic raw report theme bundle from themeCollection and already-present registered theme JSON resources _(proof: `unit-smoke`)_
- `powerbi-cli report themes presets list --json | powerbi-cli report themes presets show --preset <preset-id> [--include-bundle] --json` — List or show built-in report theme presets that apply as registered theme JSON resources _(proof: `unit-smoke`)_
- `powerbi-cli report themes show --project <project-dir-or.pbip> --json` — Show raw report-level theme state, fingerprint, themeCollection, and registered theme JSON resources _(proof: `unit-smoke`)_
- `powerbi-cli report tree --project <project-dir-or.pbip> [--include-raw] --json` — Return a stable navigable report object tree across pages, visuals, bindings, filters, slicers, bookmarks, and interactions _(proof: `unit-smoke`)_
- `powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-name-or-handle> --title <title> [--visual-type <type>] [--mode basic|dropdown|between] [--name <visual-name>] [--x <n>] [--y <n>] [--width <n>] [--height <n>] [--z <n>] [--tab-order <n>] (--binding <key=value,...> | --bindings-json <json> | --bindings-file <file>) [--allow-outside-page] (--dry-run | --in-place | --out-dir <dir>) --json` — Create a PBIR visual container on an existing page using the same minimal generated patterns as scaffold _(proof: `schema-golden`)_
- `powerbi-cli report visuals add-card --project <project-dir-or.pbip> --page <page-name-or-handle> --measure <Table.Measure> --title <text> --x <n> --y <n> --width <n> --height <n> [--name <VisualContainerX>] [--value-font-size <n>] [--category-font-size <n>] [--word-wrap] (--dry-run | --in-place | --out-dir <dir>) --json` — Scaffold a Desktop-proven KPI card visual.json with a Values measure binding and optional label/category font objects _(proof: `schema-golden`)_
- `powerbi-cli report visuals add-slicer --project <project-dir-or.pbip> --page <page-name-or-handle> --field <Table.Column> --title <text> --x <n> --y <n> --width <n> --height <n> [--name <VisualContainerX>] [--mode Basic|Dropdown] [--single-select] (--dry-run | --in-place | --out-dir <dir>) --json` — Scaffold a Desktop-proven slicer visual.json with a Column projection, Dropdown/Basic mode, and optional single-select _(proof: `schema-golden`)_
- `powerbi-cli report visuals add-textbox --project <project-dir-or.pbip> --page <page-name-or-handle> --title <text> --x <n> --y <n> --width <n> --height <n> [--name <VisualContainerX>] (--paragraphs-file <path|-> | --text <paragraph>) (--dry-run | --in-place | --out-dir <dir>) --json` — Scaffold a Desktop-proven reading-guide textbox with first paragraph bold 12pt and remaining paragraphs 10pt _(proof: `schema-golden`)_
- `powerbi-cli report visuals catalog [--visual-type <type-or-alias>] [--formatting] --json` — Return generated visual types plus complete fixture-backed role, projection, exclusivity, and runtime-parity rules, or the curated typed-formatting property catalog _(proof: `unit-smoke`)_
- `powerbi-cli report visuals clone --project <project-dir-or.pbip> (--handle <source-visual-handle> | --from-page <page-name-or-handle> --visual <visual-name-or-handle>) [--target-page <page-name-or-handle>] [--name <new-visual-name>] [--title <title>] [--x <n>] [--y <n>] [--width <n>] [--height <n>] [--z <n>] [--tab-order <n>] [--allow-outside-page] (--dry-run | --in-place | --out-dir <dir>) --json` — Clone one simple PBIR visual container by copying visual.json and patching only name, position, and clone annotations _(proof: `unit-smoke`)_
- `powerbi-cli report visuals delete --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) (--dry-run | --in-place --confirm <visual-handle> | --out-dir <dir>) --json` — Delete one PBIR visual container directory after proving it contains only visual.json; in-place delete requires exact handle confirmation _(proof: `schema-golden`)_
- `powerbi-cli report visuals formatting apply --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) --bundle <formatting-bundle.json> [--allow-literal-text] [--allow-cross-type] [--include-raw] (--dry-run | --in-place | --out-dir <dir>) --json` — Apply a visual formatting bundle by replacing only /visual/objects and /objects on the target visual _(proof: `unit-smoke`)_
- `powerbi-cli report visuals formatting conditional-formatting list --project <project-dir-or.pbip> [--page <page-name-or-handle>] [--include-raw] --json` — Inventory conditional-formatting/rule/gradient PBIR signals across visuals without authoring new rules _(proof: `unit-smoke`)_
- `powerbi-cli report visuals formatting conditional-formatting show --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) [--include-raw] --json` — Show conditional-formatting/rule/gradient PBIR signals for one visual _(proof: `unit-smoke`)_
- `powerbi-cli report visuals formatting extract --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) [--out <formatting-bundle.json>] --json` — Extract one visual's raw PBIR formatting objects into an auditable bundle for style portability _(proof: `unit-smoke`)_
- `powerbi-cli report visuals formatting list --project <project-dir-or.pbip> [--page <page-name-or-handle>] [--include-raw] --json` — Inventory per-visual PBIR formatting object containers and property names without raw formatting payloads by default _(proof: `unit-smoke`)_
- `powerbi-cli report visuals formatting set-color --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) (--slot title.fontColor|dataPoint.fill --color <#RRGGBB|#AARRGGBB> | --title-font-color <hex> | --data-point-fill <hex>) [--include-raw] (--dry-run | --in-place | --out-dir <dir>) --json` — Patch static PBIR visual title font color or wildcard data point fill without replacing other formatting objects _(proof: `unit-smoke`)_
- `powerbi-cli report visuals formatting set-text --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) [--title <text>] [--show-title true|false] [--clear-alt-text] [--include-raw] (--dry-run | --in-place | --out-dir <dir>) --json` — Patch typed PBIR visual title visibility/text or remove validator-rejected alt-text metadata without replacing sibling formatting objects _(proof: `unit-smoke`)_
- `powerbi-cli report visuals formatting show --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) [--include-raw] --json` — Show one visual's PBIR formatting object inventory; raw PBIR objects require explicit --include-raw _(proof: `unit-smoke`)_
- `powerbi-cli report visuals list --project <project-dir-or.pbip> [--page <page-name-or-handle>] --json` — List PBIR report visuals with stable handles, page context, geometry, and binding counts _(proof: `unit-smoke`)_
- `powerbi-cli report visuals repair-bindings --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) --dry-run --json` — Inspect one existing visual against the fixture-backed role map and propose the minimal proven set-bindings op for mechanical runtime-parity defects _(proof: `schema-golden`)_
- `powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) (--binding <key=value,...> | --bindings-json <json> | --bindings-file <file> | --clear-bindings) (--dry-run | --in-place | --out-dir <dir>) --json` — Replace or clear PBIR field-well bindings for an existing visual using canonical TMDL table, column, and measure names _(proof: `unit-smoke`)_
- `powerbi-cli report visuals set-display-name --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-title>) --role <Values|Category|Series|X|Y|Y2|Size|Rows|Columns|Tooltips> [--index <n>] (--display-name <text> | --clear) (--dry-run | --in-place | --out-dir <dir>) --json` — Set or clear displayName on one existing visual queryState projection _(proof: `unit-smoke`)_
- `powerbi-cli report visuals set-object --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-title>) --object <name> --property <name> --value <raw> (--dry-run | --in-place | --out-dir <dir>) --json` — Set one curated PBIR visual object property (labels, categoryLabels, categoryAxis, valueAxis, or title) using Desktop literal encoding _(proof: `unit-smoke`)_
- `powerbi-cli report visuals set-position --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) [--x <n>] [--y <n>] [--width <n>] [--height <n>] [--z <n>] [--tab-order <n>] [--allow-outside-page] (--dry-run | --in-place | --out-dir <dir>) --json` — Patch only a PBIR visual position object with guarded output semantics _(proof: `unit-smoke`)_
- `powerbi-cli report visuals set-topn-guard --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-title>) --field <Table.Column> --order-by <Table.Measure> --top <N> [--direction desc|asc] [--display-name <text>] [--name <filterName>] (--dry-run | --in-place | --out-dir <dir>) --json` — Create or update a visual-level TopN guard filter so a cheap ranking measure bounds the axis before heavy display measures evaluate _(proof: `unit-smoke`)_
- `powerbi-cli report visuals show --project <project-dir-or.pbip> (--handle <visual-handle> | --page <page-name-or-handle> --visual <visual-name-or-handle>) --json` — Show one PBIR visual with page context, geometry, type, and field bindings _(proof: `unit-smoke`)_
- `powerbi-cli report wireframe export <project-dir-or.pbip> --json` — Export report pages, visual geometry, bindings, and report handles as JSON without Power BI Desktop _(proof: `unit-smoke`)_
- `powerbi-cli robot-docs guide [--json]` — Print the in-tool agent guide so agents do not need external docs _(proof: `unit-smoke`)_
- `powerbi-cli robot-docs render [--section commands|limits|features] [--check] [--root <repo-dir>] [--json]` — Render marker-delimited README and SKILL sections from the live capabilities and feature catalogs, or check for documentation drift _(proof: `unit-smoke`)_
- `powerbi-cli robot-triage` — Alias for --robot-triage when an agent expects a normal command token _(proof: `unit-smoke`)_
- `powerbi-cli --json scaffold --schema <schema.json> --out-dir <project-dir> [--force]` — Create an offline-safe PBIP project from a schema manifest _(proof: `unit-smoke`)_
- `powerbi-cli schema normalize <schema.json> --out <canonical.json> --json` — Resolve supported schema includes and write a canonical pretty-printed manifest for review and reproducible dashboard builds _(proof: `unit-smoke`)_
- `powerbi-cli schema validate <schema.json> --json` — Validate a data schema manifest before report planning or PBIP generation _(proof: `unit-smoke`)_
- `powerbi-cli skill install --json` — Install or repair the canonical embedded Codex skill without Python, network access, or an external script _(proof: `unit-smoke`)_
- `powerbi-cli skill status --json` — Verify that the globally installed Codex skill exactly matches the repository-embedded canonical skill _(proof: `unit-smoke`)_
- `powerbi-cli source-template add --project <project-dir-or.pbip> (--handle <partition-handle> | --table <table> [--partition <partition-name>]) [--name <template-name>] --kind <sql|postgres|odbc|excel|csv|folder|sharepoint|generic-m> [kind parameters] (--dry-run | --in-place | --out-dir <dir>) --json` — Add or replace a credential-free database, file, folder, SharePoint, or closed-grammar generic M source template sidecar without changing executable partitions _(proof: `unit-smoke`)_
- `powerbi-cli source-template apply --project <project-dir-or.pbip> (--handle <source-template-handle> | --name <template-name>) [kind parameters] [--replace-existing --confirm <partition-handle>] (--dry-run | --in-place | --out-dir <dir>) --json` — Materialize one credential-free source template, including a validated generic M expression, into a generated dummy partition, or explicitly retarget a confirmed existing credential-free partition _(proof: `unit-smoke`)_
- `powerbi-cli source-template list --project <project-dir-or.pbip> [--table <table>] [--kind <sql|postgres|odbc|excel|csv|folder|sharepoint|generic-m>] --json` — List credential-free sidecar source templates used by handoff rebind plans _(proof: `unit-smoke`)_
- `powerbi-cli source-template show --project <project-dir-or.pbip> (--handle <source-template-handle> | --name <template-name>) --json` — Show one source template, its partition mapping, M template preview, and safety findings _(proof: `unit-smoke`)_
- `powerbi-cli triage <project-dir-or.pbip> --json` — Run strict validation and lint together and return ranked findings plus the next copy-paste command _(proof: `unit-smoke`)_
- `powerbi-cli --json validate [--strict] [--backend native|microsoft-report|all] <project-dir-or.pbip>` — Run native PBIP/PBIR/TMDL validation by default, or explicitly add the exact official Microsoft report validator _(proof: `unit-smoke`)_
- `powerbi-cli version --json` — Return the binary version and agent contract version for provenance checks _(proof: `unit-smoke`)_
- `powerbi-cli workflow plan --project <project-dir-or.pbip> --profile <source-profile.json> --out <new-plan.json> --out-dir <new-project-dir> [--resource <name>=<path>] --json` — Create a fingerprinted deterministic plan for one selected PBIP closure and typed source-profile replacements _(proof: `unit-smoke`)_
- `powerbi-cli workflow run --plan <plan.json> --confirm <plan-fingerprint> --json` — Recheck a fingerprinted plan, build a fresh selected-artifact closure, apply exact local MCP model edits, validate, and write a checksummed receipt _(proof: `unit-smoke`)_
- `powerbi-cli workflow synthesize --project <project-dir-or.pbip> --expressions <expressions.tmdl> --out-dir <new-project-dir> [--map <schema.item>=<ExpressionName>] [--row-scale <positive-integer>] [--seed <non-negative-integer>] --json` — Copy a live PBIP into a fresh offline project, install synthetic shared M expressions, and replace shared Database connector steps with one complete navigation shim _(proof: `schema-golden`)_
- `powerbi-cli workflow verify --plan <plan.json> --json` — Reconstruct profile-derived plan and staged-model semantics, bind output and MCP readbacks, and rerun native/official validation without editing the workflow output _(proof: `unit-smoke`)_
<!-- powerbi-cli:commands:end -->

<!-- powerbi-cli:limits:start -->
### Input-safety limits (generated from `capabilities.limits`)

The exact bounded input contract is live in the capabilities payload.

```json
{
  "dashboardSpec": {
    "maxBytes": 8388608,
    "symlinks": "refused",
    "utf8": true
  },
  "errorCode": "input_safety_violation",
  "harvestedFragments": {
    "maxBytes": 4194304,
    "persistedDataValues": "refused",
    "silentStripping": false
  },
  "images": {
    "externalUrls": "refused",
    "formats": [
      "png"
    ],
    "magicByteSniffed": true,
    "maxBytes": 16777216
  },
  "include": {
    "canonicalized": true,
    "cycles": "refused",
    "maxDepth": 8,
    "maxFragmentBytes": 8388608,
    "maxResolvedFragments": 200,
    "relativeOnly": true,
    "symlinks": "refused"
  },
  "intent": {
    "includeAndExecDirectives": "refused",
    "maxBytes": 1048576,
    "utf8": true
  },
  "jsonArtifact": {
    "maxBytes": 16777216,
    "symlinks": "refused",
    "utf8": true
  },
  "ops": {
    "maxBytes": 8388608,
    "schema": "powerbi-cli.ops.v1",
    "schemaValidationBeforeApply": true,
    "unknownOpKinds": "refused"
  },
  "profile": {
    "maxBytes": 8388608,
    "symlinks": "refused",
    "utf8": true
  },
  "projectText": {
    "maxBytesPerFile": 16777216,
    "symlinks": "refused",
    "utf8": true
  },
  "reservedApis": [
    "IncludeGuard::resolve",
    "read_rows",
    "read_png",
    "read_ops",
    "snapshot_destination",
    "read_harvested_fragment"
  ],
  "rows": {
    "decodeErrors": "refused",
    "leadingFormulaCharacters": "preserved-verbatim",
    "maxColumns": 512,
    "maxFileBytes": 67108864,
    "maxRows": 100000,
    "utf8": true
  },
  "schema": {
    "maxBytes": 8388608,
    "symlinks": "refused",
    "utf8": true
  },
  "snapshots": {
    "location": "sibling-or-explicit-snapshot-dir",
    "maxFiles": 10000,
    "maxTotalBytes": 536870912,
    "unwritableDestination": "refused"
  },
  "sourceText": {
    "maxBytes": 2097152,
    "symlinks": "refused",
    "utf8": true
  }
}
```
<!-- powerbi-cli:limits:end -->

<!-- powerbi-cli:features:start -->
### Feature catalog (generated from `features list --json`)

Each feature carries its live support status and proof level; update `src/feature_catalog.rs` rather than this region.

- `agent.codex-skill-distribution` — **supported**, embedded-install-and-hash-verification, proof `unit-smoke`: Self-contained Codex skill installation and verification. Commands: `skill install`, `skill status`.
- `desktop.canvas-check` — **planned**, planned-windows-oracle, proof `unit-smoke`: Desktop canvas and expected-value proof. Commands: `desktop canvas-check`.
- `desktop.dax-query-execution` — **supported**, explicit-opt-in-exact-document-read-only-query, proof `unit-smoke`: Bounded DAX query execution against an open Desktop model. Commands: `model dax execute`.
- `desktop.live-tmdl-export` — **supported**, explicit-opt-in-exact-document-read-only-mcp-export, proof `unit-smoke`: Read-only semantic-model TMDL export from an open Desktop document. Commands: `model live export-tmdl`.
- `desktop.reference-harvest` — **supported**, linux-safe-provenance-archive, proof `desktop-golden-pending`: Desktop-authored PBIR reference harvesting. Commands: `desktop harvest-reference`.
- `desktop.refresh-check` — **planned**, planned-windows-oracle, proof `unit-smoke`: Desktop refresh proof. Commands: `desktop refresh-check`.
- `desktop.window-evidence` — **supported**, opt-in-window-observation-and-primary-display-capture, proof `unit-smoke`: Managed Desktop sessions, window observation, and screenshot evidence. Commands: `desktop open`, `desktop close`, `desktop open-check`, `desktop screenshot`.
- `integrations.microsoft-toolchain` — **supported**, explicit-install-immutable-cache, proof `unit-smoke`: Exact optional Microsoft Power BI toolchain. Commands: `integrations status`, `integrations install`.
- `model.advanced-readback` — **supported**, read-only, proof `unit-smoke`: Advanced semantic model TMDL readback. Commands: `model advanced inventory`, `model roles list`, `model roles show`, `model perspectives list`, `model perspectives show`, `model cultures list`, `model cultures show`, `model expressions list`, `model expressions show`.
- `model.calculated-columns` — **supported**, read-write, proof `unit-smoke`: DAX calculated columns. Commands: `model calculated-columns list`, `model calculated-columns show`, `model calculated-columns add`, `model calculated-columns update`, `model calculated-columns delete`.
- `model.calculated-tables` — **supported**, read-write, proof `unit-smoke`: Calculated semantic-model tables. Commands: `model tables add-calculated`.
- `model.columns` — **supported**, read-write, proof `unit-smoke`: Semantic-model base and calculated column inventory and CRUD. Commands: `model columns list`, `model columns show`, `model columns add`, `model columns update`, `model columns delete`.
- `model.dax-static-analysis` — **supported**, read-only-static-analysis, proof `unit-smoke`: DAX dependency inventory and static lint. Commands: `model dax dependencies`, `model dax lint`, `model dax bridge-plan`.
- `model.measures` — **supported**, read-write, proof `unit-smoke`: DAX measures. Commands: `model measures list`, `model measures show`, `model measures add`, `model measures update`, `model measures delete`.
- `model.named-expressions` — **supported**, read-write, proof `unit-smoke`: Named M expression authoring. Commands: `model expressions list`, `model expressions show`, `model expressions add`, `model expressions update`, `model expressions delete`.
- `model.partition-grouped-rank` — **supported**, safe-generated-partition-mutation, proof `schema-golden`: Refresh-time grouped rank partition generator. Commands: `model partitions add-grouped-rank`.
- `model.relationships` — **supported**, read-write, proof `unit-smoke`: Model relationships. Commands: `model relationships list`, `model relationships show`, `model relationships add`, `model relationships update`, `model relationships delete`.
- `model.source-templates` — **supported**, sidecar-sql-postgres-odbc-excel-csv-folder-sharepoint-generic-m, proof `unit-smoke`: Credential-free source templates and rebind runbooks. Commands: `source-template list`, `source-template show`, `source-template add`, `source-template apply`, `handoff rebind-plan`, `handoff rebind-check`.
- `model.static-control-tables` — **supported**, add-bounded-string-table, proof `unit-smoke`: Small static selector and lookup tables. Commands: `model tables add-static`.
- `model.tables` — **supported**, read-write, proof `unit-smoke`: Semantic-model table inventory and CRUD. Commands: `model tables list`, `model tables show`, `model tables add`, `model tables rename`, `model tables delete`.
- `package.pbix-pbit-boundary` — **supported**, inspect-safe-metadata-source-pack-work-pack-export-plan, proof `unit-smoke`: PBIX/PBIT package boundary. Commands: `package inspect`, `package extract`, `package import`, `package source-pack`, `package work-pack`, `package export-plan`.
- `profile.data-profile-v2` — **supported**, schema-matched-statistics-with-redacted-values, proof `unit-smoke`: Bounded CSV/JSON data profile inference. Commands: `profile infer`, `profile validate`, `profile summarize`.
- `quality.lint-rule-registry` — **supported**, read-only-contract-catalog, proof `unit-smoke`: Discoverable lint and audit rule registry. Commands: `validate`, `lint`, `model dax lint`, `report audit`.
- `quality.model-completeness-lint` — **supported**, offline-static-heuristics, proof `unit-smoke`: DAX format and semantic-model completeness lint. Commands: `lint`, `triage`, `model dax lint`.
- `report.bookmark-mutations` — **planned**, unsupported, proof `unit-smoke`: Bookmark state capture/create/update.
- `report.bookmarks.readback` — **supported**, read-write-metadata-only, proof `unit-smoke`: Bookmark inventory/readback and metadata edits. Commands: `report bookmarks list`, `report bookmarks show`, `report bookmarks set-display-name`, `report bookmarks reorder`, `report bookmarks delete`.
- `report.conditional-formatting` — **supported**, read-only-static-scan, proof `unit-smoke`: Conditional formatting readback. Commands: `report visuals formatting conditional-formatting list`, `report visuals formatting conditional-formatting show`.
- `report.dashboard-spec-v2` — **supported**, strict-shape-partial-compile, proof `unit-smoke`: Strict dashboard spec v2 shape and compilation boundary. Commands: `report spec fields`, `report spec validate`, `report spec normalize`, `report spec upgrade`, `report build`.
- `report.design-layout` — **supported**, read-write-layout, proof `unit-smoke`: Report design planning and automatic layout. Commands: `report design-plan`, `report layout auto`.
- `report.drilldown` — **supported**, read-write-category-hierarchy, proof `unit-smoke`: Hierarchy drilldown authoring. Commands: `report drilldown set-hierarchy`.
- `report.drillthrough` — **supported**, read-write-page-binding, proof `schema-golden`: Same-report drillthrough page bindings. Commands: `report drillthrough set`, `report drillthrough show`, `report drillthrough clear`.
- `report.filters.categorical` — **supported**, read-write-categorical, proof `unit-smoke`: Categorical report/page/visual filters. Commands: `report filters list`, `report filters show`, `report filters add`, `report filters update`, `report filters delete`, `report filters clear`.
- `report.filters.numeric-range` — **supported**, read-write-advanced-comparison, proof `schema-golden`: Numeric range report/page/visual filters. Commands: `report filters list`, `report filters show`, `report filters add`, `report filters update`, `report filters delete`.
- `report.filters.relative-date` — **supported**, read-write-between-date-expressions, proof `schema-golden`: Relative-date report/page/visual filters. Commands: `report filters list`, `report filters show`, `report filters add`, `report filters update`, `report filters delete`.
- `report.filters.topn` — **supported**, read-write-visual-subquery, proof `schema-golden`: TopN visual filters ordered by a measure. Commands: `report filters list`, `report filters show`, `report filters add`, `report filters update`, `report filters delete`, `report visuals set-topn-guard`.
- `report.intent-parser` — **supported**, deterministic-json-markdown-normalization, proof `unit-smoke`: Structured report intent parsing. Commands: `report plan`.
- `report.interaction-default-reset` — **supported**, read-write-reset-to-default, proof `unit-smoke`: Interaction Default/reset semantics. Commands: `report interactions reset`.
- `report.interactions.overrides` — **supported**, read-write-explicit-overrides, proof `unit-smoke`: Explicit visual interaction overrides. Commands: `report build`, `report interactions list`, `report interactions show`, `report interactions set`, `report interactions disable`.
- `report.pages` — **supported**, read-write, proof `unit-smoke`: Report pages and layout metadata. Commands: `report pages list`, `report pages show`, `report pages add`, `report pages update`, `report pages reorder`, `report pages set-active`, `report pages delete-empty`.
- `report.slicer-authoring` — **supported**, generated-clean-state-desktop-golden-pending, proof `desktop-golden-pending`: Generated basic, dropdown, and between slicers. Commands: `report visuals catalog`, `report visuals add`, `report visuals set-bindings`, `report build`, `report slicers list`, `report slicers show`, `report slicers clear`.
- `report.slicer-clear` — **supported**, read-write-clear-only, proof `unit-smoke`: Slicer inventory and persisted-selection clear. Commands: `report slicers list`, `report slicers show`, `report slicers clear`.
- `report.slicer-sync-authoring` — **planned**, unsupported, proof `unit-smoke`: Slicer sync groups.
- `report.themes` — **supported**, guarded-bundle-copy, proof `unit-smoke`: Theme, visual formatting, and master style bundles. Commands: `report themes show`, `report themes extract`, `report themes apply`, `report themes presets`, `report themes apply-preset`, `report visuals formatting list`, `report visuals formatting show`, `report visuals formatting extract`, `report visuals formatting apply`, `report visuals formatting set-text`, `report visuals formatting set-color`, `report style inspect`, `report style extract`, `report style apply`, `report style diff`.
- `report.tooltip-pages` — **planned**, unsupported, proof `unit-smoke`: Report tooltip pages.
- `report.visuals.category-share` — **supported**, generated-desktop-golden-pending, proof `desktop-golden-pending`: Generated pie and donut visuals. Commands: `report visuals catalog`, `report visuals add`, `report visuals set-bindings`, `report build`.
- `report.visuals.combo-pareto` — **supported**, generated-manual-desktop-canvas-refresh, proof `manual-desktop-canvas-refresh`: Generated line and clustered-column combo visual. Commands: `report visuals catalog`, `report visuals add`, `report visuals set-bindings`, `report build`.
- `report.visuals.generated` — **supported**, read-write-small-catalog, proof `schema-golden`: Generated core visuals. Commands: `report visuals catalog`, `report visuals add`, `report visuals set-position`, `report visuals set-bindings`, `report visuals set-object`, `report visuals set-display-name`, `report visuals delete`.
- `report.visuals.matrix` — **supported**, generated-desktop-golden-pending, proof `desktop-golden-pending`: Generated matrix visual. Commands: `report visuals catalog`, `report visuals add`, `report visuals set-bindings`, `report build`.
- `report.visuals.planned-types` — **planned**, unsupported, proof `unit-smoke`: Generated PBIR for non-catalog visual types.
- `report.visuals.role-maps` — **supported**, validated-catalog-and-dry-run-repair, proof `unit-smoke`: Fixture-backed visual role maps and runtime-parity repair. Commands: `report visuals catalog`, `report visuals add`, `report visuals set-bindings`, `report visuals repair-bindings`, `report spec validate`.
- `report.visuals.template-clone` — **supported**, guarded-copy, proof `unit-smoke`: Template visual clone. Commands: `report visuals clone`.
- `validation.microsoft-report` — **supported**, explicit-exact-official-validator, proof `unit-smoke`: Official Microsoft report validation backend. Commands: `validate`.
- `workflow.source-profile` — **supported**, plan-run-verify, proof `unit-smoke`: Deterministic staged source-profile workflow. Commands: `workflow plan`, `workflow run`, `workflow verify`.
- `workflow.synthetic-source` — **supported**, shared-m-expressions-with-scale-and-seed, proof `schema-golden`: Offline deterministic synthetic source swap. Commands: `workflow synthesize`.
<!-- powerbi-cli:features:end -->

Start with focused capabilities instead of guessed commands:

```bash
pbi --json capabilities
pbi features list --json
pbi features list --for unsupported --json
pbi features list --for drillthrough --json
pbi --json capabilities --for scaffold --compact
pbi --json capabilities --for schema
pbi --json capabilities --for profile
pbi --json capabilities --for "report build" --compact
pbi --json capabilities --for "report spec"
pbi --json report spec schema
pbi --json report spec explain --schema <schema.json> --spec <dashboard.json>
pbi --json capabilities --for inspect --compact
pbi --json capabilities --for validate --compact
pbi --json capabilities --for lint --compact
pbi lint --rules --json
pbi lint --explain dax.reference_self --json
pbi lint --explain dax.format_missing --json
pbi lint --explain m.duplicate_step_name --json
pbi --json capabilities --for diff --compact
pbi --json capabilities --for package
pbi --json capabilities --for dax
pbi --json capabilities --for "model dax execute" --compact
pbi --json capabilities --for "model live export-tmdl" --compact
pbi --json capabilities --for calculated-columns
pbi --json capabilities --for advanced
pbi --json capabilities --for partitions
pbi --json capabilities --for "workflow synthesize"
pbi --json capabilities --for source-template
pbi --json capabilities --for rebind
pbi --json capabilities --for theme
pbi --json capabilities --for style
pbi --json capabilities --for wireframe
pbi --json capabilities --for semantic-model
pbi --json capabilities --for add-static
pbi --json capabilities --for report
pbi --json capabilities --for handoff

# Generated catalog paths not covered by the focused queries above.
pbi guid --json
pbi package export-plan --project build/sales --json
pbi robot-docs guide
pbi robot-docs render --check
pbi --robot-triage
pbi robot-triage
pbi integrations status --json
pbi integrations install --allow-network --json
pbi skill status --json
pbi skill install --json
pbi desktop bridge status --json
pbi desktop bridge reload --project build/sales --pid 1234 --json
pbi desktop bridge screenshot-page --project build/sales --pid 1234 --page ReportSection --out proof/page.png --json
pbi desktop bridge screenshot-all --project build/sales --pid 1234 --out-dir proof/pages --json
pbi schema normalize examples/sales.schema.json --out build/sales.schema.normalized.json --json
pbi profile summarize build/sales.profile.json --json
pbi model columns show --project build/sales --handle column:FactSales:Revenue --json
pbi model columns set-sort-by --project build/sales --table DimDate --column Month --by MonthNumber --dry-run --json
pbi model calculated-columns show --project build/sales --handle 'column:FactSales:Revenue Band' --json
pbi model calculated-columns update --project build/sales --handle 'column:FactSales:Revenue Band' --expression 'IF(''FactSales''[Revenue] >= 5000, ""High"", ""Standard"")' --dry-run --json
pbi model calculated-columns delete --project build/sales --handle 'column:FactSales:Revenue Band' --dry-run --json
pbi model measures update --project build/sales --handle 'measure:FactSales:Total Revenue' --expression 'SUM(''FactSales''[Revenue])' --dry-run --json
pbi model measures delete --project build/sales --handle 'measure:FactSales:Average Revenue' --dry-run --json
pbi model relationships list --project build/sales --json
pbi model relationships show --project build/sales --handle <relationship-handle> --json
pbi model relationships update --project build/sales --handle <relationship-handle> --cross-filtering-behavior bothDirections --dry-run --json
pbi model relationships delete --project build/sales --handle <relationship-handle> --dry-run --json
pbi model dax bridge-plan --project build/sales --json
pbi model roles list --project build/sales --json
pbi model perspectives list --project build/sales --json
pbi model cultures list --project build/sales --json
pbi model expressions list --project build/sales --json
pbi model roles show --project build/sales --handle role:Safety --json
pbi model perspectives show --project build/sales --handle perspective:Executive --json
pbi model cultures show --project build/sales --handle culture:de-CH --json
pbi model expressions show --project build/sales --handle expression:RefreshDate --json
pbi source-template show --project build/sales --handle source-template:FactSales:FactSales --json
pbi report design-plan --project build/sales --json
pbi report tree --project build/sales --json
pbi report find --project build/sales --kind visual --json
pbi report cat --project build/sales --handle visual:ReportSectionOverview:VisualContainerSalesKpi --json
pbi report query --project build/sales --selector kind:visual --json
pbi report audit --project build/sales --json
pbi report sanitize plan --project build/sales --json
pbi report sanitize apply --project build/sales --dry-run --json
pbi report layout auto --project build/sales --page page:ReportSectionOverview --template overview --dry-run --json
pbi report pages show --project build/sales --handle page:ReportSectionOverview --json
pbi report pages clone --project build/sales --from page:ReportSectionOverview --new-name ReportSectionOverviewCopy --visual-prefix Copy --dry-run --json
pbi report drillthrough show --project build/sales --page page:ReportSectionOverview --json
pbi report drillthrough clear --project build/sales --page page:ReportSectionOverview --dry-run --json
pbi report bookmarks reorder --project build/sales --order bookmark:A,bookmark:B --dry-run --json
pbi report bookmarks delete --project build/sales --handle bookmark:OldView --dry-run --json
pbi report filters delete --project build/sales --handle filter:report:main:ReportSegmentFilter --dry-run --json
pbi report themes presets list --json
pbi report themes apply-preset --project build/sales --preset risk-dashboard --dry-run --json
pbi report style extract --project corp/template --out master-style.json --json
pbi report style apply --project build/generated --bundle master-style.json --dry-run --json
pbi report visuals formatting conditional-formatting list --project build/sales --json
pbi report visuals formatting conditional-formatting show --project build/sales --handle <visual-handle> --include-raw --json
pbi report visuals add-card --project build/sales --page page:ReportSectionOverview --measure "FactSales.Total Revenue" --title "Revenue Card" --x 40 --y 40 --width 200 --height 120 --value-font-size 20 --category-font-size 9 --word-wrap --dry-run --json
pbi report visuals add-slicer --project build/sales --page page:ReportSectionOverview --field "DimCustomer.Segment" --title "Segment" --x 40 --y 40 --width 240 --height 80 --mode Dropdown --single-select --dry-run --json
pbi report visuals add-textbox --project build/sales --page page:ReportSectionOverview --title "Reading guide" --paragraphs-file guide.txt --x 40 --y 520 --width 400 --height 120 --dry-run --json
pbi report visuals set-topn-guard --project build/sales --handle <visual-handle> --field DimCustomer.CustomerName --order-by "FactSales[Total Revenue]" --top 28 --dry-run --json
pbi report visuals set-object --project build/sales --handle <visual-handle> --object categoryLabels --property fontSize --value 20 --dry-run --json
pbi report visuals set-display-name --project build/sales --handle <visual-handle> --role Values --display-name "Rate zuletzt (BU je 1'000 FTE)" --dry-run --json
```

`report layout auto --template` uses the deterministic twelve-column design
grid and eleven named page templates: `overview`, `time-series`, `ranking`,
`distribution`, `comparison`, `detail-table`, `drillthrough-detail`,
`exception-list`, `matrix-focus`, `scatter-focus`, and
`kpi-strip-trend-breakdown`. Each template supplies named slots, preferred
visual families, and minimum-size diagnostics. The command returns an
SVG-free JSON preview with overlap/minimum-size invariants and accepts standard
(1280x720), wide (1920x1080), or explicit `--page-size` and `--grid` values;
mutations support `--dry-run`, `--out-dir`, and guarded `--in-place`. Legacy
`--preset overview|analysis|detail|grid` values remain aliases for the named
templates.

`report wireframe export` keeps the JSON wireframe baseline and can render the
same resolved grid and deep-inspection visual geometry as deterministic SVG or
HTML. SVG output is one file per page (use an output directory for a
multi-page report); HTML embeds every page with a stable index. Use
`--dry-run` to review artifact bytes without writing, or `--out` to publish
outside the PBIP project. CSS is embedded and no network or external assets
are used.

A focused `--for` response returns the matching commands and small shared
contract fields. It deliberately leaves the large unrelated schema/visual
catalogs null and names them in `omittedCatalogs`; run the returned
`fullContractCommand` only when those catalogs are actually needed.
When the canonical command path is already known exactly, append `--compact`
to receive only its path, usage, flags, examples, proof level, follow-up fields,
and output schema.

## Authoring Loop

Use the compose-free loop that is implemented today. Keep every intermediate
artifact inspectable and let each response provide the next exact command:

```text
schema validate -> profile infer -> report plan -> report spec validate
-> report build -> triage -> report visuals add-card/add-slicer/set-object
-> validate --strict --backend all -> desktop open
```

The final `desktop open` step is an opt-in Windows oracle operation. On Linux
and macOS it returns `unsupported_feature`; local validation and schema/golden
proof remain valid but do not claim Desktop canvas or refresh compatibility.

The 2026-09-04 feature catalog has 52 IDs (46 supported, 6 planned). Keep the
proof level from `features list --json` with every claim:

| status / proof | feature IDs |
|---|---|
| supported / `unit-smoke` | `agent.codex-skill-distribution`, `desktop.dax-query-execution`, `desktop.live-tmdl-export`, `desktop.window-evidence`, `integrations.microsoft-toolchain`, `model.advanced-readback`, `model.calculated-columns`, `model.columns`, `model.dax-static-analysis`, `model.measures`, `model.relationships`, `model.source-templates`, `model.static-control-tables`, `model.tables`, `package.pbix-pbit-boundary`, `profile.data-profile-v2`, `quality.lint-rule-registry`, `quality.model-completeness-lint`, `report.bookmarks.readback`, `report.conditional-formatting`, `report.dashboard-spec-v2`, `report.design-layout`, `report.drilldown`, `report.filters.categorical`, `report.intent-parser`, `report.interaction-default-reset`, `report.interactions.overrides`, `report.pages`, `report.slicer-clear`, `report.themes`, `report.visuals.role-maps`, `report.visuals.template-clone`, `validation.microsoft-report`, `workflow.source-profile` |
| supported / `schema-golden` | `model.partition-grouped-rank`, `report.drillthrough`, `report.filters.numeric-range`, `report.filters.relative-date`, `report.filters.topn`, `report.visuals.generated`, `workflow.synthetic-source` |
| supported / `desktop-golden-pending` | `desktop.reference-harvest`, `report.slicer-authoring`, `report.visuals.category-share`, `report.visuals.matrix` |
| supported / `manual-desktop-canvas-refresh` | `report.visuals.combo-pareto` |
| planned / `unit-smoke` | `desktop.canvas-check`, `desktop.refresh-check`, `report.bookmark-mutations`, `report.slicer-sync-authoring`, `report.tooltip-pages`, `report.visuals.planned-types` |

Key live surfaces include package inspect/extract/import/source-pack/work-pack/export-plan,
schema validate/normalize (including bounded `$include` composition), profile
infer/validate/summarize, deterministic report planning, declarative report spec
validation/normalization, report build from schema/profile/spec inputs, scaffold, shallow/deep
inspect, semantic measure,
calculated-column, and relationship diff, report wireframe JSON/SVG/HTML export,
measure list/show/add/update/delete, static DAX dependencies/lint, explicitly
opted-in bounded DAX query execution against an exact already-open Desktop
PBIP/PBIX, guarded TMDL-only semantic-model export from that same exact live
engine through the pinned local Microsoft MCP,
advanced semantic-model inventory plus roles/perspectives/cultures/expressions
readback, calculated-column
list/show/add/update/delete, relationship list/show/add/update/delete,
partition list/show, source-template list/show/add/apply for SQL Server,
PostgreSQL, ODBC, Excel, CSV, folder, SharePoint/OneDrive, and closed-grammar
generic-M rebind metadata,
handoff rebind-plan and offline handoff rebind-check, fixture normalize/verify,
managed desktop open/close plus one-shot desktop open-check/screenshot and
Linux-capable desktop harvest-reference,
report page list/show/add/update/reorder/set-active/
delete-empty, report visual list/show/catalog/add/clone/delete, visual set-position,
existing-visual set-bindings, report filter list/show/add/update/delete/clear,
fixture-backed visual role maps plus dry-run binding repair proposals,
report slicer list/show/clear, report interaction list/show/set/disable/reset, report bookmark
list/show plus metadata-only display-name/reorder/delete, raw report theme
show/extract/apply bundles, master report style inspect/extract/diff/apply,
visual
formatting list/show/extract/apply bundles, visual formatting set-text for
title/legacy-alt-text cleanup, conditional-formatting readback list/show, handoff
check, lint plus registry list/explain, strict validate, doctor, version, robot docs, robot triage,
capabilities, and `features list`.
Treat filter sort and arbitrary expression updates, bookmark state capture/create/update/grouping,
slicer selection/sync mutation, unsupported
slicer modes, style
drift lint, conditional formatting authoring,
unsupported visual families, and richer typed per-visual formatting commands as
unavailable unless `features list` and `capabilities` both advertise them as
supported.

## Rules For Agents

- Use `--json` for reads and mutations.
- Run `powerbi-cli --json capabilities` before guessing command shape; it also
  advertises architecture guardrails for contributors and subagents.
- Run `powerbi-cli features list --json` before attempting advanced report
  behavior. `capabilities` answers "what syntax exists"; `features list`
  answers "what Power BI feature is supported, read-only, planned, or refused."
- Treat stdout as data and stderr as diagnostics.
- Success payloads are family-specific. Semantic mutation results and `report
  build` expose `changes[]`; readers may not. Validation/result payloads can use
  `ok:false` plus a nonzero `exitCode` on stdout. CLI errors have the stable stderr shape
  `{error:{code,exitCode,message,hint?,suggestedCommands?}}`.
- Treat every `next[]` and `suggestedCommands[]` entry as an executable
  `powerbi-cli` command template. Read prose from `instructions[]` or `notes[]`.
- Prefer CLI semantic commands over direct PBIR/TMDL file edits.
- Keep one canonical working project and one reusable QA output. Use Git or a
  deliberate backup for rollback; do not accumulate `v2`, `v3`, and other
  same-title project copies as an editing strategy.
- Rebuild that canonical generated project with `report build --force`; the
  cleanup is manifest-bounded, preserves user-added files, and clears Windows
  read-only attributes on generated OneDrive directories before removing them.
- Use handles returned by `inspect`, list, or show commands instead of guessed
  PBIR folder names or TMDL paths.
- Semantic-model handles percent-encode literal `%` and `:` inside table,
  measure, column, and partition components as `%25` and `%3A`. Always reuse
  returned handles instead of constructing them by hand.
- Delete visual containers with `report visuals delete`, never by removing
  `visual.json` directly. The command handles Windows/OneDrive read-only
  directory attributes and restores `visual.json` if the enclosing directory
  cannot be removed.
- `report visuals formatting set-text` synchronizes existing PBIR title
  containers and the generated `powerbi-cli.placeholderTitle` annotation.
- Mutate with explicit output directories or `--dry-run` when the command
  provides it. Do not assume in-place edits are safe.
- After any mutation, run generated follow-up commands such as
  `inspectCommand`, `validateCommand`, `readbackCommand`, `handoffCheckCommand`,
  or `desktopOpenCheckCommand`.
- Validate before moving a project between home and work machines.
- Do not claim Power BI Desktop compatibility from local validation alone. Use
  Desktop open/save proof when the claim matters.
- Prefer `model dax execute` over UI automation when a bounded live DAX query is
  sufficient. It requires Windows, an exact already-open PBIP/PBIX,
  `POWERBI_DESKTOP_ORACLE=1`, `--allow-data-read`, and an `EVALUATE` or `DEFINE
  ... EVALUATE` query. Treat returned rows as sensitive, keep the default bounds
  unless the task justifies widening them, and never infer canvas/refresh proof
  from a successful query. Its live preflight ignores only the report and
  semantic-model artifacts' root `.pbi/` runtime directories. Strict offline
  validation, packaging, workflow, and handoff continue to reject those files.
- Use `model live export-tmdl` when a PBIX semantic model must become readable
  TMDL before rebuilding or diagnosing it. It requires Windows, the exact
  already-open PBIP/PBIX document, `POWERBI_DESKTOP_ORACLE=1`,
  `--allow-model-read`, the pinned Modeling MCP integration, and a destination
  that does not yet exist. The command shares the exact document/process/engine
  matcher with live DAX, connects only to the validated local engine port, runs
  MCP read-only, exports into a private sibling quarantine, validates bounded
  UTF-8 TMDL shape, links/reparse points, and credential-like text, reaps the MCP
  process tree, then atomically publishes the fresh directory. Treat DAX, Power
  Query source expressions, and static table values in the export as sensitive
  model metadata. The result is only `definition/` TMDL; do not call it a report
  export or full PBIX-to-PBIP conversion.
- Separate Desktop refresh proof from accepting a Desktop save round-trip.
  Saving can normalize many otherwise unchanged PBIP files, add automatic date
  tables, cultures, diagram metadata, and local `.pbi` caches. After a proof
  session, review the full diff, remove unintended generated sidecars and model
  additions, then rerun strict validation before committing. Never commit the
  noisy save merely because refresh succeeded.
- After removing Desktop-created automatic date tables, run `validate --strict`.
  It rejects dangling TMDL `variation.relationship` and
  `variation.defaultHierarchy` references instead of leaving Desktop to fail on
  open.
- Keep slicers at least 76 px high. `report spec validate` and
  `validate --strict` reject shorter slicers because the official Power BI
  report validator does too; this catches clipped or overlapping controls before
  Desktop review.
- Do not add real data, credentials, caches, `.pbix`, or `.pbit` files to a
  home-authored project.
- Do not use package extraction as a way to smuggle imported data caches into a
  home project. Keep only source metadata unless the user explicitly requests a
  quarantine inspection outside the project.
- Treat package-extraction limits as a security boundary. Defaults are 10,000
  entries, 256 MiB per entry, 2 GiB total uncompressed, and 200:1 compression;
  raise them only with the matching explicit `--max-*` flag after inspection.
- Treat `capabilities.limits` as the input-surface safety contract. Schema,
  profile, spec, JSON bundle, intent, and DAX/text files have fixed byte limits,
  strict UTF-8 decoding, and symlink refusal. Profile row inference consumes
  bounded CSV/JSON rows through the same contract. Planned includes, PNG
  resources, ops, snapshots, and harvested fragments already have reserved
  numeric limits and typed guards in `docs/input-safety-contract.md`; do not
  bypass those guards or silently strip rejected content when adding a command.
- Schema and v2 dashboard specs may use bounded, relative `$include` fragments.
  Use `schema normalize` and `report spec normalize` when you need one
  canonical artifact for review, caching, or parity checks. Their
  `normalizedFrom[]` values are root-relative, sorted, and deterministic;
  traversal, symlink, cycle, depth, count, and fragment-size failures are
  refusals, not best-effort omissions.
- The internal operation-plan spine is `powerbi-cli.ops.v1`: typed `op` records
  use the same stable page, visual, filter, and percent-encoded semantic-model
  handles as CLI readbacks. Plans validate references and stage order before a
  temporary-directory transaction is published; no public `apply --ops` command
  is advertised until the individual mutation kernels are converted.
- `package source-pack` refuses every unknown file and every file under a
  dot-directory. Do not rename an extra file to an allowlisted extension to make
  it travel; remove it or carry an independently reviewed artifact separately.
- `package work-pack` is the separate materialized work-machine variant. It
  applies the same strict allowlist and content scans, requires every partition
  to be a recognized credential-free live connector accepted by `handoff check
  --target work`, and packages source metadata only—never imported rows, caches,
  PBIX files, or local settings. Without `--out`, it writes the sibling
  `<project>-work.pbit`.
- If a command refuses an unsupported visual, format, source, or model feature,
  preserve the refusal. `error.code = "unsupported_feature"` is a stop sign, not
  an invitation to patch raw PBIR/TMDL by memory.

## Proof Matrix

The closed, ordered `proofLevel` vocabulary is `unit-smoke < schema-golden <
desktop-golden-pending < manual-desktop-canvas-refresh <
desktop-canvas-refresh`. `desktop-launch` and `desktop-window` are observation
stages, not proof levels. The capabilities catalog exposes them as
`observedStage`; current Desktop command payloads still place these legacy stage
names in `proof.level`, so interpret that field as an observation stage until the
Desktop hardening work migrates it.

Desktop evidence committed under `testdata/desktop-proof/` uses
`powerbi-cli.desktop-proof.v1`. Each record links exact `features[].id` values
through `signals.featureIds`; `features list` takes the maximum validated record
level and catalog baseline. The loader rejects records whose `proofLevel`
exceeds their signals. In particular, a current artifact, rendered canvas,
completed refresh, absent issue dialogs, and matched expected values are all
required for manual canvas/refresh proof together with
`signals.manualReview=true`; automated proof instead requires
`signals.automated=true`.

| Claim | Minimum proof | Stronger proof |
|---|---|---|
| Project is structurally present | `pbi --json validate <project>` | `validate --strict` once available |
| Project is offline-safe | `pbi --json handoff check <project>` | `validate --strict` plus Desktop open-check |
| Live-source PBIP is safe to take to its work network | `pbi --json handoff check <project> --target work` | Work-network refresh plus Desktop canvas inspection |
| PBIX/PBIT contains usable source metadata | `package inspect` plus `package extract` into a temporary folder | `package import` succeeds and `validate --strict` passes on the imported project |
| Model object exists | `inspect --deep` or list/show command | Desktop open-check |
| DAX references are locally plausible | `model dax dependencies` and `model dax lint` | Desktop/XMLA/Fabric engine validation |
| A lint or audit finding is understood | `lint --explain <rule-id>` | Inspect the affected artifact and run the rule's remediation command |
| One bounded DAX query executes in the open model | `model dax execute` with both opt-ins, exact-project match, `ok=true`, and no truncation relevant to the assertion | Repeat the targeted query after refresh; canvas/render proof remains separate |
| One live PBIP/PBIX semantic model was exported to guarded TMDL | `model live export-tmdl` with both opt-ins, exact-document match, validated output hash/counts, and `integration.cleanup.childrenReaped=true` plus `pumpsJoined=true` | Wrap the reviewed TMDL in a PBIP semantic-model artifact and run strict local/official/Desktop proof; report pages remain separate |
| Advanced semantic metadata exists | `model advanced inventory` or the relevant roles/perspectives/cultures/expressions list/show command | Desktop open/save round-trip |
| Page metadata/order was written/read locally | `report pages add/update/reorder/set-active/delete-empty` dry-run/apply plus `report pages list/show` and `validate --strict` | Desktop open/save round-trip |
| Visual was created/read locally | `report visuals add` dry-run/apply plus `report visuals show` and `validate --strict` | Desktop-authored golden fixture match and Desktop open/save round-trip |
| Visual was cloned/read locally | `report visuals clone` dry-run/apply plus `report visuals show` and `validate --strict` | Desktop open/save round-trip, especially for Desktop-authored template visuals |
| Visual was deleted locally | `report visuals delete` dry-run/apply plus `report visuals list` and `validate --strict` | Desktop open/save round-trip |
| Visual binding was written/read locally | `report visuals set-bindings` dry-run/apply plus `report visuals show` and `validate --strict` | Desktop-authored golden fixture match and Desktop open/save round-trip |
| Pie, donut, matrix, or Basic/Dropdown slicer binding/canvas baseline has prior manual proof | `testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json` plus exact current `visual.json` assertions, `validate --strict`, `handoff check`, and `fixture verify` against `catalog-proof.summary.json` | Re-open/refresh/save the current title-bearing bytes; Between slicers require that same Desktop proof before a compatibility claim |
| Same-report one-column drillthrough matches the public schema-golden shape | `report drillthrough set/show/clear` shape/readback tests plus the public page schema and Desktop-authored reference shape | Reproducible Desktop well/context-menu/navigation/carried-filter proof; visual-action, multi-field, and cross-report fixtures before widening scope |
| Visual formatting bundle was applied | `report visuals formatting extract/apply` dry-run/apply plus `report visuals formatting show` and `validate --strict` | Desktop-authored golden fixture match and Desktop open/save round-trip |
| Visual interaction override was written/read locally | `report interactions set/disable/reset` dry-run/apply plus `report interactions list/show` and `validate --strict` | Desktop open/save round-trip with interaction inspection |
| Bookmark metadata was edited locally | `report bookmarks set-display-name/reorder/delete` dry-run/apply plus `report bookmarks list/show` and `validate --strict` | Desktop open/save round-trip with bookmark pane inspection |
| Categorical filter was added or updated locally | `report filters add/update` dry-run/apply plus `report filters list/show` and `validate --strict` | Desktop canvas/open-save round-trip with filter pane inspection |
| Numeric range filter matches the schema-golden contract | `report filters add --min/--max` dry-run/apply plus exact `show` shape and `validate --strict` | Desktop canvas/open-save round-trip for closed and open-ended ranges at every scope |
| TopN filter matches the schema-golden contract | visual-scoped `report filters add --top/--bottom --by` dry-run/apply plus exact `show` subquery shape and `validate --strict` | Desktop-authored golden comparison plus canvas/open-save round-trip for measure-ranked top and bottom filters |
| Relative-date filter matches the schema-golden contract | `report filters add --relative --unit --span` dry-run/apply plus exact `show` expression shape and `validate --strict` | Desktop-authored golden comparison plus canvas/open-save round-trip for rolling and calendar variants at every scope |
| Report theme bundle applied | `report themes show` fingerprint and `validate --strict` | Desktop open/save round-trip with visual inspection |
| Report style bundle applied | `report style diff/apply` plus `report visuals formatting list/show` and `validate --strict` | Desktop open/save round-trip with visual inspection |
| Golden fixture summary is stable | `fixture normalize` plus `fixture verify` against a committed summary | Same summary captured from a Desktop-authored fixture |
| Desktop process launched for the PBIP | `desktop open-check` with `POWERBI_DESKTOP_ORACLE=1` on Windows, canonical `proof.level=unit-smoke`, and `proof.observedStage=desktop-launch` or `desktop-window` | Exact matching titled-window observation |
| Matching Desktop window title appeared | `desktop open-check` with `proof.observedStage=desktop-window`, `windowObserved=true`, and `titleMatched=true`; matching is exact on the normalized project stem | Manual/screen-agent canvas inspection |
| Reviewable screen evidence was captured | `desktop screenshot <project> --out <outside-project.png>` with `screenshot.captured=true` and `screenshot.foregroundVerified=true` | Human or screen-agent review of the PNG plus refresh/canvas inspection |
| Report canvas rendered and refreshed correctly | Manual Desktop canvas/refresh inspection and a committed proof record | Future `desktop-canvas-refresh` automation; window/title/screenshot signals alone are insufficient |
| Work-machine rebind is prepared | `source-template add` plus `handoff rebind-plan` and post-apply `handoff rebind-check` | successful Desktop refresh at work |

Always name what remains unproven. Validation can prove local file invariants;
Desktop proves Power BI compatibility.

## Common Workflows

### Package Or Extract A Handoff Safely

```bash
pbi --json package inspect template.pbit
pbi --json package extract template.pbit --out-dir build/template-source
pbi --json handoff check build/sales
pbi --json package source-pack --project build/sales --out build/sales-source.pbit
pbi --json package work-pack --project build/sales-live
```

Extraction removes partial output if the entry-count, per-entry, total-size, or
compression-ratio budget is exceeded. Source packing permits only root `.pbip`,
report PBIR/definition JSON, semantic-model PBISM/TMDL, registered/shared JSON
resources, generated `.gitignore`, `POWERBI_HANDOFF.md`,
`powerbi-cli.manifest.copy.json` sidecars, and root `profile*.json`/`*.profile*.json`
metadata.
Files under `.git`, `.vscode`,
`.powerbi-cli`, or any other dot-directory are refused. The command scans all
included content before creating the archive; credential-like content is unsafe,
PII-suspect row literals require review, data-bearing profile v2 documents are
refused, and non-dummy or unverified partition sources are refused.

### Export A Live PBIX Semantic Model To TMDL

Use this only on Windows when the PBIX/PBIP semantic model must be inspected or
rebuilt as editable source. Keep one managed Desktop session and close it after
the live operations:

```bash
export POWERBI_DESKTOP_ORACLE=1
pbi --json desktop open SourceProfile.pbix
pbi --json model live export-tmdl --document SourceProfile.pbix --out-dir build/source-profile-model --allow-model-read
pbi --json model dax execute --project SourceProfile.pbix --query 'EVALUATE ROW("Value", 1)' --allow-data-read --max-rows 10
pbi --json desktop close
```

Require a fresh output directory, `output.kind=tmdl-definition-export`, a
nonempty `output.sha256`, and complete MCP cleanup. The local Modeling MCP
server receives one closed canonical `localhost:<port>` request and writes the
quarantined TMDL; its response exposes only an opaque connection name, so the
CLI revalidates the exact local Desktop/model process, creation, workspace, and
port identity before connection and after export instead of claiming endpoint
readback. The CLI does not send the PBIX file to a hosted MCP service.
The pinned preview may independently emit Microsoft usage telemetry. Never
commit the live export before reviewing Power Query source expressions and
static values. A successful export proves semantic-model source extraction,
not report-page extraction, refresh, canvas behavior, or full PBIX-to-PBIP
materialization.

### Build A Dashboard From Schema/Profile/Spec

Use this as the default data-agnostic dashboard loop. It keeps report intent in
an explicit dashboard spec instead of relying on hidden inference:

Dashboard spec keys are strict. Before authoring one, run `report spec fields`
without a schema for the allowed-key catalog or with `--schema` for both the
catalog and exact binding references. An unknown key returns
`spec.unknown_field` with an RFC 6901 pointer and, when available, a
`didYouMean` correction; do not bypass that diagnostic with raw PBIR edits.
The accepted schemas are `powerbi-cli.dashboard.v1` and the v2 superset. A v2
section is safe to retain before its compiler lands: compiled validation/build
will return `unsupported_feature` with the owning T3 bead id, never silently
discard it. `examples/sales.dashboard.v2.json` is the minimal compiled-v2
  reference. Migrate a validated v1 spec with
  `report spec upgrade --spec <v1.json> --out <v2.json>`; it rewrites only
  `/schema`, preserves array order, recursively normalizes object keys, and
  reports every transformed pointer. Use `--dry-run` to inspect the v2
  document without writing; unknown v1 keys fail with
  `spec.unknown_field` before output.
  `report spec validate` writes validation failures to stdout as structured
  `errors[]` objects (`code` and `message` are required; `pointer`,
  `didYouMean`, `hint`, and `suggestedCommands` are optional). Consumers should
  read `errors[].message`, never treat an entry as a bare string. See
  `capabilities.responseShapes.reportSpecValidate` for the machine contract.

`report spec schema --json` emits the draft 2020-12 JSON Schema generated from
the strict v1/v2 key tables. `report spec explain --schema <schema.json>
--spec <dashboard.json> [--profile <profile.json>] --json` previews the staged
typed operation plan, stable handles, resolved layout/defaults, unsupported
sections, and proof commands without writing files.

For a composed spec, normalize it before handing it to another agent or build
stage, then validate the normalized file. `report spec normalize` accepts the
same positional path or `--spec` spelling as validation and writes a canonical
JSON document plus `normalizedFrom[]` provenance. The schema-side equivalent is
`schema normalize`; report build and artifact parity already normalize their
schema/spec inputs internally, so an inline and include-composed document are
expected to be byte-equivalent when their content is equivalent.

```bash
pbi --json schema validate examples/sales.schema.json
pbi --json schema normalize examples/sales.schema.json --out build/sales.schema.normalized.json
pbi --json profile infer --schema examples/sales.schema.json --out examples/sales.profile.json
pbi --json profile infer --schema examples/sales.schema.json --rows build/sales-rows.csv --out build/sales.profile.v2.json
pbi --json profile validate examples/sales.profile.json
pbi --json report plan --schema examples/sales.schema.json --profile examples/sales.profile.json --intent examples/intents/sales.intent.json --out build/sales.planned.dashboard.json
pbi --json report spec validate --schema examples/sales.schema.json --profile examples/sales.profile.json --spec examples/sales.dashboard.json
pbi --json report spec normalize examples/sales.dashboard.json --out build/sales.dashboard.normalized.json
pbi --json report spec upgrade --spec examples/sales.dashboard.json --out build/sales.dashboard.v2.json
pbi --json report build --schema examples/sales.schema.json --profile examples/sales.profile.json --spec examples/sales.dashboard.json --out-dir build/generic-sales --force
pbi --json validate --strict build/generic-sales
pbi --json handoff check build/generic-sales
pbi --json fixture verify build/generic-sales --expected testdata/golden/generic-sales.summary.json
```

For bounded profile statistics, pass `--rows <rows.csv|rows.json>` to
`profile infer`. The rows reader enforces the limits in
`docs/input-safety-contract.md`; CSV uses its first record as a header and JSON
accepts object records or a header-row array. Profile v2 emits null rates,
distinct counts, numeric/date min/max, time coverage, duplicate-key grain
conflicts, and type-coercion diagnostics. Literal top values are redacted by
default (`topValueCounts` and cardinality remain available). Only an explicit
`--include-data-values` may emit at most five bounded top values per column,
after credential/PII scanning; profiles stamped `dataValues:true` are
data-bearing and are reported by `handoff check` and refused by
`package source-pack`. `--redact` is retained as a deprecated no-op alias.
`profile summarize` additionally emits a deterministic `summary.shape` object
with facts, dimensions, date-table proposals, key candidates, high-cardinality
noise, and evidence strings. Shape evidence names the row-count ratio, numeric
column share, relationship/cardinality fan-out, and date coverage used; weak
signals return `kind=ambiguous` plus competing hypotheses instead of a guess.

`report plan` is implemented as a deterministic starter-spec planner. Give it a
schema, optional profile, and either `--intent <intent.md|intent.json>` or the
backward-compatible objective text, then validate the emitted spec before
`report build`. Intent v1 normalizes audience, questions, KPIs, comparisons,
periods, drill paths, alerts, filter dimensions, preferred archetypes, page
flow, and handoff requirements. KPI names resolve to exact model measures;
unresolved names return `spec.missing_input` with a pointer and candidates.
Fields not compiled by this starter planner remain in the response with an
owning-bead warning. It is not a substitute for reviewing generated report
intent or for Desktop compatibility proof.
The response's top-level `shape` and `decisions[]` model-shape entry reuse the
same profile/schema classifier. A date-like column without a related date
dimension is surfaced as a proposal rather than silently treated as a calendar.

When the compiler cannot safely infer a required value, it asks through a
structured `spec.missing_input` diagnostic instead of silently choosing a
visual type, binding, TopN order, drillthrough target, slicer column, semantic
color, or date for a measure pattern. Read `pointer`, `field`, and `reason`,
then run the returned `candidatesCommand` (normally
`powerbi-cli report spec fields --schema <schema.json> --json`) and repair that
pointer. The error also includes an `example` shape. Optional documented
defaults are listed in `defaultsApplied[]` in build/plan responses, so a
downstream agent can distinguish an intentional default from a missing input.

V2 proof requirements are compiled into `proofPlan` and the report build
`next[]` list. `proof.desktop.expectValues[]` becomes one bounded
`model dax execute` command per expectation, and each `proof.goldens[]` entry
becomes a `fixture verify` command. Proof planning is side-effect free: no
Desktop session, query, refresh, or fixture verification runs automatically.
On Linux and macOS, Desktop-dependent commands are listed in
`proofPlan.unavailable[]` with the Windows oracle instruction; the compiler
never claims a Desktop proof level that the host cannot deliver.

`report build` returns `compiled.ops`, flattened `changes[]`, and a `readback`
object keyed by stable `report:`, `page:`, `visual:`, `table:`, and `measure:`
handles. The embedded `scorecard.v1` is shared with `triage` and separates
native validation, Microsoft-validator availability, lint findings grouped by
severity, the fixed unavailable design-lint shape, offline handoff status, and
the honest proof level. Pass `--trace` when diagnosing a build to include the
deterministic `{op, ms}` planning trace; it is omitted by default so ordinary
responses stay small. The complete field contract is published at
`capabilities.responseShapes.scorecard.v1` and
`capabilities.responseShapes.reportBuild`.

### Scaffold From A Schema

```bash
pbi --json scaffold --schema examples/sales.schema.json --out-dir build/sales --force
pbi --json inspect build/sales
pbi --json validate build/sales
```

For the larger multi-page `regional-sales` archetype:

```bash
pbi --json scaffold --schema examples/archetypes/regional-sales.schema.json --out-dir build/regional-sales --force
pbi --json inspect build/regional-sales
pbi --json validate build/regional-sales
```

Read the `next` array in the scaffold response and prefer those generated
commands over remembered examples.

`scaffold --force` is safe only for a directory carrying the prior
`powerbi-cli.manifest.copy.json`. It deletes the exact scaffold artifacts from
that manifest, removes only empty generated directories, preserves user-added
files, and refuses an unmarked non-empty directory.

### Inspect Before Editing

```bash
pbi --json inspect build/sales
pbi --json validate build/sales
```

Use `inspect --deep` before report or model edits. It returns tables, columns,
measures, relationships, pages, visuals, bindings,
handles, hazards, and proof status.

M lint reports `m.duplicate_step_name` as an error when a partition or named
expression defines the same `let` step identifier more than once. Quoted
identifiers and the final step before `in` are included; comments and string
literals are ignored. Each finding includes the first and duplicate one-based
source positions. Use `pbi lint --explain m.duplicate_step_name --json` for the
remediation contract, then rename or remove the duplicate before opening the
project in Desktop. The warning-level `m.untyped_expansion` and
`m.unbuffered_reuse` rules also flag unsafe expansion and reused table values
without buffering; inspect their explanations before shipping a refresh
partition.

### Repair And Verify An Existing Dashboard

Use the exact command paths below instead of guessing shortened families:

```bash
pbi --json validate --strict build/sales
pbi --json model dax dependencies --project build/sales
pbi --json model dax lint --project build/sales
pbi --json lint --rules
pbi --json lint --explain dax.reference_self
pbi --json lint --explain m.duplicate_step_name
pbi --json report wireframe export build/sales
pbi report wireframe export build/sales --format svg --out proof/sales-wireframe --json
pbi report wireframe export build/sales --format html --dry-run --json
pbi --json report interactions list --project build/sales
pbi --json handoff check build/sales
```

The combined lint and triage scorecards also report model completeness
warnings: measures without an explicit format, malformed custom format
strings, visible relationship keys, suspicious both-direction
fact-to-dimension relationships, and columns unused by visuals, measures, or
relationships. Each finding has a stable handle and a remediation hint; use
model dax lint when only DAX and measure-format diagnostics are needed.

Use `report visuals list/show` handles for every visual mutation. Delete a
visual only with `report visuals delete --dry-run`, then an output copy or a
confirmed in-place mutation. Never leave an empty visual directory.
Keep the edit/test cycle on one canonical project path so Desktop title matching,
receipts, diffs, and handoff checks all refer to the same artifact.

Treat three report behaviors separately:

- A hierarchy drill changes one visual's category grain, such as branch to
  company. Use `report drilldown set-hierarchy` and verify the visual's drill
  controls in Desktop.
- Comparing several companies at once is not hierarchy drill. Bind company as
  the chart's Series/Legend field or use a multi-select company slicer, keeping
  year on the axis.
- Drillthrough navigates to a target page with filter context. Use `report
  drillthrough show` to inspect an existing target; do not substitute it for
  hierarchy drill or multi-series comparison.

After a source rebind, use `model dax execute` for bounded model assertions and
then inspect every changed page in Desktop. A successful DAX query does not
prove canvas interactions, drill controls, bubbles, or refresh.

### Author Measures

Use measure commands only when `capabilities` advertises them:

```bash
pbi --json capabilities --for measure
pbi --json model measures list --project build/sales
pbi --json model measures show --project build/sales --handle "measure:FactSales:Total Revenue"
pbi --json model measures add --project build/sales --table FactSales --name "Average Revenue" --expression "DIVIDE([Total Revenue], [Total Units])" --dry-run
pbi --json model measures add --project build/sales --table FactSales --name "Average Revenue" --expression "DIVIDE([Total Revenue], [Total Units])" --out-dir build/sales-v2
pbi --json diff build/sales build/sales-v2
pbi --json validate build/sales-v2
```

Use `--expression-file <path|->` for multiline DAX or awkward shell quoting.
Measure add/update also accepts `--format-string-definition <dax>` for a
dynamic format expression; static formats use `--format-string`.
Use `--in-place` only after the dry-run block is correct. For in-place delete,
pass `--confirm <measure-handle>`. These commands preserve and rewrite TMDL
structure and refuse update blocks with unsupported Desktop-authored TMDL
metadata; they do not execute DAX, so Desktop remains the semantic oracle.

### Author Calculated Columns

Use calculated-column commands only when `capabilities` advertises them:

```bash
pbi --json capabilities --for calculated-columns
pbi --json model calculated-columns list --project build/sales
pbi --json model calculated-columns add --project build/sales --table FactSales --name "Revenue Band" --expression "IF('FactSales'[Revenue] >= 10000, \"High\", \"Standard\")" --data-type string --dry-run
pbi --json model calculated-columns add --project build/sales --table FactSales --name "Revenue Band" --expression "IF('FactSales'[Revenue] >= 10000, \"High\", \"Standard\")" --data-type string --out-dir build/sales-v2
pbi --json diff build/sales build/sales-v2 --scope model.calculatedColumns
pbi --json validate build/sales-v2
```

Use `column:<table>:<name>` handles from list/show/inspect. Add requires
`--data-type`; update can change the DAX expression, data type, format string,
summarization, display folder, description, and hidden state. These commands edit
TMDL metadata and refuse update blocks with unsupported Desktop-authored lines;
Desktop remains the oracle and calculated-column expression changes may require
refresh when opened at work. Input type `date` is normalized to TMDL `dateTime`
and receives `formatString: "Short Date"` unless an explicit format string is
provided. Colon-bearing table and column names round-trip through percent-encoded
handles returned by the CLI.

### Author Tables And Columns

Use the generic semantic-model commands for typed table and column inventory
and guarded CRUD:

```bash
pbi --json capabilities --for "model tables"
pbi --json model tables list --project build/sales
pbi --json model tables show --project build/sales --handle table:FactSales
pbi --json model tables add --project build/sales --table DimSegment --column Code --data-type string --dry-run
pbi --json model tables add-calculated --project build/sales --table SalesAbovePlan --expression "FILTER('FactSales', 'FactSales'[Revenue] > 0)" --dry-run
pbi --json model tables rename --project build/sales --handle table:DimDate --new-name Calendar --rename-references --dry-run
pbi --json model tables delete --project build/sales --handle table:DimSegment --dry-run
pbi --json model expressions add --project build/sales --name SharedQuery --expression "#table(type table [Value = Int64.Type], {{1}})" --dry-run
pbi --json model expressions update --project build/sales --handle expression:SharedQuery --expression-file checks/query.m --dry-run
pbi --json model expressions delete --project build/sales --handle expression:SharedQuery --dry-run
pbi --json capabilities --for "model columns"
pbi --json model columns list --project build/sales
pbi --json model columns add --project build/sales --table FactSales --name Margin --data-type decimal --dry-run
pbi --json model columns update --project build/sales --handle column:FactSales:Revenue --format-string '$#,##0' --dry-run
pbi --json model columns delete --project build/sales --handle column:FactSales:Margin --dry-run
pbi --json diff build/sales build/sales-v2 --scope model.tables
pbi --json diff build/sales build/sales-v2 --scope model.columns
```

Table handles are `table:<name>` and column handles are
`column:<table>:<name>`; literal `%` and `:` in every component are encoded as
`%25` and `%3A`. Table rename refuses and lists relationship/DAX/variation
references unless `--rename-references` is explicit. Column updates refuse a
targeted block containing unknown Desktop-authored properties (including
annotations or extended properties) rather than dropping them. Every mutation
supports `--dry-run`, guarded `--in-place`, and isolated `--out-dir`; run the
returned inspect and validate commands after applying a plan.

`model tables add-calculated` writes an offline-safe DAX `calculated` partition;
Desktop may materialize its columns on refresh, so the interim no-columns
completeness check is deferred. Named M expressions use
`model expressions add/update/delete` with bounded input, newline-preserving
TMDL edits, duplicate-step linting, and unknown-metadata refusal.

### Add A Small Selector Or Lookup Table

Use the guarded static-table command for report controls such as a metric toggle
or for a compact non-sensitive lookup dimension:

```bash
pbi --json capabilities --for add-static
pbi --json model tables add-static --project build/sales --table Metric --column Metric --values-json '["Count","Cost"]' --dry-run
pbi --json model tables add-static --project build/sales --table Metric --column Metric --values-json '["Count","Cost"]' --in-place
pbi --json model tables add-static --project build/sales --table DimSegment --columns-json '["Code","Label"]' --rows-json '[["A","Alpha"],["B","Beta"]]' --dry-run
pbi --json model tables add-static --project build/sales --table DimSegment --columns-json '["Code","Label"]' --rows-json '[["A","Alpha"],["B","Beta"]]' --in-place
pbi --json model relationships add --project build/sales --from-table FactSales --from-column SegmentCode --to-table DimSegment --to-column Code --cross-filtering-behavior oneDirection --dry-run
pbi --json model partitions show --project build/sales --handle "partition:Metric:Metric"
pbi --json validate --strict build/sales
```

The selector form creates one disconnected string column with 1-100 unique
short labels. The lookup form creates 1-10 string columns and 1-100 short rows;
the first column is a unique key. It is intended for compact, non-sensitive
reference dimensions, not fact-data ingestion. It refuses replacement,
credentials, multiline cells, duplicate keys/rows, and arbitrary fact tables.
Relationships are deliberately separate: dry-run and add one with `model
relationships add`. Use a DAX `SELECTEDVALUE`/`SWITCH` measure to connect a
disconnected selector to report behavior; Desktop remains the DAX and
interaction oracle. Relationship add/update also exposes endpoint
`one|many` cardinalities, `active`/`inactive` state, and
`oneDirection`/`bothDirections`/`automatic` cross-filtering behavior; review
the returned metadata before relying on bidirectional filtering.

### Inspect Partitions And Handoff Safety

Use partition and handoff commands only when `capabilities` advertises them:

```bash
pbi --json capabilities --for partition
pbi --json model partitions list --project build/sales
pbi --json model partitions show --project build/sales --handle "partition:FactSales:FactSales"
pbi --json model partitions show --project build/sales --handle "partition:FactSales:FactSales" --include-source
pbi --json model partitions add-grouped-rank --project build/analytics --table Signals --group-by Segment --order-by Score --desc --rank-column GroupRank --eligible-when '[IsEligible] = true' --dry-run
pbi --json handoff check build/sales
pbi --json handoff check report/live.pbip --target work
```

Generated partitions should normally report `sourceKind: dummyMTable` and
`offlineSafety.safeForHome: true`. `handoff check` exits 10 on unsafe caches,
Power BI binaries, local settings, embedded data files, real connector
partitions, or credential-like partition source text. A literal `#table`
substring is not proof: the M expression must match the generated Source shape,
the model column list, supported literal types, and row arity. PII-suspect row
literals yield `status: review`. Partition show returns redacted previews by
default; `--include-source` is refused unless the partition status is `safe`.
For a canonical live-source report, use `--target work`: recognized connectors
are accepted. Unknown M can be explicitly declared model-derived with the
table-level TMDL annotation
`annotation PowerBICli_SourceKind = ModelDerived`; this is an author trust
contract, remains non-home-safe, and never overrides credential or other error
findings. Credentials, caches, embedded data, and unannotated unknown partition
sources still fail. Check `safeForWorkHandoff`, not
`safeForOfflineHandoff`, in that workflow.

For a disconnected refresh-time analytics table whose generated dummy source
already includes an `int64` rank placeholder, generate the standard per-group
rank chain with `model partitions add-grouped-rank`. The command accepts one or
more existing source-backed group columns, one order column, optional `--desc`,
and a bounded M row predicate. It buffers each group, assigns eligible rows
1-based ranks and ineligible rows zero, and explicitly retypes the result. It
refuses live/unknown/unsafe or multi-partition tables. Review `changes[].after`,
then run the returned lint and strict-validation commands; Desktop refresh and a
bounded DAX assertion remain required for semantic proof.

### Prepare Source Templates And Rebind Plans

Use source-template, rebind-plan, and rebind-check commands only when
`capabilities` advertises them:

```bash
pbi --json capabilities --for source-template
pbi --json source-template add --project build/sales --table FactSales --kind sql --server "<server>" --database "<database>" --schema dbo --object FactSales --dry-run
pbi --json source-template add --project build/sales --table FactSales --kind postgres --server "<server>" --database "<database>" --schema public --object "<object>" --dry-run
pbi --json source-template add --project build/sales --table FactSales --kind odbc --dsn "<dsn>" --database "<database>" --schema "<schema>" --object "<object>" --dry-run
pbi --json source-template add --project build/sales --table FactSales --kind excel --file "<workbook.xlsx>" --sheet FactSales --dry-run
pbi --json source-template add --project build/sales --table FactSales --kind csv --file "<file.csv>" --delimiter , --encoding 65001 --has-header true --dry-run
pbi --json source-template add --project build/sales --table FactSales --kind folder --path "<folder>" --pattern *.csv --dry-run
pbi --json source-template add --project build/sales --table FactSales --kind sharepoint --site-url "<siteUrl>" --library "<library>" --path "<path>" --dry-run
pbi --json source-template add --project build/sales --table FactSales --kind generic-m --m-template 'let Source = Sql.Database("{{powerbi-cli.placeholder:server}}", "{{powerbi-cli.placeholder:database}}") in Source' --dry-run
pbi --json source-template add --project build/sales --table FactSales --kind postgres --server "<server>" --database "<database>" --schema public --object "<object>" --out-dir build/sales-rebind
pbi --json source-template list --project build/sales-rebind
pbi --json handoff rebind-plan build/sales-rebind --out build/sales-rebind/work-machine-rebind.md
pbi --json source-template apply --project build/sales-rebind --handle source-template:FactSales:FactSales --server sql.example.internal --database Sales --out-dir build/sales-live
pbi --json handoff check build/sales-live --target work
pbi --json handoff rebind-check build/sales-live --partition partition:FactSales:FactSales
```

Source templates are sidecar metadata in `.powerbi-cli/source-templates.json`.
`source-template apply` materializes one template into a generated dummy partition.
For an intentional source-to-source retarget, `--replace-existing` also requires
the exact `--confirm <partition-handle>` and accepts only recognized credential-free
SQL, PostgreSQL, ODBC, external-file, or SharePoint sources. Unknown, web, credential-bearing,
and unconfirmed sources are refused. Excel templates select one worksheet or Excel
table, promote its headers, add explicit Power Query conversions from the model's
TMDL column types, and materialize an absolute workbook path; reapply or patch the
path after moving the project. Use placeholders for source identifiers at
home and configure database credentials only in Power BI Desktop at work. Current
Power BI Desktop releases include the Npgsql provider; only
Desktop releases before December 2019 or on-premises data gateway releases
before June 2025 require a separate Npgsql installation;
ODBC templates require a bare DSN name without `;`/`=` attributes and require the
named DSN there. The rebind runbook includes these prerequisites and post-refresh
checks. `--out` refuses to overwrite an existing runbook unless `--force` is
passed, and credential detection redacts response content and suppresses the
runbook write entirely.

After `source-template apply` on the work machine, run
`pbi --json handoff rebind-check build/sales-live`. This offline gate checks
every selected partition for a concrete non-placeholder source, validates
SQL/PostgreSQL/ODBC/SharePoint call shapes, probes only local paths, and runs
strict native validation. It emits stable per-partition findings and
`refresh.status: not-run`; follow its `desktop open` command for the separate
authenticated refresh and canvas proof. Rebind-check never evaluates M or
opens a source connection.

The `generic-m` kind accepts one complete expression through `--m-template` or
`--m-file`. It reuses the workflow/source-profile closed grammar: a direct
allowlisted connector root, complete placeholder tokens, and safe transformation
namespaces only. Credential-like text, hard-coded file/URI paths, unknown
functions, and computed/postfix calls are refused with a pointer into the M text;
the expression is checked again when `source-template apply` materializes it.

### Copy Report Theme Bundles

Use theme commands only when `capabilities` advertises them:

```bash
pbi --json capabilities --for theme
pbi --json report themes show --project corp/template
pbi --json report themes extract --project corp/template --out build/corp-theme-bundle.json
pbi --json report themes apply --project build/sales --bundle build/corp-theme-bundle.json --dry-run
pbi --json report themes apply --project build/sales --bundle build/corp-theme-bundle.json --out-dir build/sales-themed
pbi --json report themes show --project build/sales-themed
pbi --json validate --strict build/sales-themed
```

This is raw report-level theme copying: `themeCollection` plus already-present
registered theme JSON resources. It is not visual formatting copy. Do not invent
PBIR formatting JSON for titles, legends, labels, conditional formatting,
filter expression authoring, bookmarks, logos, or custom visuals.

### Copy Visual Formatting Bundles

Use visual formatting bundle commands only when `capabilities` advertises them:

```bash
pbi --json capabilities --for "report visuals formatting"
pbi --json report visuals formatting list --project corp/template
pbi --json report visuals formatting extract --project corp/template --handle "visual:<page>:<source-visual>" --out build/visual-formatting-bundle.json
pbi --json report visuals formatting apply --project build/sales --handle "visual:<page>:<target-visual>" --bundle build/visual-formatting-bundle.json --dry-run
pbi --json report visuals formatting apply --project build/sales --handle "visual:<page>:<target-visual>" --bundle build/visual-formatting-bundle.json --allow-literal-text --out-dir build/sales-styled
pbi --json report visuals formatting set-text --project build/sales-styled --handle "visual:<page>:<target-visual>" --title "Revenue Overview" --show-title true --dry-run
pbi --json report visuals formatting show --project build/sales-styled --handle "visual:<page>:<target-visual>"
pbi --json validate --strict build/sales-styled
```

This is raw per-visual PBIR formatting portability. Apply writes only
`/visual/objects` on a same-type target visual and removes forbidden root-level
`/objects`. It refuses
copied literal title/alt-text/display strings unless `--allow-literal-text` is
explicit. `set-text` is the typed patch surface for title text and title
visibility; with `--clear-alt-text` it removes only a legacy misplaced
`altText` property. Authoring a new alt-text value remains unsupported until
Microsoft exposes a validator-supported PBIR location. It preserves sibling
formatting properties.
`set-color` is the typed patch surface for static literal `title.fontColor` and
wildcard/static `dataPoint.fill`. These commands are not typed legend, axis,
data-label, selector-specific color, or conditional formatting APIs.

### Author Pages And Visuals

Use report commands only when `capabilities` advertises them:

```bash
pbi --json capabilities --for report
pbi --json report pages list --project build/sales
pbi --json report pages add --project build/sales --display-name "Executive Summary" --dry-run
pbi --json report pages add --project build/sales --display-name "Executive Summary" --out-dir build/sales-pages
pbi --json report pages update --project build/sales-pages --handle "page:ReportSectionExecutiveSummary" --display-name "Executive Board" --dry-run
pbi --json report pages reorder --project build/sales-pages --order page:ReportSectionExecutiveSummary,page:ReportSectionOverview --dry-run
pbi --json report pages set-active --project build/sales-pages --handle "page:ReportSectionExecutiveSummary" --dry-run
pbi --json report pages delete-empty --project build/sales-pages --handle "page:ReportSectionExecutiveSummary" --dry-run
pbi --json report bookmarks list --project build/sales
pbi --json report bookmarks show --project build/sales --handle "bookmark:<bookmark-name>"
pbi --json report filters list --project build/sales
pbi --json report filters show --project build/sales --handle "filter:report:main:<filter-name>"
pbi --json report filters add --project build/sales --target "DimCustomer[Segment]" --value Enterprise --dry-run
pbi --json report filters add --project build/sales --target "FactSales[Revenue]" --min 1000 --max 5000 --dry-run
pbi --json report filters add --project build/sales --scope visual --visual "visual:ReportSectionOverview:<visual-name>" --target "DimCustomer[CustomerName]" --top 10 --by "Total Revenue" --dry-run
pbi --json report filters add --project build/sales --target "DimDate[Date]" --relative last --unit months --span 12 --dry-run
pbi --json report filters add --project build/sales --target "DimDate[Date]" --relative this --unit calendar-years --span 1 --dry-run
pbi --json report filters update --project build/sales --handle "filter:report:main:<filter-name>" --display-name "Reviewed filter" --dry-run
pbi --json report filters update --project build/sales --handle "filter:report:main:<filter-name>" --values-json '["Enterprise","SMB"]' --dry-run
pbi --json report filters clear --project build/sales --page page:ReportSectionOverview --dry-run
pbi --json report slicers list --project build/sales
pbi --json report slicers show --project build/sales --handle "slicer:<page-name>:<visual-name>"
pbi --json report slicers clear --project build/sales --handle "slicer:<page-name>:<visual-name>" --dry-run
pbi --json report interactions list --project build/sales
pbi --json report interactions show --project build/sales --handle "interaction:<page-name>:<ordinal>"
pbi --json report interactions disable --project build/sales --page page:ReportSectionOverview --source "visual:ReportSectionOverview:<source-visual>" --target "visual:ReportSectionOverview:<target-visual>" --dry-run
pbi --json report interactions set --project build/sales --page page:ReportSectionOverview --source "visual:ReportSectionOverview:<source-visual>" --target "visual:ReportSectionOverview:<target-visual>" --type HighlightFilter --out-dir build/sales-interactions
pbi --json report interactions reset --project build/sales-interactions --page page:ReportSectionOverview --source "visual:ReportSectionOverview:<source-visual>" --target "visual:ReportSectionOverview:<target-visual>" --dry-run
pbi --json report interactions show --project build/sales-interactions --page page:ReportSectionOverview --source "visual:ReportSectionOverview:<source-visual>" --target "visual:ReportSectionOverview:<target-visual>"
pbi --json report visuals list --project build/sales --page page:ReportSectionOverview
pbi --json report visuals catalog
pbi --json report visuals catalog --formatting
pbi --json report visuals add --project build/sales --page page:ReportSectionOverview --title "Revenue Card" --binding "role=Values,table=FactSales,measure=Total Revenue" --dry-run
pbi --json report visuals add --project build/sales --page page:ReportSectionOverview --title "Revenue Card" --binding "role=Values,table=FactSales,measure=Total Revenue" --out-dir build/sales-visual
pbi --json report visuals add --project build/sales --page page:ReportSectionOverview --visual-type pie --title "Revenue Share" --binding "role=Category,table=DimCustomer,column=Segment" --binding "role=Y,table=FactSales,measure=Total Revenue" --dry-run
pbi --json report visuals add --project build/sales --page page:ReportSectionOverview --visual-type combo --title "Pareto" --binding "role=Category,table=DimCustomer,column=Segment" --binding "role=Y,table=FactSales,measure=Total Revenue,sort=descending" --binding "role=Y2,table=FactSales,measure=Cumulative Revenue Share" --dry-run
pbi --json report visuals add --project build/sales --page page:ReportSectionOverview --visual-type matrix --title "Revenue Matrix" --binding "role=Rows,table=DimCustomer,column=Segment" --binding "role=Columns,table=DimDate,column=Year" --binding "role=Values,table=FactSales,measure=Total Revenue" --dry-run
pbi --json report visuals add --project build/sales --page page:ReportSectionOverview --visual-type slicer --mode basic --title "Segment Slicer" --binding "role=Values,table=DimCustomer,column=Segment" --dry-run
pbi --json report visuals clone --project corp/template --handle "visual:<page>:<template-visual>" --title "Revenue Copy" --dry-run
pbi --json report visuals clone --project corp/template --handle "visual:<page>:<template-visual>" --title "Revenue Copy" --out-dir build/sales-cloned
pbi --json report visuals show --project build/sales --handle "visual:ReportSectionOverview:<visual-name>"
pbi --json report visuals delete --project build/sales --handle "visual:ReportSectionOverview:<visual-name>" --dry-run
pbi --json report visuals delete --project build/sales --handle "visual:ReportSectionOverview:<visual-name>" --out-dir build/sales-minus-visual
pbi --json report visuals set-position --project build/sales --handle "visual:ReportSectionOverview:<visual-name>" --x 120 --y 140 --width 360 --height 220 --dry-run
pbi --json report visuals set-bindings --project build/sales --handle "visual:ReportSectionOverview:<visual-name>" --bindings-json '[{"role":"Values","table":"FactSales","measure":"Total Revenue"}]' --dry-run
pbi --json report visuals set-bindings --project build/sales --handle "visual:ReportSectionOverview:<visual-name>" --bindings-json '[{"role":"Values","table":"FactSales","measure":"Total Revenue"}]' --out-dir build/sales-bound
pbi --json report visuals repair-bindings --project build/sales --handle "visual:ReportSectionOverview:<visual-name>" --dry-run
pbi --json report visuals formatting set-color --project build/sales --handle "visual:ReportSectionOverview:<visual-name>" --slot title.fontColor --color "#123456" --dry-run
pbi --json report visuals show --project build/sales-bound --handle "visual:ReportSectionOverview:<visual-name>"
```

Page mutation commands patch only PBIR page metadata and `pages.json`.
`delete-empty` refuses pages with visuals or unknown page-local files. Use the
returned readback, wireframe, inspect, and validate commands before chaining
more work.

`report visuals catalog` returns the generated visual type and role contract.
Its `rules[]` table has one row per generated type with required/optional roles,
measure-only roles, projection limits, mutually exclusive roles,
runtime-parity rules, and honest fixture provenance. Only pie, donut,
pivotTable, and slicer currently cite independent Desktop-authored reference
files; do not promote repository-generated rows beyond their reported proof
level. When strict validation finds a mechanical binding parity defect, run
`report visuals repair-bindings --dry-run`: it may propose only proven role
canonicalization or Sum aggregation wrappers as a typed `setBindings` op.
Review the returned preview before applying it. Missing roles, duplicate fields,
and unproven substitutions remain explicit refusals.
Use `report visuals catalog --formatting --json` for the complete, embedded
`formatting-catalog.v1` surface consumed by `report visuals set-object`. It
currently contains exactly eleven proven object/property pairs, including their
encoding, PBIR container, wildcard visual-type scope, and dated Desktop/pilot
reference. The catalog is strict and deterministic; an entry is not implied by
memory, and new properties require a Desktop-authored fixture or dated pilot
observation. `--formatting` cannot be combined with `--visual-type`.
`report visuals add` creates only cataloged generated visual containers: card,
tableEx, line/area/bar/column families, scatterChart, pieChart, donutChart,
hundredPercentStackedColumnChart, lineClusteredColumnComboChart, matrix (PBIR
`pivotTable`), and slicer.
Combo charts require Category columns, Y column measures, and Y2 line measures.
Use `sort=descending` in binding text or `sortDirection=Descending` in JSON on
at most one projected measure for explicit category ordering; ascending and
multi-key sort are refused. Generated titles are visible literal
container titles under `/visual/visualContainerObjects/title` with `show = true`.
Pie/donut require exactly one Category column plus one or more Y measures;
matrix requires Rows columns, optional Columns columns, and Values measures;
when matrix has more than one Rows binding, generation enables the native
per-row `+/-` expand/collapse controls;
slicer requires exactly one Values column and supports Basic (default),
Dropdown, or Between mode. Use Between for numeric/date range sliders. Generated
slicers write mode under `/visual/objects/data`; Between additionally writes
`/visual/objects/slider.show = true` and requires at least 104 pixels of height
so Desktop has room to render the visible draggable band. In a dashboard spec,
set `singleSelect: true` on a slicer when the downstream DAX expects exactly one
selected value; this writes the native selection property without persisting a
selected value. Generated slicers never write `general.filter` or other
selection state.

Schema manifests may set `sortByColumn` on a column to preserve controlled
display order (for example, severity labels ordered by a hidden numeric column).
The target must be a different column in the same table.

Dashboard pages may declare `interactions` using page-local visual IDs:
`{"source":"matrix","target":"trend","type":"DataFilter"}`. `report build`
validates the references and emits PBIR `visualInteractions`, avoiding a
separate post-build interaction mutation.
Pie, donut, matrix, and slicer bindings retain `manual-desktop-canvas-refresh`
evidence:
`testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json` records
their refreshed canvases with exact expected values and a live slicer
interaction in Desktop Store 2.155.756.0. Automated `desktop-canvas-refresh`
proof and wider typed formatting remain open. Current title-bearing generated
bytes are `desktop-golden-pending` until Desktop open/refresh/save
re-verification. Do not infer support for
arbitrary visual families, slicer selections/sync, filter shapes beyond the
documented surface.

Raw columns are refused with `unsupported_feature` in card Values, chart Y,
matrix Values, and scatter X/Y/Size roles. Define a measure or wait for a
Desktop-authored aggregation-binding fixture. A model field may appear only once
per visual; duplicate queryRef/nativeQueryRef numbering is not invented without
Desktop ground truth. Category, Series, table detail Values, matrix
Rows/Columns, slicer Values, and Tooltips retain their proven column paths.
For scatter/bubble color grouping the field-well label is Legend but the PBIR
role is `Series`; `legend` remains an accepted CLI input alias only.

`report visuals clone` is template reuse, not new visual-family generation. It
copies only a simple visual container whose directory contains `visual.json` and
no sidecars, then patches the cloned name, position, visible title, and clone
annotations. `--title` therefore updates both Power BI's literal container title
and `powerbi-cli.placeholderTitle`; do not follow cloning with a redundant
`formatting set-text` call. The clone preserves visual type, bindings,
formatting, filters, and raw PBIR already
inside `visual.json`, so it remains the path for non-catalog visual shapes and
Desktop-authored formatting/state that generated families do not cover.

`report visuals formatting set-color` is typed static formatting only. It patches
`title.fontColor` and wildcard/static `dataPoint.fill` literal colors, returns
readback/raw-review/visual-readback/wireframe/inspect/validate commands, and
refuses data-bound dataPoint selectors. Do not use it as conditional formatting
support; rules, measure-driven colors, and selector-specific colors still need
Desktop-authored fixtures.

`report filters list/show/add/update/delete/clear` is the guarded PBIR filter
surface. `list/show` scans report/page/visual `filterConfig.filters`, gives
stable handles, and warns when filter metadata may contain selected semantic
model values. `add` writes exactly one supported Version 2 filter to
`/filterConfig/filters`:

- categorical: `--value`, `--value-json`, or `--values-json`;
- numeric range: `--min`, `--max`, or both, on a numeric TMDL column;
- TopN: `--top N` or `--bottom N` plus `--by <measure>`, at visual scope only;
- relative date: `--relative last|next|this`, `--unit
  days|weeks|months|years|calendar-weeks|calendar-months|calendar-years`, and a
  positive `--span`, on a date-typed TMDL column.

Handles are identity-based: a named record uses
`filter:<scope>:<owner>:<name>`, a nameless record uses `@<fnv-prefix>`, and an
entry from legacy `/filters` ends in `#legacy`. List output includes
`handleIdentity`, `handleAmbiguous`, and `arrayOrigin`. Duplicate identities get
deterministic `~N` list suffixes but cannot be mutated by handle; ordinal handles
from older releases are rejected with a re-list hint. Generated names include
raw target/type plus condition hashes, remain at most 50 characters, and let
different conditions coexist on one target. An exact duplicate still fails.

`--condition-type categorical|range|topn|relative-date` is optional when the
kind-specific flags identify the shape. Do not combine flag families.
Categorical values and numeric thresholds are persisted in PBIR, so use
dummy/offline-safe literals away from work.

`update` selects by the same stable handle as `show`. It can change
`displayName` on any filter and replace the complete values array of an exact
categorical In filter. It preserves name, ordinal, owner, and filter type. Named
handles stay stable; changing a nameless filter changes its content-addressed
fingerprint handle, which the mutation response returns. A requested type change
or a range/TopN/relative condition change returns
`unsupported_feature`; use a separately reviewed delete/add sequence instead.
Update dry-runs always expose exact raw before/after filter JSON.

`delete` removes one exact filter handle. `clear` removes filters by exact
filter handle, report scope, one page owner, one visual owner, or explicit
`--all`; `--page` clears only page-owned filters, not visual filters on that
page. Numeric range, TopN, and relative-date emission is schema-golden, not yet
Desktop canvas/open-save proven. Do not use it as tuple-filter, arbitrary
Advanced-expression, filter-sort, or type-changing update support.

Generated slicer creation is available through `report visuals add` and
dashboard specs for a single column in Basic, Dropdown, or Between mode. The generator
emits no persisted selections. `report slicers list/show/clear` covers PBIR
slicer inventory and the first guarded state-clear slice. `list/show` scans
slicer visuals, returns both
`slicer:` handles and underlying `visual:` handles, summarizes field
bindings/state, and warns when slicer visual metadata may contain selected
semantic-model values. `clear` removes persisted selection filters matching the
slicer binding from `/filterConfig/filters` and legacy `/filters`, with
`--dry-run`, `--out-dir`, or confirmed `--in-place`, and preserves slicer
bindings, layout, and formatting. Do not use it as selection/default-value or
sync-group authoring support; those still require Desktop-authored fixtures.
Basic/Dropdown generation is
backed by `manual-desktop-canvas-refresh` binding/canvas evidence in the
2026-07-10 proof record; the current title-bearing bytes are
`desktop-golden-pending`. Between emits the exact Desktop-authored mode literal
but still needs a committed Desktop canvas/refresh proof; automated proof and
wider formatting coverage remain open.

`report interactions list/show/set/disable/reset` covers the PBIR interaction
authoring slice. `list/show` scans page-level `visualInteractions`, resolves
source/target visuals to stable handles, flags stale visual references, and
states that missing rows mean Power BI default interaction behavior rather than
`NoFilter`. `disable` upserts an explicit `NoFilter` row. `set` upserts
DataFilter, HighlightFilter, or NoFilter for live source/target visual pairs
with guarded output modes and readback/wireframe/inspect/validate commands.
`reset` removes one matching explicit row and documents that its absence
restores the target visual's default interaction behavior. This reset shape is
locally proven at `unit-smoke`; Desktop canvas confirmation remains open.

`report bookmarks list/show` provides PBIR bookmark inventory. It scans
`definition/bookmarks/*.bookmark.json` plus `bookmarks.json` order/group
metadata, gives stable handles, and warns when bookmark state may contain
filter, slicer, highlight, or selected semantic-model values. Metadata-only
`set-display-name`, flat `reorder`, and guarded `delete` are implemented.
Capturing or creating bookmark state, updating captured visual/filter/slicer
state, and group edits still require Desktop-authored fixtures.

`report visuals delete` removes only a proven visual container directory that
contains exactly `visual.json`; it does not edit `page.json`, `pages.json`,
bindings elsewhere, bookmarks, filters, interactions, `z`, or `tabOrder`. Use
`--dry-run` or `--out-dir` first. In-place visual deletion requires the exact
`--confirm <visual-handle>`.

`set-bindings` is a first-slice existing-visual command: it replaces or clears
PBIR `queryState`, validates table/column/measure names against local TMDL, and
returns readback, wireframe, inspect, and validate commands. It covers
card/table values, standard category/value charts, category-share pie/donut,
Rows/Columns/Values matrix, scatter/bubble, and single-column slicer bindings,
with the measure-only value-role and single-use field gates described above.
More visual families, slicer selection/sync authoring, filter sort or arbitrary
expression mutation beyond the documented categorical update, conditional
formatting, and rich formatting beyond title/alt-text/static color must still be driven by
Desktop-authored fixtures.
Do not invent PBIR formatting JSON by memory.

When DAX chooses between two table expressions, do not assign the choice with
`VAR T = IF(condition, TableA, TableB)`. DAX `IF()` is scalar. Put the
table-consuming `CALCULATE`, `CONTAINS`, `TREATAS`, or iterator in each scalar
branch. `model dax lint` and `validate --strict` catch common direct uses, but
they are not a complete DAX engine.

### Handoff Between Home And Work

For deterministic offline refresh/performance fixtures, supply shared M
generator functions that accept positional `(rowScale, seed)` numeric
arguments, then synthesize a fresh project outside the source tree:

```bash
pbi --json workflow synthesize --project Report.pbip --expressions qa/generators.tmdl --out-dir ../powerbi-build/Report-QA-100x --row-scale 100 --seed 42
pbi --json lint ../powerbi-build/Report-QA-100x
pbi --json validate --strict ../powerbi-build/Report-QA-100x
```

The same scale/seed pair emits byte-identical partition M. Supplying only one
option uses row scale `1` or seed `0`; row scale must remain positive. Use this
copy for Desktop refresh timing and canvas QA without carrying live connector
text, credentials, or real rows.

For a deterministic resource/source reorientation, prefer the fingerprinted workflow:

```bash
pbi --json workflow plan --project Report.pbip --profile workflow/source-profile.json --out ../powerbi-build/report.plan.json --out-dir ../powerbi-build/report
pbi --json workflow run --plan ../powerbi-build/report.plan.json --confirm sha256:<plan-fingerprint>
pbi --json workflow verify --plan ../powerbi-build/report.plan.json
```

Use exactly the `powerbi-cli.source-profile.v1` shape: one stable `profileId`,
named `resources`, and typed `partition.replaceSource` entries. Every entry
must provide the exact table/partition, `expectedBeforeSha256`, a complete
profile-relative M `template`, `expectedConnector`, and the exact resource
names referenced as `{{powerbi-cli.resourcePath:<name>}}`. Each resource also
declares its lowercase `expectedSha256`. Do not put absolute machine paths or
credentials in a tracked profile. Supply a machine-local resource at plan time
as `--resource name=path`.

Keep `--out` and `--out-dir` outside the entire source project. The workflow
rejects caches, private directories, unregistered nested data, links, and
credential-bearing text inside selected Report/SemanticModel artifacts. An
`expectedConnector` is narrowly `Excel.Workbook` or `PostgreSQL.Database` and
must be the direct `Source = ...` root flow. Excel accepts exactly one declared
resource through `File.Contents`; PostgreSQL accepts none. Comments, strings,
dynamic calls, unknown connectors, and hard-coded file/URI paths never satisfy
the contract.

`workflow plan` writes only a new plan. Read its `planFingerprint` and pass it
unchanged to `workflow run`; never manufacture or shorten the confirmation.
Run creates a separate selected-artifact closure and leaves the source byte
identical. Treat an output containing `.powerbi-cli-workflow-incomplete` as
diagnostic only. A publishable result must pass `workflow verify`, which
reconstructs plan semantics and the expected staged definition from the
profile, checks copied closure/resource bytes, binds local MCP partition
readbacks, and reruns both validators.

The target workflow is:

```bash
pbi --json validate build/sales
pbi --json handoff check build/sales
pbi --json handoff rebind-plan build/sales --allow-unmapped
pbi --json handoff rebind-check build/sales-live --partition partition:FactSales:FactSales
pbi --json fixture normalize build/sales --out testdata/golden/sales-desktop-filter-contract.summary.json
pbi --json fixture verify build/sales --expected testdata/golden/sales-desktop-filter-contract.summary.json
```

Use `source-template add` before the final rebind plan when you know a
credential-free SQL Server, PostgreSQL, ODBC, Excel, CSV, folder, or
SharePoint/OneDrive mapping. Missing templates
produce structured findings and suggested commands; `--allow-unmapped` is useful
while drafting. Write the final work-machine instructions with
`handoff rebind-plan <project> --out <file.md>` and keep every credential in
Power BI Desktop at work. `handoff check` reports exactly one of `safe`, `review`,
or `unsafe`; an offline-safe result sets `safeForOfflineHandoff`, while
`--target work` sets `safeForWorkHandoff` for a credential-free recognized live
source. Credential matching
is case-insensitive and separator-tolerant but anchored to key/value syntax,
Bearer authorization headers, or recognizable GitHub/AWS token formats. Plain
prose such as `Passwort ändern` does not match. All matched values are rendered
as `***` in previews and plans.

### Desktop Oracle

`fixture normalize` and `fixture verify` are local golden-summary tools. They
prove deterministic project shape and are safe in default CI. They do not prove
Power BI Desktop compatibility by themselves. The normalized summary includes
path-free PBIR filter contract fields such as `desktopSafeName`,
`categoricalVersion`, `fromCount`, `whereCount`, and `whereUsesSourceAlias`.
On mismatch, `fixture verify` returns the actual summary in
`verification.actual` without writing a file. Add `--write-actual <path>` only
when a mismatch artifact is explicitly required.

On Windows with Power BI Desktop installed, opt into the Desktop oracle before
launching:

```bash
export POWERBI_DESKTOP_ORACLE=1
pbi --json desktop open-check build/sales
pbi --json desktop screenshot build/sales --out proof/sales.png
pbi --json desktop open build/sales --preflight normal
```

Use `open-check` or `screenshot` for one-shot proof; both attempt bounded
identity-checked cleanup. Require `proof.signals.cleanup.closed=true` before
starting the next Desktop test.
Use `desktop open` only when Bridge, DAX, or manual inspection needs the app to
remain open, and always run `desktop close` in a `finally`/cleanup step:

```bash
pbi --json desktop open build/sales
# bridge, bounded DAX, guarded live TMDL export, refresh, or canvas inspection
pbi --json desktop close
```

Require `session.owned=true` before using an interactive session and
`cleanup.closed=true` (or `session.alreadyClosed=true`) before the next test.
`desktop open` closes a prior CLI-owned session before launching another. Never
launch generated PBIPs with raw `Start-Process`, never accumulate same-title
project copies in Desktop, and never close a user-owned session by title.

`desktop screenshot --out` accepts only PNG paths outside the PBIP project
directory so evidence does not contaminate the handoff. It activates the exactly
selected `PBIDesktop*` PID, verifies that PID or one of its descendants owns the
foreground window, captures
to a unique same-directory temporary file, and publishes the destination only
after success. A failed capture preserves previous evidence. The response records
activation and foreground PIDs plus a `changes` entry when the PNG was created or
replaced. `--allow-unverified-capture` bypasses foreground verification and may
capture unrelated sensitive screen content; use it only with explicit risk
acceptance. Default cleanup reports every targeted PID with its ownership reason,
follows only the exact observed PID and verified descendants, never sweeps by
title or executable path, and verifies targeted PIDs are dead. `--leave-open` is
rejected; use the managed `desktop open`/`desktop close` pair.

Use `desktop harvest-reference` to archive a Desktop-saved `visual.json`,
`page.json`, or `report.json` fragment by stable handle:

```bash
pbi --json desktop harvest-reference \
  --project build/sales \
  --visual visual:ReportSectionOverview:VisualContainer1 \
  --out docs/reference/desktop-authored-visuals/sales-card.json
```

The archive wraps the fragment under `fragment` and records `provenance` with
the source path, source-project SHA-256 fingerprint, date, license note, and
Desktop version (`unknown` when none is supplied). The command calls the
shared harvested-fragment input guard and refuses persisted selection/filter
values, malformed or oversized files, links, and invalid UTF-8; it never
silently strips rejected state. Already-saved Linux projects remain
`desktop-golden-pending` because this path does not prove a Desktop canvas or
refresh.

When duplicate Desktop windows share the project title, selection prefers the
association-launch PID and then a new post-baseline Desktop PID. If only
pre-existing duplicates remain, the command reports `desktop_title_ambiguous`
instead of guessing. Close duplicates or keep the newly launched instance open
and retry.

`--timeout-ms` is a total watchdog for the bounded Desktop version probe,
pre-launch process baseline, file-association launch, and exact window/title
observation. Read
`proof.signals.observation` for elapsed time, poll count, completion reason,
and timeout state. The status/exit mapping is:

- Non-Windows: `error.code=unsupported_feature`, exit 2, before oracle opt-in is
  evaluated.
- Oracle disabled or Desktop not found on Windows: `oracle_unavailable`, exit 30.
- Launch succeeds but `open-check` observes no titled window before timeout:
  exit 0 with `proof.level=unit-smoke`, `proof.observedStage=desktop-launch`, and
  a timeout status.
- Launch succeeds but `screenshot` cannot capture because no exact project title
  appeared: `proof_incomplete`, exit 20.
- Foreground verification fails without the explicit override: `oracle_failed`,
  exit 40, with no PNG published.
- Launch, observer, capture, or cleanup subsystem failure:
  `oracle_failed`, exit 40.

`desktop refresh-check` and `desktop canvas-check` are cataloged forward-compatible
oracle commands. They currently return `error.code=unsupported_feature` without
launching Desktop or writing evidence; proof plans may emit them as templates
until their T9 Windows implementation lands. `desktop save-check` and Desktop
round-trip diffing remain planned as well. Do not expect a Desktop proof claim
until `capabilities --for desktop` advertises an available implementation.

If Desktop commands are unavailable, say the project has local validation and
fixture-summary proof only, not Desktop compatibility proof.

When a Desktop oracle command is available, inspect `proof.level`,
`proof.observedStage`, `proof.status`, `proof.signals.windowObserved`,
`proof.signals.titleMatched`, and `proof.claimedCompatibility`.
The canonical command proof level remains `unit-smoke`; `desktop-launch` and
`desktop-window` are observation stages only. `desktop-window` means a
`PBIDesktop*` main window title had the exact normalized PBIP project stem, either
plain (as in committed proofs) or followed by a ` - Power BI Desktop` dash
variant. A screenshot records the primary display but is not parsed by the CLI.
None of these signals proves the canvas rendered,
dummy partitions refreshed, or issue banners/dialogs were absent. Treat
`proof.claimedCompatibility=false` as mandatory until a future
automated `desktop-canvas-refresh` proof is advertised.

## Repo Work

When improving `powerbi-cli`:

1. Check `git status` and keep unrelated user changes.
2. Reproduce the missing or awkward behavior through the CLI boundary.
3. Patch behavior, capabilities, help, docs, this skill, and tests together
   when the user-visible contract changes.
4. Use snapshot/golden tests for output contracts.
5. Run focused tests first, then `cargo fmt --check` and
   `cargo test --all-targets`.
6. Use Desktop oracle proof only after generated files are expected to open.

High-value improvement targets:

- richer `capabilities` command schemas;
- `--json` accepted anywhere;
- stable object handles;
- `inspect --deep`;
- generated proof/follow-up commands on every mutation;
- strict validation diagnostics with machine-readable codes, source paths, and
  RFC 6901 JSON pointers;
- `handoff check` and source rebind planning;
- Desktop rebind/refresh proof for SQL Server, PostgreSQL/Npgsql, ODBC/DSN,
  Excel, CSV, folder, and SharePoint/OneDrive source templates;
- Desktop golden fixtures for visual binding and formatting.

## Verification

Focused loop:

```bash
cargo fmt --check
cargo check --all-targets
cargo test --test cli_smoke '<focused-filter>' -- --nocapture
```

For an invocation-by-invocation failure record, enable the shared integration
test logger. Each CLI run is emitted as one JSON line with exact argv, stdout,
stderr, exit code, and elapsed milliseconds:

```bash
POWERBI_CLI_TEST_LOG=1 cargo test --test e2e -- --nocapture
```

The offline e2e target runs the schema/profile/plan/spec/build/validate/handoff/
lint/triage/fixture loop for every checked-in archetype. JSON contract snapshots
use `tests/common::assert_json_snapshot`; update them only with
`UPDATE_SNAPSHOTS=1` and review the resulting diff. Ignored performance budgets
run through `cargo test --test perf -- --ignored`. The complete harness contract
is documented in `docs/testing.md`.

Broader loop:

```bash
cargo test --all-targets
git diff --check
```

Report concrete evidence:

- changed artifact paths or commit hash;
- exact commands used;
- validation, inspect, readback, handoff, or Desktop proof result;
- known limitations;
- next useful slice when work remains.
