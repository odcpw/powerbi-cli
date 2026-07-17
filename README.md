# powerbi-cli

`powerbi-cli` is a Rust command-line helper for agents authoring offline-safe
Power BI dashboard projects. It writes PBIP folders with PBIR report metadata
and TMDL semantic model metadata from a schema manifest, without connecting to
real data or writing imported data caches.

The intended workflow is:

```text
bring schema/dummy rows home
-> scaffold PBIP/PBIR/TMDL project
-> author report/model metadata with agents
-> validate and handoff-check no cache/credentials/real data are present
-> open at work in Power BI Desktop
-> replace dummy M partitions with corporate sources and refresh
```

The deterministic Rust commands need no vendor runtime. To enable Microsoft's
semantic engine, official report validator, and (on Windows) Desktop Bridge,
explicitly install the committed exact tool graph into your private user cache:

```bash
powerbi-cli integrations install --allow-network --json
powerbi-cli integrations status --deep --json
```

These tools run as bounded local child processes over stdio or one-shot argv;
the report/model is not uploaded to a hosted MCP service. The pinned Modeling
MCP preview may independently emit Microsoft usage telemetry and exposes no
disable flag, so this is not described as an OS-enforced zero-egress sandbox.
Normal commands never invoke npm or download packages. See
[`docs/microsoft-powerbi-agentic-integration-plan.md`](docs/microsoft-powerbi-agentic-integration-plan.md)
for the exact architecture and licensing boundary.

The committed npm lock and package integrity values authenticate what the
explicit install downloads. The installed-tree checksum then detects accidental
cache drift before a child is launched. The private cache is not a privileged
trust store: a hostile process already running as the same OS user could rewrite
both cached tools and their receipts, just as it could replace `powerbi-cli`
itself.

For repeatable source changes, use the closed staged workflow instead of
editing the source project:

```bash
powerbi-cli workflow plan \
  --project Report.pbip \
  --profile workflow/source-profile.json \
  --out ../powerbi-build/report.plan.json \
  --out-dir ../powerbi-build/report \
  --json
powerbi-cli workflow run \
  --plan ../powerbi-build/report.plan.json \
  --confirm sha256:<plan-fingerprint> \
  --json
powerbi-cli workflow verify --plan ../powerbi-build/report.plan.json --json
```

The versioned `powerbi-cli.source-profile.v1` JSON contract registers named
resources and `partition.replaceSource` entries. Each entry names one exact
table/partition, its expected current source hash, a complete profile-relative
M template, one of the two closed root connectors (`Excel.Workbook` or
`PostgreSQL.Database`), and the resource placeholders used by that template. A
resource path is either profile-relative or supplied at plan time with
`--resource name=path`, and its exact SHA-256 is declared in the profile;
credentials are never accepted, including credential-like canonical override
paths. Computed/postfix M calls cannot bypass the closed connector grammar.

`plan` fingerprints only the selected PBIP, its referenced Report and
SemanticModel, the registered templates/resources, and the pinned Microsoft
integration lock. `run` rechecks those inputs, creates a new output directory,
copies only that selected closure, applies the typed edits through the local
Microsoft MCP child, and requires strict native plus official report validation.
All output mutations are create-only and relative to the newly opened output
directory capability, so an ambient rename or junction/symlink swap cannot
redirect a write. It never edits the source. `verify` recomputes the output,
evidence, receipt checksum and semantic invariants, and validation claims
without changing the workflow output. It binds the complete evidence tree to a
fresh canonical read-only MCP export and credential-scans every bounded TMDL
file. A failed run remains marked incomplete for diagnosis and is not a
publishable result. Both the plan file and output directory must stay outside
the complete source project root. The selected artifact closure rejects caches,
private metadata, unregistered data, links, and credential-bearing source text.
See [`docs/source-profile-workflow.md`](docs/source-profile-workflow.md) for the
complete profile shape and command contract.

This project does not generate `.pbix` or `.pbit` binaries directly. It can
inspect and safely extract metadata/source files from PBIX/PBIT archives when
those entries are present, and it can import PBIP/PBIR/TMDL source folders from
such archives. Binary export remains a Desktop handoff: use Power BI Desktop to
open, save, or convert PBIP/PBIX/PBIT files.

## Desktop Compatibility Notes

Power BI Desktop is the compatibility oracle. Local JSON validation and
Microsoft's PBIR validator are useful, but Desktop can still reject issues that
schema validation misses. The current hard-won PBIR/Desktop findings, exact
proof commands, and next implementation backlog are recorded in
[`docs/pbir-desktop-oracle.md`](docs/pbir-desktop-oracle.md).

The checked-in `flat-ops`, `scatter-bubble`, and `catalog-proof` archetypes have
deterministic golden summaries and manual Desktop canvas/refresh proof records
under `testdata/desktop-proof/`. In particular,
`canvas-proof.2026-07-10.refresh-session.json` records generated pie, donut,
matrix, and slicer visuals rendering after refresh with exact expected values.
Those public records remain `manual-desktop-canvas-refresh` evidence for their
binding/canvas baselines. Same-report drillthrough has `schema-golden` proof
from the public schema and Desktop-authored reference shape; reproducible
end-to-end Desktop interaction proof remains open. Current generated visuals
  add title-container bytes and are
  `desktop-golden-pending` until re-verified. The opt-in live `desktop open-check`
  command reports process launch and exact project-title observations under
  `proof.observedStage`; its canonical `proof.level` remains `unit-smoke`.
  `desktop-launch` and `desktop-window` are observation stages, not proof
  levels. The closed ladder is `unit-smoke < schema-golden <
  desktop-golden-pending < manual-desktop-canvas-refresh <
  desktop-canvas-refresh`.
  `desktop screenshot` captures the primary display only after the foreground
  window PID is verified as the exactly matched Desktop process. Neither command
  automates canvas or refresh proof; the `desktop-canvas-refresh` level remains
  open.

Two additional Desktop-discovered guardrails are enforced locally. Scatter
color grouping is stored under PBIR `queryState.Series`, even though Desktop's
field well is labelled Legend; CLI inputs `legend`, `series`, `color`, and
`colour` all normalize to `Series`, and validation rejects a raw stale
`Legend` role. DAX lint also rejects a variable assigned with scalar `IF()`
when that variable is later passed directly as a table argument (for example to
`TREATAS` or `CONTAINS`). These are focused static checks, not a replacement for
refreshing every changed page in Desktop.

## No Fake Fallbacks

`powerbi-cli` is agent-first: supported features emit real PBIP/PBIR/TMDL
metadata, and unproven Power BI features fail with
`error.code = "unsupported_feature"` instead of writing partial guessed JSON.
Use `powerbi-cli features list --json` to see which feature surfaces are
supported, read-only, planned, or Desktop-golden gated.

## JSON Response Contract

Successful JSON is family-specific; there is no mandatory five-field success
envelope. Reader commands expose the records and counts documented by
`capabilities.commands[].followUpFields` and may omit `changes`. Semantic
mutation responses and `report build` expose `changes[]`, including dry-run
before/after plans. Artifact writers such as scaffold, schema normalize, and
profile output retain their documented family-specific fields.

Validation/result families may emit `ok:false` with a nonzero `exitCode` on
stdout. CLI errors are written to stderr with required `code`, `exitCode`, and
`message`, for example
`{"error":{"code":"invalid_args","exitCode":2,"message":"..."}}`.
`hint` and `suggestedCommands` are optional error fields.
Every `next[]` or `suggestedCommands[]` string is an executable `powerbi-cli`
command template; prose belongs in `instructions[]` or `notes[]`. The exact
machine-readable contract is available at `capabilities.responseShapes`.

## Build

```powershell
cargo build --bin powerbi-cli
cargo run --bin powerbi-cli -- --json capabilities
```

The CLI is pure Rust and should compile on Windows, Linux, and macOS. Power BI
Desktop open-proof is Windows-only, but PBIP/PBIR/TMDL scaffold and validation
commands are normal filesystem operations and are covered by CI on all three
platform families.

## First Commands

```powershell
cargo run --bin powerbi-cli -- --json doctor
cargo run --bin powerbi-cli -- --json capabilities
cargo run --bin powerbi-cli -- features list --json
cargo run --bin powerbi-cli -- features list --for drillthrough --json
cargo run --bin powerbi-cli -- robot-docs guide
cargo run --bin powerbi-cli -- --robot-triage
cargo run --bin powerbi-cli -- package inspect .\template.pbit --json
cargo run --bin powerbi-cli -- package extract .\template.pbit --out-dir .\build\template-source --json
cargo run --bin powerbi-cli -- package import .\source.pbix --out-dir .\build\imported-source --json
cargo run --bin powerbi-cli -- package source-pack --project .\build\sales --out .\build\sales-source.pbit --json
cargo run --bin powerbi-cli -- package export-plan --project .\build\sales --json
cargo run --bin powerbi-cli -- schema validate .\examples\sales.schema.json --json
cargo run --bin powerbi-cli -- profile infer --schema .\examples\sales.schema.json --out .\examples\sales.profile.json --json
cargo run --bin powerbi-cli -- report plan --schema .\examples\sales.schema.json --profile .\examples\sales.profile.json --objective "Executive sales overview" --out .\build\sales.planned.dashboard.json --json
cargo run --bin powerbi-cli -- report spec fields --schema .\examples\sales.schema.json --profile .\examples\sales.profile.json --json
cargo run --bin powerbi-cli -- report spec validate --schema .\examples\sales.schema.json --profile .\examples\sales.profile.json --spec .\examples\sales.dashboard.json --json
cargo run --bin powerbi-cli -- report build --schema .\examples\sales.schema.json --profile .\examples\sales.profile.json --spec .\examples\sales.dashboard.json --out-dir .\build\generic-sales --force --json
cargo run --bin powerbi-cli -- validate --strict .\build\generic-sales --json
cargo run --bin powerbi-cli -- handoff check .\build\generic-sales --json
cargo run --bin powerbi-cli -- fixture verify .\build\generic-sales --expected .\testdata\golden\generic-sales.summary.json --json
cargo run --bin powerbi-cli -- --json scaffold --schema examples/sales.schema.json --out-dir .\build\sales --force
cargo run --bin powerbi-cli -- --json inspect .\build\sales
cargo run --bin powerbi-cli -- inspect --deep .\build\sales --json
cargo run --bin powerbi-cli -- model measures list --project .\build\sales --json
cargo run --bin powerbi-cli -- model dax dependencies --project .\build\sales --json
cargo run --bin powerbi-cli -- model dax lint --project .\build\sales --json
$env:POWERBI_DESKTOP_ORACLE='1'
cargo run --bin powerbi-cli -- model dax execute --project .\build\sales --query 'EVALUATE ROW("Revenue", [Total Revenue])' --allow-data-read --max-rows 10 --json
cargo run --bin powerbi-cli -- model advanced inventory --project .\build\sales --json
cargo run --bin powerbi-cli -- model roles list --project .\build\sales --json
cargo run --bin powerbi-cli -- model perspectives list --project .\build\sales --json
cargo run --bin powerbi-cli -- model cultures list --project .\build\sales --json
cargo run --bin powerbi-cli -- model expressions list --project .\build\sales --json
cargo run --bin powerbi-cli -- model measures add --project .\build\sales --table FactSales --name "Average Revenue" --expression "DIVIDE([Total Revenue], [Total Units])" --dry-run --json
cargo run --bin powerbi-cli -- model measures add --project .\build\sales --table FactSales --name "Average Revenue" --expression "DIVIDE([Total Revenue], [Total Units])" --out-dir .\build\sales-v2 --json
cargo run --bin powerbi-cli -- diff .\build\sales .\build\sales-v2 --json
cargo run --bin powerbi-cli -- model calculated-columns add --project .\build\sales --table FactSales --name "Revenue Band" --expression "IF('FactSales'[Revenue] >= 10000, ""High"", ""Standard"")" --data-type string --dry-run --json
cargo run --bin powerbi-cli -- model calculated-columns add --project .\build\sales --table FactSales --name "Revenue Band" --expression "IF('FactSales'[Revenue] >= 10000, ""High"", ""Standard"")" --data-type string --out-dir .\build\sales-calc --json
cargo run --bin powerbi-cli -- diff .\build\sales .\build\sales-calc --scope model.calculatedColumns --json
cargo run --bin powerbi-cli -- model relationships list --project .\build\sales --json
cargo run --bin powerbi-cli -- model relationships update --project .\build\sales --handle <relationship-handle> --cross-filtering-behavior bothDirections --out-dir .\build\sales-relationships --json
cargo run --bin powerbi-cli -- diff .\build\sales .\build\sales-relationships --scope model.relationships --json
cargo run --bin powerbi-cli -- model partitions list --project .\build\sales --json
cargo run --bin powerbi-cli -- model partitions show --project .\build\sales --handle <partition-handle> --json
cargo run --bin powerbi-cli -- model partitions show --project .\build\sales --handle <partition-handle> --include-source --json
cargo run --bin powerbi-cli -- source-template add --project .\build\sales --table FactSales --kind sql --server "<server>" --database "<database>" --schema dbo --object FactSales --dry-run --json
cargo run --bin powerbi-cli -- source-template add --project .\build\sales --table FactSales --kind excel --file "<workbook.xlsx>" --sheet FactSales --dry-run --json
cargo run --bin powerbi-cli -- source-template add --project .\build\sales --table FactSales --kind sql --server "<server>" --database "<database>" --schema dbo --object FactSales --out-dir .\build\sales-rebind --json
cargo run --bin powerbi-cli -- handoff rebind-plan .\build\sales-rebind --json
cargo run --bin powerbi-cli -- source-template apply --project .\build\sales-rebind --handle source-template:FactSales:FactSales --server sql.example.internal --database Sales --out-dir .\build\sales-live --json
cargo run --bin powerbi-cli -- fixture normalize .\build\sales --out .\testdata\golden\sales.summary.json --json
cargo run --bin powerbi-cli -- fixture verify .\build\sales --expected .\testdata\golden\sales.summary.json --json
cargo run --bin powerbi-cli -- desktop open-check .\build\sales --json
cargo run --bin powerbi-cli -- desktop screenshot .\build\sales --out .\proof\sales.png --json
cargo run --bin powerbi-cli -- report design-plan --project .\build\sales --json
cargo run --bin powerbi-cli -- report wireframe export .\build\sales --json
cargo run --bin powerbi-cli -- report layout auto --project .\build\sales --page page:ReportSectionOverview --preset overview --dry-run --json
cargo run --bin powerbi-cli -- report pages list --project .\build\sales --json
cargo run --bin powerbi-cli -- report pages add --project .\build\sales --display-name "Executive Summary" --out-dir .\build\sales-pages --json
cargo run --bin powerbi-cli -- report pages update --project .\build\sales-pages --handle <page-handle> --display-name "Executive Board" --dry-run --json
cargo run --bin powerbi-cli -- report pages reorder --project .\build\sales-pages --order <page-handle>,<page-handle> --dry-run --json
cargo run --bin powerbi-cli -- report pages set-active --project .\build\sales-pages --handle <page-handle> --dry-run --json
cargo run --bin powerbi-cli -- report pages delete-empty --project .\build\sales-pages --handle <page-handle> --dry-run --json
cargo run --bin powerbi-cli -- report bookmarks list --project .\build\sales --json
cargo run --bin powerbi-cli -- report bookmarks show --project .\build\sales --handle <bookmark-handle> --json
cargo run --bin powerbi-cli -- report bookmarks set-display-name --project .\build\sales --handle <bookmark-handle> --display-name "Executive View" --dry-run --json
cargo run --bin powerbi-cli -- report bookmarks reorder --project .\build\sales --order <bookmark-handle>,<bookmark-handle> --dry-run --json
cargo run --bin powerbi-cli -- report bookmarks delete --project .\build\sales --handle <bookmark-handle> --dry-run --json
cargo run --bin powerbi-cli -- report filters list --project .\build\sales --json
cargo run --bin powerbi-cli -- report filters show --project .\build\sales --handle <filter-handle> --json
cargo run --bin powerbi-cli -- report filters add --project .\build\sales --target "DimCustomer[Segment]" --value Enterprise --dry-run --json
cargo run --bin powerbi-cli -- report filters add --project .\build\sales --target "FactSales[Revenue]" --min 1000 --max 5000 --dry-run --json
cargo run --bin powerbi-cli -- report filters add --project .\build\sales --scope visual --visual <visual-handle> --target "DimCustomer[CustomerName]" --top 10 --by "Total Revenue" --dry-run --json
cargo run --bin powerbi-cli -- report filters add --project .\build\sales --target "DimDate[Date]" --relative last --unit months --span 12 --dry-run --json
cargo run --bin powerbi-cli -- report filters update --project .\build\sales --handle <filter-handle> --display-name "Reviewed filter" --dry-run --json
cargo run --bin powerbi-cli -- report filters clear --project .\build\sales --page <page-handle> --dry-run --json
cargo run --bin powerbi-cli -- report slicers list --project .\build\sales --json
cargo run --bin powerbi-cli -- report slicers show --project .\build\sales --handle <slicer-handle> --json
cargo run --bin powerbi-cli -- report slicers clear --project .\build\sales --handle <slicer-handle> --dry-run --json
cargo run --bin powerbi-cli -- report interactions list --project .\build\sales --json
cargo run --bin powerbi-cli -- report interactions show --project .\build\sales --handle <interaction-handle> --json
cargo run --bin powerbi-cli -- report interactions disable --project .\build\sales --page <page-handle> --source <visual-handle> --target <visual-handle> --dry-run --json
cargo run --bin powerbi-cli -- report interactions set --project .\build\sales --page <page-handle> --source <visual-handle> --target <visual-handle> --type HighlightFilter --out-dir .\build\sales-interactions --json
cargo run --bin powerbi-cli -- report themes show --project .\build\sales --json
cargo run --bin powerbi-cli -- report themes extract --project .\corp\template --out .\build\corp-theme-bundle.json --json
cargo run --bin powerbi-cli -- report themes apply --project .\build\sales --bundle .\build\corp-theme-bundle.json --out-dir .\build\sales-themed --json
cargo run --bin powerbi-cli -- report themes presets list --json
cargo run --bin powerbi-cli -- report themes apply-preset --project .\build\sales --preset risk-dashboard --dry-run --json
cargo run --bin powerbi-cli -- report style inspect --project .\build\sales --json
cargo run --bin powerbi-cli -- report style extract --project .\corp\template --out .\build\corp-style-bundle.json --json
cargo run --bin powerbi-cli -- report style diff --project .\build\sales --bundle .\build\corp-style-bundle.json --json
cargo run --bin powerbi-cli -- report style apply --project .\build\sales --bundle .\build\corp-style-bundle.json --out-dir .\build\sales-styled --allow-literal-text --json
cargo run --bin powerbi-cli -- report visuals list --project .\build\sales --json
cargo run --bin powerbi-cli -- report visuals catalog --json
cargo run --bin powerbi-cli -- report visuals formatting list --project .\build\sales --json
cargo run --bin powerbi-cli -- report visuals formatting show --project .\build\sales --handle <visual-handle> --json
cargo run --bin powerbi-cli -- report visuals formatting conditional-formatting list --project .\build\sales --json
cargo run --bin powerbi-cli -- report visuals formatting conditional-formatting show --project .\build\sales --handle <visual-handle> --json
cargo run --bin powerbi-cli -- report visuals formatting extract --project .\corp\template --handle <source-visual-handle> --out .\build\visual-formatting-bundle.json --json
cargo run --bin powerbi-cli -- report visuals formatting apply --project .\build\sales --handle <target-visual-handle> --bundle .\build\visual-formatting-bundle.json --dry-run --json
cargo run --bin powerbi-cli -- report visuals formatting apply --project .\build\sales --handle <target-visual-handle> --bundle .\build\visual-formatting-bundle.json --allow-literal-text --out-dir .\build\sales-styled --json
cargo run --bin powerbi-cli -- report visuals formatting set-text --project .\build\sales --handle <visual-handle> --title "Revenue Overview" --alt-text "Revenue KPI card" --dry-run --json
cargo run --bin powerbi-cli -- report visuals add --project .\build\sales --page <page-handle> --title "Revenue Card" --binding "role=Values,table=FactSales,measure=Total Revenue" --out-dir .\build\sales-visual --json
cargo run --bin powerbi-cli -- report visuals clone --project .\corp\template --handle <template-visual-handle> --title "Revenue Clone" --out-dir .\build\sales-cloned --json
cargo run --bin powerbi-cli -- report visuals set-position --project .\build\sales --handle <visual-handle> --x 120 --y 140 --width 360 --height 220 --out-dir .\build\sales-layout --json
cargo run --bin powerbi-cli -- report visuals show --project .\build\sales-layout --handle <visual-handle> --json
cargo run --bin powerbi-cli -- report visuals delete --project .\build\sales-layout --handle <visual-handle> --dry-run --json
cargo run --bin powerbi-cli -- report visuals delete --project .\build\sales-layout --handle <visual-handle> --out-dir .\build\sales-layout-minus-visual --json
cargo run --bin powerbi-cli -- report visuals set-bindings --project .\build\sales --handle <visual-handle> --bindings-json "[{""role"":""Values"",""table"":""FactSales"",""measure"":""Total Revenue""}]" --dry-run --json
cargo run --bin powerbi-cli -- report visuals set-bindings --project .\build\sales --handle <visual-handle> --bindings-json "[{""role"":""Values"",""table"":""FactSales"",""measure"":""Total Revenue""}]" --out-dir .\build\sales-bound --json
cargo run --bin powerbi-cli -- report drilldown set-hierarchy --project .\build\sales --handle <line-chart-handle> --field "DimDate[FiscalYear]" --field "DimDate[Month]" --dry-run --json
cargo run --bin powerbi-cli -- lint .\build\sales --json
cargo run --bin powerbi-cli -- handoff check .\build\sales --json
cargo run --bin powerbi-cli -- validate --strict .\build\sales --json
cargo run --bin powerbi-cli -- --json validate .\build\sales
```

`scaffold --force` only rebuilds a non-empty directory when its prior
`powerbi-cli.manifest.copy.json` is present and readable. It removes the exact
artifacts named by that prior manifest (including removed table/page/visual
files), prunes only empty generated directories, and preserves user-added
files. An unmarked non-empty directory is refused.

## Schema Manifest

Start with `examples/sales.schema.json` for a tiny star-schema smoke test, or
`examples/archetypes/regional-sales.schema.json` for a multi-page sample that
exercises drillthrough chains, TopN-by-measure filters, multi-page slicers,
and non-ASCII column/measure names. The manifest describes:

- `tables`: table names, columns, types, measures, and optional dummy rows
- `relationships`: column-to-column model relationships
- `pages`: report pages and visual containers
- `bindings`: visual field-well bindings by role, table, and column/measure

Semantic-model handles percent-encode literal `%` and `:` inside table, column,
measure, and partition components as `%25` and `%3A`; always reuse returned
handles instead of constructing them by hand. Manifest and calculated-column input type `date` emits TMDL
`dataType: dateTime`; calculated-column authoring also supplies `formatString:
"Short Date"` unless the caller provides a format string.

Generated table partitions use Power Query M `#table(...)` expressions. Those
dummy partitions are there to preserve model shape and field names while the
project is away from the corporate data environment.

For small report controls and compact reference dimensions, `model tables
add-static` adds either a disconnected single-string-column selector or a
1-10-column string lookup table backed by a generated inline M `#table`
partition. Lookup keys in the first column are unique; relationships are added
separately with `model relationships add`. The command refuses replacement,
credentials, multiline cells, duplicate rows/keys, and arbitrary fact-table
ingestion, and validates the project after every write.

The `regional-sales` archetype is deliberately dummy data, but keeps the
column names and shape close enough to exercise a non-ASCII column
(`Größenklasse`) and measure (`Umsatz Übersicht`), a model relationship, DAX
measures, and bound card/table/chart/slicer PBIR visual definitions across
three pages.

## Current Limits

- The live feature boundary is `powerbi-cli features list --json`. Known but
  unimplemented or unproven report features such as tooltip pages, bookmark
  state capture/create/update/grouping, slicer selection/sync authoring, interaction
  reset/default semantics, non-catalog generated visual families, visual
  drillthrough action links, cross-report drillthrough, and conditional
  formatting authoring return `error.code = "unsupported_feature"` and do not
  write fallback PBIR.
- PBIX/PBIT package commands are metadata doors, not binary writers.
  `package inspect` classifies archive entries, `package extract` extracts only
  safe metadata/source entries by default, `package import` succeeds only when
  real allowlisted PBIP/PBIR/TMDL source files exist inside the archive,
  `package source-pack` first refuses unknown files and files in dot-directories,
  then scans every included file for credentials and PII-suspect row literals;
  non-dummy or unverified partition sources are also refused, and
  `package export-plan` emits the Desktop handoff. `package export/compile/pack`
  is intentionally refused.
- Package extraction streams through four default budgets: 10,000 archive
  entries, 256 MiB per entry, 2 GiB total uncompressed, and a 200:1 maximum
  compression ratio. `--max-entries`, `--max-entry-bytes`,
  `--max-total-bytes`, and `--max-compression-ratio` are explicit overrides.
  Any limit failure removes partial extraction output. Zip-slip paths remain
  skipped and extraction still requires an empty destination.
- Source-package files are allowlisted to root `.pbip`, report `.platform`/
  `definition.pbir`/definition JSON, semantic-model `.platform`/
  `definition.pbism`/definition TMDL, registered/shared JSON resources, and the
  generated `.gitignore`, `POWERBI_HANDOFF.md`, and
  `powerbi-cli.manifest.copy.json` sidecars. Other files—including every file
  below `.git`, `.vscode`, `.powerbi-cli`, or another dot-directory—cause a
  deterministic refusal listing and no archive is written.
- Programmatic visual authoring currently covers first-slice PBIR visual
  discovery with `report visuals catalog` and generated PBIR visual creation
  with `report visuals add` for card, tableEx, lineChart, areaChart,
  stackedAreaChart, clusteredBarChart, clusteredColumnChart, barChart,
  columnChart, scatterChart, pieChart, donutChart, matrix (emitted as PBIR
  `pivotTable`), and slicer generated patterns, plus PBIR
  `queryState` generation, `report visuals set-bindings` replacement/clear
  operations for existing visuals, and guarded `report visuals delete` for
  simple visual containers that contain only `visual.json`. `report visuals
  clone` copies one simple existing visual container as template reuse, patches
  only name, position, and clone annotations, and preserves visual type,
  bindings, formatting, filters, and raw PBIR already inside `visual.json`.
  It validates table, column, and measure names against local TMDL and returns
  readback commands. Generated `--title` text is emitted as PBIR container chrome
  under `/visual/visualContainerObjects/title` (`show = true`), with alt text
  under the shared `/visual/visualContainerObjects/general` object and
  annotation metadata retained for accessibility/readback. Raw columns are
  refused in card Values, chart Y, matrix Values, and scatter X/Y/Size roles;
  define a measure until a Desktop-authored aggregation binding is available.
  Reusing the same model field twice in one visual is also refused because no
  Desktop-authored duplicate queryRef numbering convention is available.
  Scatter color grouping uses the canonical PBIR `Series` role. User-facing
  aliases such as `legend` are accepted on input but never written to
  `queryState` because Desktop silently leaves that field well unbound.
  Pie and donut use exactly one Category column plus one or more Y measures and
  emit the Desktop-authored default descending sort by the first Y field. Matrix
  uses ordered Rows, optional Columns, and one or more Values measures. Slicer
  uses exactly one Values column and emits only Basic (default) or Dropdown mode
  under `/visual/objects/data`; it never generates persisted selection state.
  The four binding families retain `manual-desktop-canvas-refresh` evidence:
  `testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json` records
  refreshed Desktop canvases with exact expected values plus live slicer
  interaction. Current title-bearing generated bytes are
  `desktop-golden-pending` until Desktop open/refresh/save re-verification;
  automated `desktop-canvas-refresh` proof and broader typed formatting remain
  open.
  `report visuals formatting list/show` inventories existing PBIR formatting
  object cards and property names with raw payloads omitted unless
  `--include-raw` is passed. `report visuals formatting extract/apply` copies
  raw per-visual PBIR formatting bundles between same-type visuals while
  replacing only `/visual/objects` and removing any forbidden root-level
  `/objects`; apply refuses copied literal text unless `--allow-literal-text` is
  passed.
  `report visuals formatting set-text` patches typed title text, title
  visibility, and shared visual-container alt text while preserving sibling formatting
  properties. More visual families, richer typed formatting mutations,
  `Default`/reset interaction semantics, slicer selection/sync and additional
  mode authoring, filter
  sort and arbitrary expression-level filter mutations, and conditional
  formatting still need Desktop-authored golden fixtures.
- Programmatic DAX measure authoring covers `model measures
  list/show/add/update/delete` over generated TMDL table files. Local validation
  proves file structure and readback, not DAX engine semantics.
  `model dax dependencies` and `model dax lint` add offline static reference
  checks for measures and calculated columns: missing fields, ambiguous
  references, self references, simple measure cycles, and scalar `IF()`
  variables passed directly to known table-argument functions. They do not
  parse or execute the complete DAX language. On Windows, `model dax execute`
  provides a separate bounded live-engine path: the exact PBIP must already be
  open, `POWERBI_DESKTOP_ORACLE=1` and `--allow-data-read` are both required,
  only `EVALUATE` or `DEFINE ... EVALUATE` query forms are accepted, and the
  query text is never returned. Rows and cell text are capped because result
  data can be sensitive. This live preflight ignores only each selected
  artifact's root `.pbi/` runtime directory, which Desktop creates beside the
  source definition. Offline validation, packaging, workflow, and handoff keep
  rejecting those runtime files. Updates refuse blocks with unsupported Desktop-authored TMDL metadata
  instead of silently dropping it; Power BI Desktop or an explicit engine bridge
  remains the compatibility oracle.
- Programmatic static-table authoring covers `model tables add-static` for a
  new disconnected single-string-column selector or a small 1-10-column string
  lookup dimension backed by a generated inline `#table` partition. Cells are
  bounded, short, and screened for credential-like text; the first column is a
  unique key. Broader table/column CRUD, automatic relationships, and arbitrary
  fact-table ingestion remain outside this guarded surface.
- Programmatic DAX calculated column authoring covers `model calculated-columns
  list/show/add/update/delete` with explicit data types, guarded output modes,
  readback commands, and `diff --scope model.calculatedColumns`. Updates refuse
  unsupported Desktop-authored TMDL metadata instead of silently dropping it.
  Input `--data-type date` is normalized to TMDL `dateTime` with a default
  `Short Date` format string, matching scaffolded date columns.
- Programmatic relationship authoring covers `model relationships
  list/show/add/update/delete` with endpoint validation, guarded output modes,
  readback commands, and `diff --scope model.relationships`. Endpoint rewiring
  is currently modeled as delete+add for clearer audit trails.
  Measure, calculated-column, and relationship writes retain the original TMDL
  file through post-write validation. A failed validation restores that file and
  returns `projectModified: false` plus rollback details.
- Programmatic partition inspection covers `model partitions list/show` with
  source kind, strict generated `#table(...)` shape/model-column/row-arity
  checks, redacted source previews, and offline safety findings. Full source and
  TMDL block readback requires `--include-source` and is refused for `review` or
  `unsafe` partitions.
- Programmatic advanced semantic-model readback covers
  `model advanced inventory` plus `model roles|perspectives|cultures|expressions
  list/show` for TMDL metadata already present in a project. Mutating those
  advanced surfaces remains blocked until object-specific writers and fixtures
  exist.
- Source-template authoring covers `source-template list/show/add/apply` for
  credential-free SQL Server, PostgreSQL, ODBC, and Excel rebind metadata stored
  as sidecar JSON. PostgreSQL templates record current Npgsql compatibility guidance;
  ODBC templates accept only a bare DSN name (no `;`/`=` attributes) and record
  that the named DSN must already exist there. `source-template apply` is the
  explicit materialization step that replaces one safe generated dummy partition.
  With `--replace-existing` and an exact `--confirm <partition-handle>`, it can also
  intentionally retarget a recognized credential-free SQL, PostgreSQL, ODBC, or
  external-file partition; unknown, web, credential-bearing, and unconfirmed
  sources remain refused. Excel templates use `Excel.Workbook(File.Contents(...))`,
  promote the selected sheet/table headers, explicitly convert imported columns to
  their TMDL model types, and require an absolute workbook path when applied.
  `handoff rebind-plan` maps
  templates to partitions and can write a self-contained Markdown runbook with
  `--out <file.md>` (existing files require `--force`). Credential detection
  redacts JSON/Markdown excerpts and suppresses runbook creation. CSV and
  generic M template kinds remain planned and refused.
- Programmatic report layout authoring covers `report pages
  list/show/add/update/reorder/set-active/delete-empty`, `report visuals
  list/show/add/clone/delete`, guarded `report visuals set-position`, and guarded
  `report visuals set-bindings`. Page edits patch only PBIR page metadata plus
  `pages.json`; `delete-empty` refuses pages with visuals or unknown page-local
  files. Visual add writes one generated `visual.json`; position edits patch only
  PBIR visual geometry; binding edits patch only PBIR field-well `queryState`;
  visual clone copies only a proven simple `visual.json` container and patches
  the cloned name/position/annotations;
  visual delete removes only a proven `visuals/<name>/visual.json` container
  and requires exact `--confirm <visual-handle>` for in-place deletion. On
  Windows it safely clears a read-only visual-directory attribute (including
  OneDrive-backed folders), and restores the visual if directory removal fails.
  Typed title edits synchronize both supported PBIR title containers and an
  existing `powerbi-cli.placeholderTitle` annotation.
  Mutations return readback, wireframe, inspect, and validate commands. Every
  report mutation with `--out-dir` first runs the complete plan against the
  source project, so an invalid handle or unsupported plan does not populate the
  output directory.
- Programmatic drillthrough authoring covers `report drillthrough
  set/show/clear` for same-report page drillthrough bindings over one model
  column. `set` links the `pageBinding` parameter's `boundFilter` and
  `fieldExpr` to a paired bodyless Categorical Drillthrough filter, updates the
  page type, and hides the page by default. It does not author visual action
  links or support cross-report drillthrough. Readback surfaces the linked
  binding and filter metadata without selected data values. The supported
  same-report slice is `schema-golden`, backed by the public page schema and
  Desktop-authored reference shape; reproducible Desktop drillthrough
  navigation proof remains open.
- Programmatic report filter handling covers `report filters
  list/show/add/update/delete/clear` for raw report/page/visual PBIR
  `filterConfig.filters` readback; categorical, numeric range, visual TopN, and
  relative-date authoring; type-preserving updates; exact-handle deletion; and
  owner-scoped clear. `add` validates model targets against TMDL: range columns
  must be numeric, relative-date columns must be date typed, and TopN `--by`
  must resolve to a measure. TopN is visual-only. Range supports closed and
  open bounds; relative-date supports rolling and calendar day/week/month/year
  variants. Categorical values and numeric thresholds persist in PBIR, so use
  dummy/offline-safe values away from work. `update` changes any display name
  and can replace categorical In-filter values; it returns
  `unsupported_feature` for filter type changes or edits to range bounds, TopN
  ranking, and relative windows. Dry-run update exposes exact raw before/after
  filter JSON. `clear` requires an exact
  filter handle, report scope, one page, one visual, or explicit `--all`; a
  page clear removes only page-owned filters, not visual filters on that page.
  Filter handles are identity-based rather than ordinal: named entries use
  `filter:<scope>:<owner>:<name>`, nameless legacy entries use an `@` FNV
  fingerprint prefix, and `/filters` entries carry `#legacy`. Duplicate
  identities get unique deterministic list handles marked `handleAmbiguous`;
  handle-targeted mutation refuses them. Cached handles therefore do not
  retarget after an earlier deletion, and old ordinal handles fail with a
  re-list hint. Generated names include raw target/type and condition hashes,
  stay within Desktop's 50-character limit, and allow distinct conditions on
  one field while exact duplicates still fail loudly.
  List output and applied filter mutations hide raw filter JSON by default and
  mark filters that may persist selected semantic-model values. Numeric range,
  TopN, and relative-date emission is `schema-golden`: it follows Microsoft's
  PBIR schemas and reference shapes, but Desktop canvas/open-save verification
  remains pending. Filter sort, tuple filters, arbitrary Advanced expressions,
  and type-changing updates remain unsupported.
- Programmatic visual formatting authoring covers raw formatting bundle
  extract/apply plus typed `report visuals formatting set-text` and
  `set-color`. `set-color` patches only static literal `title.fontColor` and
  wildcard/static `dataPoint.fill`. `report visuals formatting
  conditional-formatting list/show` can inventory existing conditional-formatting
  signals in PBIR. Conditional-formatting authoring and data-bound color
  selectors remain Desktop-fixture gated.
- Programmatic report slicer handling covers generated Basic/Dropdown slicers
  through `report visuals add`/dashboard specs and inspection/state clear through
  `report slicers list/show/clear`. List output hides raw
  slicer visual JSON by default, returns both `slicer:` and underlying
  `visual:` handles, summarizes field bindings and slicer state, and warns when
  slicer metadata may persist selected semantic-model values. `clear` removes
  persisted selection filters matching the slicer binding with guarded output
  modes while preserving bindings, layout, and formatting. Generated slicers
  contain no `general.filter` or cached selection state. Additional modes,
  default selections, selection mutation, and sync groups remain Desktop-fixture
  gated; the generated Basic/Dropdown family is
  `manual-desktop-canvas-refresh` proven by the checked-in 2026-07-10 canvas
  proof record.
- Programmatic report interaction authoring covers `report interactions
  list/show/set/disable` for explicit PBIR page `visualInteractions` overrides.
  `disable` upserts an explicit `NoFilter` row; `set` upserts DataFilter,
  HighlightFilter, or NoFilter with guarded output modes, stable source/target
  visual resolution, duplicate-row refusal, readback, wireframe, inspect, and
  validate commands. Missing rows still mean Power BI default interaction
  behavior, not `NoFilter`; authoring `Default`/reset semantics remains
  Desktop-fixture gated.
- Programmatic report bookmark handling covers `report bookmarks list/show` for
  raw PBIR `definition/bookmarks/*.bookmark.json` readback plus `bookmarks.json`
  order/group metadata. Metadata-only mutation is supported for display-name
  edits, flat reorder, and guarded delete. Capturing bookmark state, creating
  new stateful bookmarks, updating captured visual/filter/slicer state, and
  group reorder remain unsupported until Desktop-authored golden fixtures exist.
  List output hides raw bookmark JSON by default and marks bookmark state that
  may persist filter, slicer, highlight, or selected semantic-model values.
- Programmatic report design/layout authoring covers `report design-plan`,
  `report layout auto`, and `report drilldown set-hierarchy`. Design-plan is a
  read-only profile with exact next commands; auto-layout rewrites only visual
  `position` blocks; drilldown hierarchy replaces a chart's Category
  projections with two or more resolved model columns and requires an existing
  Y binding.
- Programmatic report theme authoring covers `report themes show/extract/apply`
  for raw report-level theme bundles plus `report themes presets` and
  `report themes apply-preset` for built-in registered-resource theme presets.
  `report style inspect/extract/diff/apply` is the higher-level master-format
  workflow: it combines report theme material and per-visual formatting, then
  reapplies formatting by visual type and ordinal without copying bindings or
  data roles. Apply refuses copied literal text unless `--allow-literal-text` is
  passed. Filter sort and arbitrary expression mutation beyond the documented
  categorical update, bookmark captured-state mutation,
  logos, richer typed PBIR formatting, and conditional formatting authoring
  remain planned.
- `handoff check` fails fast on unsafe home/work transfer hazards, including
  Power BI caches/binaries, local settings, embedded data files, real connector
  partitions, and credential-like text in TMDL, M, JSON, Markdown, PBIP, PBIR,
  PBISM, and `.platform` files. Structurally valid literal tables with
  PII-suspect row values produce `status: review`; credentials or non-dummy
  sources produce `status: unsafe`; only `status: safe` sets
  `safeForOfflineHandoff: true`.
- Credential matching is case-insensitive and separator-tolerant for anchored
  key/value syntax (`password`, `pwd`, `pass`, account/access/SAS/API keys,
  `sig`, user identifiers, secrets/tokens), recognizes Bearer authorization
  headers plus GitHub/AWS token formats, and redacts matched values as `***`.
  Bare prose is not enough: German UI text such as `Passwort ändern` and words
  containing `pass` do not match without credential assignment syntax.
- `lint` now includes a small BPA-style report/model pass: DAX static findings,
  duplicate page/visual titles, and missing visual alt text. Generated visuals
  include default alt text so new reports start on the right side of that rule.
- Structural validation reports an empty PBIR visual container as a missing
  `visual.json` with an explicit remove-or-restore repair, instead of allowing a
  later deep-inspection `file_not_found` failure.
- `diff` compares normalized semantic summaries with stable handles, so agents
  can verify measure, calculated-column, and relationship changes after CLI
  mutations or Desktop round-trips without reading raw TMDL.
- `fixture normalize` and `fixture verify` provide deterministic, path-free
  golden summaries for generated or Desktop-authored PBIP fixtures, including
  explicit page visual interaction summaries and PBIR filter contract summaries
  without raw PBIR. Checked-in summaries include the compact
  `testdata/golden/sales.summary.json` baseline and the wider
  `testdata/golden/sales-desktop-filter-contract.summary.json`
  report/page filter fixture.
  A verify mismatch includes the actual normalized JSON in
  `verification.actual` and writes nothing by default. Use
  `--write-actual <path>` only when an explicit mismatch artifact is wanted.
- `desktop open-check` and `desktop screenshot` are opt-in Windows oracle
  commands. `--timeout-ms` is one watchdog budget for the bounded version probe,
  pre-launch process baseline, file-association launch, and window/title polling.
  `proof.level` uses the canonical `unit-smoke` level; launch and exact normalized
  project-stem matches are reported separately as `proof.observedStage`. Window
  candidates must be `PBIDesktop*`; `AnnualSales` never matches project `Sales`.
  Cleanup never targets baseline/pre-launch processes and reports a reason for
  every owned PID it targets. Screenshot output must be a PNG outside the project
  directory. Capture uses a same-directory temporary file, verifies foreground
  PID ownership, and publishes/replaces the requested PNG only after success.
  `--allow-unverified-capture` explicitly bypasses foreground verification and
  risks capturing unrelated sensitive screen content. Responses always include
  `changes` (`[]` unless a PNG was created or replaced). Canvas rendering,
  blank-canvas rejection, refresh completion, and issue banner/dialog detection
  remain unimplemented. A confirmed launch with no titled window before expiry
  remains the honest `desktop-launch` observation stage; it is not
  `oracle_failed`. On Windows, a disabled oracle returns exit 30; on
  non-Windows systems Desktop commands return `error.code = "unsupported_feature"`
  before oracle opt-in evaluation. An attempted oracle subsystem failure is exit
  40, while evidence blocked by launch/observation timeout or title mismatch is
  `proof_incomplete` (exit 20).
- Validation checks file structure, parseable JSON, page references, TMDL table
  presence, relationship endpoints, and offline hazards. It is not a Power BI
  Desktop open proof.
- `.pbix`, `.pbit`, `.abf`, `.pbi/`, embedded data files, and
  `localSettings.json` are treated as unsafe for the home/offline workflow.

## Format References

- Microsoft PBIP project docs: <https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-overview>
- Microsoft report/PBIR docs: <https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-report>
- Microsoft enhanced PBIR docs: <https://learn.microsoft.com/en-us/power-bi/developer/embedded/projects-enhanced-report-format>
- Microsoft semantic model/TMDL docs: <https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-dataset>
- TMDL overview: <https://learn.microsoft.com/en-us/analysis-services/tmdl/tmdl-overview>
- Power BI Desktop template docs: <https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-templates>
- Microsoft Power BI report authoring skill docs: <https://learn.microsoft.com/en-us/power-bi/developer/agentic/power-bi-report-authoring-skill-overview>
- Microsoft semantic model authoring skill docs: <https://learn.microsoft.com/en-us/power-bi/developer/agentic/semantic-model-authoring-skill-overview>
- PBIR Desktop oracle notes:
  [docs/pbir-desktop-oracle.md](docs/pbir-desktop-oracle.md)

## Roadmap

- [goal.md](goal.md): current data-agnostic product goal for agent-first
  dashboard authoring from arbitrary schema/profile/intent inputs.
- [docs/roadmap.md](docs/roadmap.md): planned command surface, development
  phases, and Desktop-backed test strategy.
- [docs/porting-analysis.md](docs/porting-analysis.md): clean-room analysis of
  adjacent Power BI tooling and what to port, reimplement, or defer.
- [docs/pbir-desktop-oracle.md](docs/pbir-desktop-oracle.md): Desktop
  round-trip findings, source links, proof commands, and immediate backlog.
- [docs/reviews/agent-first-review-synthesis.md](docs/reviews/agent-first-review-synthesis.md):
  independent Claude/Grok review synthesis focused on making the CLI
  agent-first in the style of `ooxml-cli`.
- [skills/powerbi-cli/SKILL.md](skills/powerbi-cli/SKILL.md): canonical
  agent-facing operating guide for using and improving `powerbi-cli`.

## License

`powerbi-cli` source is available under the [MIT License](LICENSE). Optional
Microsoft integrations are downloaded directly into each user's private cache
and remain governed by their upstream licenses; see
[the recorded integration license decision](integrations/microsoft/LICENSE-REVIEW.md).
