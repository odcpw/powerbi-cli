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

For offline Desktop refresh and performance QA, `workflow synthesize` can call
shared M generator functions with an exact row scale and seed while replacing
the live database root in a fresh project copy:

```bash
powerbi-cli workflow synthesize \
  --project Report.pbip \
  --expressions qa/generators.tmdl \
  --out-dir ../powerbi-build/Report-QA-100x \
  --row-scale 100 \
  --seed 42 \
  --json
```

Each mapped expression in a scaled run is invoked positionally as
`Expression(rowScale, seed)`. Supplying only one option uses `1` for row scale
or `0` for seed. Both values must be exact non-negative M integers and row scale
must be positive. Re-run with the same pair for byte-identical partition M; vary
the scale to reproduce load behavior without moving real data or credentials.

This project does not generate `.pbix` or `.pbit` binaries directly. It can
inspect and safely extract metadata/source files from PBIX/PBIT archives when
those entries are present, and it can import PBIP/PBIR/TMDL source folders from
such archives. On Windows, PBIX is also a first-class managed Desktop document:
`desktop open` launches it through the same owned-session lifecycle as PBIP, and
`model dax execute` can issue bounded read-only queries against its exact live
semantic-model engine. `model live export-tmdl` uses that same exact engine
identity and the pinned local Microsoft Modeling MCP in read-only mode to
publish a bounded, credential-scanned semantic-model TMDL definition into one
fresh output directory. This is semantic-model extraction only: it does not
export report pages or claim full PBIX-to-PBIP conversion. Binary export remains
a Desktop handoff; PBIP/TMDL stays the editable source format.

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
  Committed records under `testdata/desktop-proof/` use the strict
  `powerbi-cli.desktop-proof.v1` shape. They name the linked feature IDs and
  explicit evidence signals; the embedded loader rejects overclaims and
  `features list` reports the maximum valid level per feature. Placeholder
  records can remain `desktop-golden-pending`, while launch, title, or
  screenshot evidence alone cannot claim canvas/refresh compatibility.
  `desktop screenshot` captures the primary display only after the foreground
  window PID is verified as the selected Desktop process or one of its process
  descendants. `desktop open` and idempotent `desktop close` provide one bounded,
  CLI-owned interactive session for Bridge/DAX/manual inspection; opening another
  managed session closes the prior owned session first. Neither workflow
  automates canvas or refresh proof; the `desktop-canvas-refresh` level remains
  open.

  `desktop harvest-reference` archives a visual, page, or report JSON fragment
  from an already-saved PBIP into a provenance-stamped wrapper. It records the
  source path, source-project SHA-256 fingerprint, harvest date, license note,
  and Desktop version when supplied. The selected fragment is read through the
  bounded harvested-fragment safety contract; persisted selection/filter values
  are refused rather than silently stripped. Linux archives record
  `desktopVersion: "unknown"` and remain `desktop-golden-pending` until explicit
  Desktop canvas/refresh evidence exists.

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
- `powerbi-cli report build --schema <schema.json> [--profile <profile.json>] [--spec <dashboard.json>] (--dry-run | --out-dir <project-dir> [--force]) [--trace] --json` — Compile a data schema plus optional strict v1/v2 dashboard spec into an offline-safe PBIP/PBIR/TMDL project using supported primitives only; root/page/visual filters compile through AddFilter and page drillthrough through SetDrillthrough with model/type validation, and the response includes operation changes/outcomes, stable-handle readback, scorecard, and side-effect-free proofPlan commands _(proof: `unit-smoke`)_
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
- `powerbi-cli report plan --schema <schema.json> [--profile <profile.json>] (--intent <intent.md|intent.json> | --objective <goal>) [--out <dashboard.json>] [--explain-rules] --json` — Create a deterministic starter dashboard spec and slot-agnostic planner-v2 proposals from schema/profile candidates and a typed JSON or Markdown report intent (with backward-compatible objective text) _(proof: `unit-smoke`)_
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
- `powerbi-cli report wireframe export <project-dir-or.pbip> [--format json|svg|html] [--template <name>] [--page-size 1280x720|1920x1080] [--grid columns=12,gutter=16,margin=24,rowUnit=8] [--out <path> | --dry-run] --json` — Export report pages, deterministic grid slots, visual geometry, bindings, and lint markers as JSON, SVG, or HTML without Power BI Desktop _(proof: `unit-smoke`)_
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
- `report.design-layout` — **supported**, read-write-layout, proof `unit-smoke`: Report design planning and automatic layout. Commands: `report design-plan`, `report layout auto`, `report wireframe export`.
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

## Input Safety Limits

User-supplied schema, profile, dashboard-spec, JSON bundle, intent, DAX/text,
and binding files are read through bounded, strict-UTF-8, non-symlink input
guards. Safety refusals use `error.code = "input_safety_violation"` (exit 10)
with a hint and executable next command. `capabilities.limits` is the
machine-readable source for every current and reserved surface limit, including
`$include` depth/count, CSV/JSON rows and columns, PNG magic-byte checks, ops
schema validation, snapshots, and harvested PBIR fragments. See
[`docs/input-safety-contract.md`](docs/input-safety-contract.md) for the exact
numbers and the APIs future command owners must call. Package archives and the
staged workflow retain their stronger specialized streaming/identity policies.

`profile infer --rows <rows.csv|rows.json>` emits a `powerbi-cli.dataProfile.v2`
document with bounded null rates, distinct counts, numeric/date ranges, time
coverage, duplicate-key grain conflicts, and type-coercion diagnostics. CSV
uses its first row as the header; JSON accepts an array of objects or an array
whose first item is a header row. Top-value counts and cardinality are always
available, while literal top values are replaced with `[REDACTED]` by default.
`--include-data-values` is an explicit opt-in for a maximum of five bounded
values per column and is refused when credential/PII scanning flags a column.
`--redact` remains a deprecated no-op alias. Profiles stamped
`dataValues:true` are data-bearing: `handoff check` reports them and
`package source-pack` refuses to write an archive.

## JSON Response Contract

Successful JSON is family-specific; there is no mandatory five-field success
envelope. Reader commands expose the records and counts documented by
`capabilities.commands[].followUpFields` and may omit `changes`. Semantic
mutation responses and `report build` expose `changes[]`, including dry-run
before/after plans. Artifact writers such as scaffold, schema normalize, and
profile output retain their documented family-specific fields.

`report build` also returns `compiled.ops`, a flat operation-change aggregate,
and `readback` command arrays keyed by stable report/page/visual/table/measure
handles. When a dashboard spec emits typed operations, `operationOutcomes[]`
records each kernel's concrete changes, readback commands, warnings, and
created handles. Its shared `scorecard.v1` contains native validation, Microsoft
validator availability, lint grouped by severity, the fixed unavailable design
lint shape, offline handoff status, and the honest proof level. Add `--trace`
to include a deterministic `trace[]` of `{op, ms}` planning buckets; the
default response omits that optional field. `triage` embeds the same scorecard
projection for an existing project. See
`capabilities.responseShapes.scorecard.v1` and
`capabilities.responseShapes.reportBuild` for the machine-readable details.

Validation/result families may emit `ok:false` with a nonzero `exitCode` on
stdout. CLI errors are written to stderr with required `code`, `exitCode`, and
`message`, for example
`{"error":{"code":"invalid_args","exitCode":2,"message":"..."}}`.
`hint` and `suggestedCommands` are optional error fields.
Every `next[]` or `suggestedCommands[]` string is an executable `powerbi-cli`
command template; prose belongs in `instructions[]` or `notes[]`. The exact
machine-readable contract is available at `capabilities.responseShapes`.

The internal operation-plan spine uses the durable `powerbi-cli.ops.v1` JSON
shape. It is intentionally not a public command yet: converted mutation
kernels will consume typed `op` records through a temporary-directory
transaction, validate the staged PBIP tree, and publish only through explicit
dry-run, out-dir, or snapshotted in-place modes.

## Build

```powershell
cargo build --bin powerbi-cli
cargo run --bin powerbi-cli -- --json capabilities
cargo run --bin powerbi-cli -- --json capabilities --for "report build" --compact
```

The CLI is pure Rust and should compile on Windows, Linux, and macOS. Power BI
Desktop open-proof is Windows-only, but PBIP/PBIR/TMDL scaffold and validation
commands are normal filesystem operations and are covered by CI on all three
platform families.

## Testing

Integration tests use one shared, structured CLI runner plus repository-backed
archetype/spec helpers. Set `POWERBI_CLI_TEST_LOG=1` to emit each invocation as
one JSON line containing argv, stdout, stderr, exit code, and elapsed time. See
[`docs/testing.md`](docs/testing.md) for the focused e2e, snapshot, and nightly
performance workflows.

## First Commands

```powershell
cargo run --bin powerbi-cli -- --json doctor
cargo run --bin powerbi-cli -- --json capabilities
cargo run --bin powerbi-cli -- features list --json
cargo run --bin powerbi-cli -- features list --for drillthrough --json
cargo run --bin powerbi-cli -- robot-docs guide
cargo run --bin powerbi-cli -- robot-docs render --check
cargo run --bin powerbi-cli -- --robot-triage
cargo run --bin powerbi-cli -- skill status --json
cargo run --bin powerbi-cli -- skill install --json
cargo run --bin powerbi-cli -- package inspect .\template.pbit --json
cargo run --bin powerbi-cli -- package extract .\template.pbit --out-dir .\build\template-source --json
cargo run --bin powerbi-cli -- package import .\source.pbix --out-dir .\build\imported-source --json
cargo run --bin powerbi-cli -- package source-pack --project .\build\sales --out .\build\sales-source.pbit --json
cargo run --bin powerbi-cli -- package work-pack --project .\build\sales-live --json
cargo run --bin powerbi-cli -- package export-plan --project .\build\sales --json
cargo run --bin powerbi-cli -- schema validate .\examples\sales.schema.json --json
cargo run --bin powerbi-cli -- schema normalize .\examples\sales.schema.json --out .\build\sales.schema.normalized.json --json
cargo run --bin powerbi-cli -- profile infer --schema .\examples\sales.schema.json --out .\examples\sales.profile.json --json
# Bounded CSV/JSON profile inference keeps top literals redacted by default.
cargo run --bin powerbi-cli -- profile infer --schema .\examples\sales.schema.json --rows .\build\sales-rows.csv --out .\build\sales.profile.v2.json --json
cargo run --bin powerbi-cli -- report plan --schema .\examples\sales.schema.json --profile .\examples\sales.profile.json --objective "Executive sales overview" --out .\build\sales.planned.dashboard.json --json
cargo run --bin powerbi-cli -- report plan --schema .\examples\sales.schema.json --profile .\examples\sales.profile.json --intent .\examples\intents\sales.intent.json --out .\build\sales.intent.dashboard.json --json
cargo run --bin powerbi-cli -- report spec fields --schema .\examples\sales.schema.json --profile .\examples\sales.profile.json --json
cargo run --bin powerbi-cli -- report spec schema --json
cargo run --bin powerbi-cli -- report spec validate --schema .\examples\sales.schema.json --profile .\examples\sales.profile.json --spec .\examples\sales.dashboard.json --json
cargo run --bin powerbi-cli -- report spec explain --schema .\examples\sales.schema.json --profile .\examples\sales.profile.json --spec .\examples\sales.dashboard.json --json
cargo run --bin powerbi-cli -- report spec normalize .\examples\sales.dashboard.json --out .\build\sales.dashboard.normalized.json --json
cargo run --bin powerbi-cli -- report spec upgrade --spec .\examples\sales.dashboard.json --out .\build\sales.dashboard.v2.json --json
cargo run --bin powerbi-cli -- report build --schema .\examples\sales.schema.json --profile .\examples\sales.profile.json --spec .\examples\sales.dashboard.json --out-dir .\build\generic-sales --force --json
# V2 spec filters compile through the same typed AddFilter kernels as the CLI.
cargo run --bin powerbi-cli -- report build --schema .\examples\sales.schema.json --spec .\examples\filter-kinds.dashboard.v2.json --out-dir .\build\filter-kinds --force --json
cargo run --bin powerbi-cli -- validate --strict .\build\generic-sales --json
cargo run --bin powerbi-cli -- handoff check .\build\generic-sales --json
cargo run --bin powerbi-cli -- fixture verify .\build\generic-sales --expected .\testdata\golden\generic-sales.summary.json --json
cargo run --bin powerbi-cli -- --json scaffold --schema examples/sales.schema.json --out-dir .\build\sales --force
cargo run --bin powerbi-cli -- --json inspect .\build\sales
cargo run --bin powerbi-cli -- inspect --deep .\build\sales --json
cargo run --bin powerbi-cli -- model measures list --project .\build\sales --json
cargo run --bin powerbi-cli -- model dax dependencies --project .\build\sales --json
cargo run --bin powerbi-cli -- model dax lint --project .\build\sales --json
cargo run --bin powerbi-cli -- lint --rules --json
cargo run --bin powerbi-cli -- lint --explain dax.reference_self --json
$env:POWERBI_DESKTOP_ORACLE='1'
cargo run --bin powerbi-cli -- model dax execute --project .\build\sales --query 'EVALUATE ROW("Revenue", [Total Revenue])' --allow-data-read --max-rows 10 --json
cargo run --bin powerbi-cli -- desktop open .\SourceProfile.pbix --json
cargo run --bin powerbi-cli -- model dax execute --project .\SourceProfile.pbix --query 'EVALUATE TOPN(20, INFO.VIEW.TABLES())' --allow-data-read --max-rows 20 --json
cargo run --bin powerbi-cli -- model live export-tmdl --document .\SourceProfile.pbix --out-dir .\build\source-profile-model --allow-model-read --json
cargo run --bin powerbi-cli -- desktop close --json
cargo run --bin powerbi-cli -- model advanced inventory --project .\build\sales --json
cargo run --bin powerbi-cli -- model roles list --project .\build\sales --json
cargo run --bin powerbi-cli -- model perspectives list --project .\build\sales --json
cargo run --bin powerbi-cli -- model cultures list --project .\build\sales --json
cargo run --bin powerbi-cli -- model expressions list --project .\build\sales --json
cargo run --bin powerbi-cli -- workflow synthesize --project .\Report.pbip --expressions .\qa\generators.tmdl --out-dir .\build\Report-QA --row-scale 100 --seed 42 --json
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
cargo run --bin powerbi-cli -- model partitions add-grouped-rank --project .\build\analytics --table Signals --group-by Segment --order-by Score --desc --rank-column GroupRank --eligible-when "[IsEligible] = true" --dry-run --json
cargo run --bin powerbi-cli -- source-template add --project .\build\sales --table FactSales --kind sql --server "<server>" --database "<database>" --schema dbo --object FactSales --dry-run --json
cargo run --bin powerbi-cli -- source-template add --project .\build\sales --table FactSales --kind excel --file "<workbook.xlsx>" --sheet FactSales --dry-run --json
cargo run --bin powerbi-cli -- source-template add --project .\build\sales --table FactSales --kind csv --file "<file.csv>" --delimiter , --encoding 65001 --has-header true --dry-run --json
cargo run --bin powerbi-cli -- source-template add --project .\build\sales --table FactSales --kind folder --path "<folder>" --pattern *.csv --dry-run --json
cargo run --bin powerbi-cli -- source-template add --project .\build\sales --table FactSales --kind sharepoint --site-url "<siteUrl>" --library "<library>" --path "<path>" --dry-run --json
cargo run --bin powerbi-cli -- source-template add --project .\build\sales --table FactSales --kind generic-m --m-template 'let Source = Sql.Database("{{powerbi-cli.placeholder:server}}", "{{powerbi-cli.placeholder:database}}") in Source' --dry-run --json
cargo run --bin powerbi-cli -- source-template add --project .\build\sales --table FactSales --kind sql --server "<server>" --database "<database>" --schema dbo --object FactSales --out-dir .\build\sales-rebind --json
cargo run --bin powerbi-cli -- handoff rebind-plan .\build\sales-rebind --json
cargo run --bin powerbi-cli -- source-template apply --project .\build\sales-rebind --handle source-template:FactSales:FactSales --server sql.example.internal --database Sales --out-dir .\build\sales-live --json
cargo run --bin powerbi-cli -- handoff rebind-check .\build\sales-live --partition partition:FactSales:FactSales --json
cargo run --bin powerbi-cli -- fixture normalize .\build\sales --out .\testdata\golden\sales.summary.json --json
cargo run --bin powerbi-cli -- fixture verify .\build\sales --expected .\testdata\golden\sales.summary.json --json
cargo run --bin powerbi-cli -- desktop open .\build\sales --json
cargo run --bin powerbi-cli -- desktop close --json
cargo run --bin powerbi-cli -- desktop open-check .\build\sales --json
cargo run --bin powerbi-cli -- desktop screenshot .\build\sales --out .\proof\sales.png --json
cargo run --bin powerbi-cli -- report design-plan --project .\build\sales --json
cargo run --bin powerbi-cli -- report wireframe export .\build\sales --json
cargo run --bin powerbi-cli -- report wireframe export .\build\sales --format svg --out .\proof\sales-wireframe --json
cargo run --bin powerbi-cli -- report wireframe export .\build\sales --format html --out .\proof\sales-wireframe.html --json
cargo run --bin powerbi-cli -- report layout auto --project .\build\sales --page page:ReportSectionOverview --template overview --dry-run --json
cargo run --bin powerbi-cli -- report layout auto --project .\build\sales --page page:ReportSectionOverview --template kpi-strip-trend-breakdown --grid columns=12,gutter=16,margin=24,rowUnit=8 --out-dir .\build\sales-layout --json
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
cargo run --bin powerbi-cli -- report interactions reset --project .\build\sales-interactions --page <page-handle> --source <visual-handle> --target <visual-handle> --dry-run --json
cargo run --bin powerbi-cli -- report themes show --project .\build\sales --json
cargo run --bin powerbi-cli -- report themes extract --project .\corp\template --out .\build\corp-theme-bundle.json --json
cargo run --bin powerbi-cli -- report themes apply --project .\build\sales --bundle .\build\corp-theme-bundle.json --out-dir .\build\sales-themed --json
cargo run --bin powerbi-cli -- report themes presets list --json
cargo run --bin powerbi-cli -- report themes apply-preset --project .\build\sales --preset risk-dashboard --dry-run --json
cargo run --bin powerbi-cli -- report style inspect --project .\build\sales --json
cargo run --bin powerbi-cli -- report style extract --project .\corp\template --out .\build\corp-style-bundle.json --json
cargo run --bin powerbi-cli -- report style diff .\build\style-before.json .\build\style-after.json --json
cargo run --bin powerbi-cli -- report style apply --project .\build\sales --bundle .\build\corp-style-bundle.json --out-dir .\build\sales-styled --allow-literal-text --json
cargo run --bin powerbi-cli -- report visuals list --project .\build\sales --json
cargo run --bin powerbi-cli -- report visuals catalog --json
cargo run --bin powerbi-cli -- report visuals catalog --formatting --json
cargo run --bin powerbi-cli -- report visuals formatting list --project .\build\sales --json
cargo run --bin powerbi-cli -- report visuals formatting show --project .\build\sales --handle <visual-handle> --json
cargo run --bin powerbi-cli -- report visuals formatting conditional-formatting list --project .\build\sales --json
cargo run --bin powerbi-cli -- report visuals formatting conditional-formatting show --project .\build\sales --handle <visual-handle> --json
cargo run --bin powerbi-cli -- report visuals formatting extract --project .\corp\template --handle <source-visual-handle> --out .\build\visual-formatting-bundle.json --json
cargo run --bin powerbi-cli -- report visuals formatting apply --project .\build\sales --handle <target-visual-handle> --bundle .\build\visual-formatting-bundle.json --dry-run --json
cargo run --bin powerbi-cli -- report visuals formatting apply --project .\build\sales --handle <target-visual-handle> --bundle .\build\visual-formatting-bundle.json --allow-literal-text --out-dir .\build\sales-styled --json
cargo run --bin powerbi-cli -- report visuals formatting set-text --project .\build\sales --handle <visual-handle> --title "Revenue Overview" --dry-run --json
cargo run --bin powerbi-cli -- report visuals formatting set-text --project .\build\sales --handle <visual-handle> --clear-alt-text --dry-run --json
cargo run --bin powerbi-cli -- report visuals add --project .\build\sales --page <page-handle> --title "Revenue Card" --binding "role=Values,table=FactSales,measure=Total Revenue" --out-dir .\build\sales-visual --json
cargo run --bin powerbi-cli -- report visuals clone --project .\corp\template --handle <template-visual-handle> --title "Revenue Clone" --out-dir .\build\sales-cloned --json
cargo run --bin powerbi-cli -- report visuals set-position --project .\build\sales --handle <visual-handle> --x 120 --y 140 --width 360 --height 220 --out-dir .\build\sales-layout --json
cargo run --bin powerbi-cli -- report visuals show --project .\build\sales-layout --handle <visual-handle> --json
cargo run --bin powerbi-cli -- report visuals delete --project .\build\sales-layout --handle <visual-handle> --dry-run --json
cargo run --bin powerbi-cli -- report visuals delete --project .\build\sales-layout --handle <visual-handle> --out-dir .\build\sales-layout-minus-visual --json
cargo run --bin powerbi-cli -- report visuals set-bindings --project .\build\sales --handle <visual-handle> --bindings-json "[{""role"":""Values"",""table"":""FactSales"",""measure"":""Total Revenue""}]" --dry-run --json
cargo run --bin powerbi-cli -- report visuals set-bindings --project .\build\sales --handle <visual-handle> --bindings-json "[{""role"":""Values"",""table"":""FactSales"",""measure"":""Total Revenue""}]" --out-dir .\build\sales-bound --json
cargo run --bin powerbi-cli -- report visuals repair-bindings --project .\build\sales --handle <visual-handle> --dry-run --json
cargo run --bin powerbi-cli -- report visuals formatting set-color --project .\build\sales --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json
cargo run --bin powerbi-cli -- report visuals add-card --project .\build\sales --page page:ReportSectionOverview --measure "FactSales.Total Revenue" --title "Revenue Card" --x 40 --y 40 --width 200 --height 120 --value-font-size 20 --category-font-size 9 --word-wrap --dry-run --json
cargo run --bin powerbi-cli -- report visuals add-slicer --project .\build\sales --page page:ReportSectionOverview --field "DimCustomer.Segment" --title "Segment" --x 40 --y 40 --width 240 --height 80 --mode Dropdown --single-select --dry-run --json
cargo run --bin powerbi-cli -- report visuals add-textbox --project .\build\sales --page page:ReportSectionOverview --title "Reading guide" --paragraphs-file guide.txt --x 40 --y 520 --width 400 --height 120 --dry-run --json
cargo run --bin powerbi-cli -- report visuals set-topn-guard --project .\build\sales --handle <visual-handle> --field DimCustomer.CustomerName --order-by "FactSales[Total Revenue]" --top 28 --dry-run --json
cargo run --bin powerbi-cli -- report visuals set-object --project .\build\sales --handle <visual-handle> --object categoryLabels --property fontSize --value 20 --dry-run --json
cargo run --bin powerbi-cli -- report visuals set-display-name --project .\build\sales --handle <visual-handle> --role Values --display-name "Rate zuletzt (BU je 1'000 FTE)" --dry-run --json
cargo run --bin powerbi-cli -- report drilldown set-hierarchy --project .\build\sales --handle <line-chart-handle> --field "DimDate[FiscalYear]" --field "DimDate[Month]" --dry-run --json
cargo run --bin powerbi-cli -- desktop bridge status --json
cargo run --bin powerbi-cli -- desktop bridge reload --project .\build\sales --pid 1234 --json
cargo run --bin powerbi-cli -- desktop bridge screenshot-page --project .\build\sales --pid 1234 --page ReportSection --out proof/page.png --json
cargo run --bin powerbi-cli -- desktop bridge screenshot-all --project .\build\sales --pid 1234 --out-dir proof/pages --json
cargo run --bin powerbi-cli -- profile validate .\build\sales.profile.json --json
cargo run --bin powerbi-cli -- profile summarize .\build\sales.profile.json --json
cargo run --bin powerbi-cli -- model tables show --project .\build\sales --handle table:FactSales --json
cargo run --bin powerbi-cli -- model tables rename --project .\build\sales --handle table:DimDate --new-name Calendar --rename-references --dry-run --json
cargo run --bin powerbi-cli -- model tables delete --project .\build\sales --handle table:DimSegment --dry-run --json
cargo run --bin powerbi-cli -- model columns show --project .\build\sales --handle column:FactSales:Revenue --json
cargo run --bin powerbi-cli -- model columns add --project .\build\sales --table FactSales --name Margin --expression '[Revenue] - [Cost]' --data-type decimal --dry-run --json
cargo run --bin powerbi-cli -- model columns update --project .\build\sales --handle column:FactSales:Revenue --format-string '$#,##0' --dry-run --json
cargo run --bin powerbi-cli -- model columns delete --project .\build\sales --handle column:DimSegment:Label --dry-run --json
cargo run --bin powerbi-cli -- model columns set-sort-by --project .\build\sales --table DimDate --column Month --by MonthNumber --dry-run --json
cargo run --bin powerbi-cli -- model calculated-columns list --project .\build\sales --json
cargo run --bin powerbi-cli -- model calculated-columns show --project .\build\sales --handle 'column:FactSales:Revenue Band' --json
cargo run --bin powerbi-cli -- model calculated-columns update --project .\build\sales --handle 'column:FactSales:Revenue Band' --expression 'IF(''FactSales''[Revenue] >= 5000, ""High"", ""Standard"")' --dry-run --json
cargo run --bin powerbi-cli -- model calculated-columns delete --project .\build\sales --handle 'column:FactSales:Revenue Band' --dry-run --json
cargo run --bin powerbi-cli -- model measures show --project .\build\sales --handle 'measure:FactSales:Total Revenue' --json
cargo run --bin powerbi-cli -- model measures update --project .\build\sales --handle 'measure:FactSales:Total Revenue' --expression 'SUM(''FactSales''[Revenue])' --dry-run --json
cargo run --bin powerbi-cli -- model measures delete --project .\build\sales --handle 'measure:FactSales:Average Revenue' --dry-run --json
cargo run --bin powerbi-cli -- model relationships show --project .\build\sales --handle <relationship-handle> --json
cargo run --bin powerbi-cli -- model relationships delete --project .\build\sales --handle <relationship-handle> --dry-run --json
cargo run --bin powerbi-cli -- model dax bridge-plan --project .\build\sales --json
cargo run --bin powerbi-cli -- model roles show --project .\build\sales --handle role:Safety --json
cargo run --bin powerbi-cli -- model perspectives show --project .\build\sales --handle perspective:Executive --json
cargo run --bin powerbi-cli -- model cultures show --project .\build\sales --handle culture:de-CH --json
cargo run --bin powerbi-cli -- model expressions show --project .\build\sales --handle expression:RefreshDate --json
cargo run --bin powerbi-cli -- source-template show --project .\build\sales --handle source-template:FactSales:FactSales --json
cargo run --bin powerbi-cli -- report tree --project .\build\sales --json
cargo run --bin powerbi-cli -- report find --project .\build\sales --kind visual --json
cargo run --bin powerbi-cli -- report cat --project .\build\sales --handle visual:ReportSectionOverview:VisualContainerSalesKpi --json
cargo run --bin powerbi-cli -- report query --project .\build\sales --selector kind:visual --json
cargo run --bin powerbi-cli -- report audit --project .\build\sales --json
cargo run --bin powerbi-cli -- report sanitize plan --project .\build\sales --json
cargo run --bin powerbi-cli -- report sanitize apply --project .\build\sales --dry-run --json
cargo run --bin powerbi-cli -- report pages show --project .\build\sales --handle page:ReportSectionOverview --json
cargo run --bin powerbi-cli -- report pages clone --project .\build\sales --from page:ReportSectionOverview --new-name ReportSectionOverviewCopy --visual-prefix Copy --dry-run --json
cargo run --bin powerbi-cli -- report drillthrough set --project .\build\sales --page page:ReportSectionOverview --target 'DimCustomer[Segment]' --dry-run --json
cargo run --bin powerbi-cli -- report drillthrough show --project .\build\sales --page page:ReportSectionOverview --json
cargo run --bin powerbi-cli -- report drillthrough clear --project .\build\sales --page page:ReportSectionOverview --dry-run --json
cargo run --bin powerbi-cli -- report filters delete --project .\build\sales --handle filter:report:main:ReportSegmentFilter --dry-run --json
cargo run --bin powerbi-cli -- desktop open .\build\sales --preflight normal --json
cargo run --bin powerbi-cli -- model dax execute --project .\build\sales --query-file checks/total-revenue.dax --allow-data-read --json
cargo run --bin powerbi-cli -- model measures add --project .\build\sales --table FactSales --name "Dynamic Revenue" --expression "SUM(FactSales[Revenue])" --format-string-definition "IF([Mode] = \"raw\", \"$#,##0\", \"$#,##0.00\")" --dry-run --json
cargo run --bin powerbi-cli -- validate .\build\sales --strict --backend all --json
cargo run --bin powerbi-cli -- lint .\build\sales --json
cargo run --bin powerbi-cli -- handoff check .\build\sales --json
cargo run --bin powerbi-cli -- validate --strict .\build\sales --json
cargo run --bin powerbi-cli -- --json validate .\build\sales
```

When a v2 spec contains `proof.desktop.level`, `proof.desktop.pages`,
`proof.desktop.expectValues`, or `proof.goldens`, `report build` returns a
`proofPlan` and appends the same commands to `next[]`. Expectation entries
become bounded `model dax execute` templates and golden names become
`fixture verify` templates. The compiler never runs those commands. On
non-Windows hosts Desktop-dependent entries are listed in
`proofPlan.unavailable[]` with the exact Windows oracle instruction; a
Desktop proof level is not claimed locally.

`report plan` accepts a bounded `--intent <intent.md|intent.json>` document in
the `intent.v1` shape. JSON fields and Markdown H2 sections cover audience,
questions, KPIs, comparisons, periods, drill paths, alert rules, filter
dimensions, preferred visual archetypes, page flow, and handoff requirements.
The response preserves the normalized document under `intent`; each KPI must
resolve to an exact model measure or the command returns `spec.missing_input`
with its pointer and measure candidates. Fields that the starter planner does
not compile remain visible in `warnings[]` with their owning bead. The
free-form `--objective` form remains available for quick question-only plans.
The response also includes an evidence-backed `shape` classification and the
same shape under `profileSummary` when a profile is supplied. It reports flat,
star, snowflake, or multi-fact only when schema relationships, cardinalities,
row-count ratios, and profile column signals support that verdict; otherwise
it returns `ambiguous` with competing hypotheses. Date-like columns without a
related date dimension produce a date-table proposal, and high-cardinality
categorical columns are called out as possible noise.

Planner v2 evaluates the embedded, versioned `planner-rules.v1` catalog after
shape and intent normalization. Pass `--explain-rules` (or use the equivalent
`report plan explain` form) to make the fired rules, deterministic scores, and
actual evidence values explicit. The response always carries the same
`planner.proposals[]` records: each names its rule id, visual family, bindings,
priority, size class, and semantic color token without coordinates or hex
values. The build-compatible `spec` remains dashboard.v1; `specV2` is the
slot/template/style candidate for the layout compiler and is marked
`desktop-golden-pending` until a Desktop canvas proof exists. Current rule ids are:
`planner.time-series`, `planner.category-ranking`, `planner.scatter-focus`,
`planner.detail-table`, `planner.measure-target`,
`planner.measure-total`, `planner.alert-exception-list`,
`planner.high-cardinality-drillthrough`, `planner.shape-flat-template`,
`planner.shape-snowflake-template`, `planner.shape-multi-fact-template`,
`planner.shape-ambiguous-template`, and `planner.overview`.

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

- `tables`: table names, columns, types, measures, optional dummy rows, and
  optional same-table `sortByColumn` metadata for controlled display order
- `relationships`: column-to-column model relationships
- `pages`: report pages and visual containers
- `bindings`: visual field-well bindings by role, table, and column/measure
- `interactions`: optional same-page visual pairs with `DataFilter`,
  `HighlightFilter`, or `NoFilter`; referenced visual IDs are validated and
  compiled into PBIR `visualInteractions`

Schema manifests may declare a non-empty `schemaVersion` (missing values emit
a compatibility warning for one release before becoming an error) and compose
bounded JSON fragments with `$include`. Includes are resolved relative to the
including file and are supported at the schema root, table entries, and the
v2 dashboard spec's `model`, `pages[]`, and `style` sections. The input-safety
guard rejects traversal, canonical paths outside the root, symlinks, cycles,
fragments deeper than eight levels, more than 200 fragments, or fragments
larger than 8 MiB. Run `schema normalize` or `report spec normalize` to write
a canonical, byte-stable document; the JSON response records sorted,
root-relative `normalizedFrom[]` provenance. Schema validation and report build
consume the same normalized values, so an inline document and an equivalent
include tree produce the same artifact output and parity fingerprint.

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

The generic semantic-model surface covers `model tables list/show/add/add-calculated/rename/delete`
and `model columns list/show/add/update/delete`. Table handles use the form
`table:<name>`; column handles use `column:<table>:<name>`. Literal `%` and `:`
inside each component are encoded as `%25` and `%3A`. Table rename refuses when
relationships, DAX, or variation metadata still reference the old name unless
`--rename-references` is supplied. Column updates refuse unknown
Desktop-authored properties in the targeted block, so annotations and
extended properties are never silently dropped. All mutating commands support
`--dry-run`, guarded `--in-place`, and isolated `--out-dir` output.

Calculated tables are authored with `model tables add-calculated`; the command
writes a real `partition <table> = calculated` DAX source and leaves schema
materialization to Power BI Desktop. Named M expressions use
`model expressions add/update/delete` with the same guarded output modes. The
existing DAX and M lint commands inspect these new blocks, while unknown
Desktop-authored expression metadata is refused rather than discarded. Until
Desktop refresh materializes a calculated table's columns, model completeness
lint defers the generic no-columns error for that calculated partition.

The `regional-sales` archetype is deliberately dummy data, but keeps the
column names and shape close enough to exercise a non-ASCII column
(`Größenklasse`) and measure (`Umsatz Übersicht`), a model relationship, DAX
measures, and bound card/table/chart/slicer PBIR visual definitions across
three pages.

## Current Limits

The 2026-09-04 build advertises 52 feature IDs (46 supported and 6 planned).
This generated snapshot keeps status and proof claims aligned with
`features list --json`; planned rows remain explicit refusals.

| status / proof | feature IDs |
|---|---|
| supported / `unit-smoke` | `agent.codex-skill-distribution`, `desktop.dax-query-execution`, `desktop.live-tmdl-export`, `desktop.window-evidence`, `integrations.microsoft-toolchain`, `model.advanced-readback`, `model.calculated-columns`, `model.columns`, `model.dax-static-analysis`, `model.measures`, `model.relationships`, `model.source-templates`, `model.static-control-tables`, `model.tables`, `package.pbix-pbit-boundary`, `profile.data-profile-v2`, `quality.lint-rule-registry`, `quality.model-completeness-lint`, `report.bookmarks.readback`, `report.conditional-formatting`, `report.dashboard-spec-v2`, `report.design-layout`, `report.drilldown`, `report.filters.categorical`, `report.intent-parser`, `report.interaction-default-reset`, `report.interactions.overrides`, `report.pages`, `report.slicer-clear`, `report.themes`, `report.visuals.role-maps`, `report.visuals.template-clone`, `validation.microsoft-report`, `workflow.source-profile` |
| supported / `schema-golden` | `model.partition-grouped-rank`, `report.drillthrough`, `report.filters.numeric-range`, `report.filters.relative-date`, `report.filters.topn`, `report.visuals.generated`, `workflow.synthetic-source` |
| supported / `desktop-golden-pending` | `desktop.reference-harvest`, `report.slicer-authoring`, `report.visuals.category-share`, `report.visuals.matrix` |
| supported / `manual-desktop-canvas-refresh` | `report.visuals.combo-pareto` |
| planned / `unit-smoke` | `desktop.canvas-check`, `desktop.refresh-check`, `report.bookmark-mutations`, `report.slicer-sync-authoring`, `report.tooltip-pages`, `report.visuals.planned-types` |

- Dashboard specs are strict at every supported object level. `report spec
  validate` and `report build` reject unknown keys with
  `spec.unknown_field`, an RFC 6901 `pointer`, and a `didYouMean` suggestion
  when one is unambiguous; recognized sections that are not compiled still
  return `unsupported_feature`. Run `report spec fields --json` for the key
  catalog, adding `--schema` when exact model binding references are needed.
  Both `powerbi-cli.dashboard.v1` and its v2 superset are accepted. V2 defines
  model, style, layout, filter, slicer, visual-formatting, and proof sections;
  `proof.desktop` and `proof.goldens` compile to a side-effect-free
  `proofPlan` plus exact `next[]` commands. Desktop levels are never claimed by
  the Linux compiler: `proofPlan.unavailable[]` records the platform, missing
  Desktop, or missing-reference reason and the Windows instruction. Sections
  Root, page, and visual `filters[]` compile to the same typed `AddFilter`
  operations used by `report filters add`; categorical, numeric range,
  relative-date, and visual TopN filters are model/type checked and surfaced
  in `operationOutcomes[]`. Sections whose other compiler bead has not landed
  return `unsupported_feature` with the owning bead id instead of being dropped.
  Page `drillthrough` blocks likewise compile to the typed `SetDrillthrough`
  operation: `target` must resolve to an existing model column and `hidden`
  defaults to `true`. A requested `backButton:true` keeps the page binding but
  returns a `spec.feature_pending` warning naming
  `pbi-t4-pbir-catalog-expansion-sn2.8` until the proven action-button kernel
  lands; no guessed visual is emitted.
  The checked-in `examples/sales.dashboard.v2.json` demonstrates the minimal
  compiled-v2 subset, while `examples/filter-kinds.dashboard.v2.json` exercises
  every supported filter kind. To migrate any
  validated v1 spec, run `report spec upgrade --spec <v1.json> --out <v2.json>`;
  the command rewrites only `/schema`, preserves array order, normalizes object
  keys, and returns every transformed pointer. Unknown v1 keys fail with
  `spec.unknown_field` before the output is created.
  Validation failures are returned on stdout as `errors[]` objects with required
  `code` and `message` fields plus optional `pointer`, `didYouMean`, `hint`, and
  `suggestedCommands`; `spec.missing_input` additionally includes `field`,
  `reason`, `candidatesCommand`, and an `example` value. The compiler refuses
  to infer required schema, intent, and field-well inputs; optional documented
  defaults remain explicit in `defaultsApplied[]`. These are not legacy error
  strings. The exact response shape is published at
  `capabilities.responseShapes.reportSpecValidate`.
- `report spec schema --json` emits a draft 2020-12 JSON Schema for the v1 and
  v2 key surfaces. `report spec explain --schema <schema.json> --spec
  <dashboard.json> [--profile <profile.json>] --json` previews the typed,
  staged operation plan, stable handles, layout coordinates, defaults,
  unsupported sections, and proof commands without writing a project.
- The live feature boundary is `powerbi-cli features list --json`. Known but
  unimplemented or unproven report features such as tooltip pages, bookmark
  state capture/create/update/grouping, slicer selection/sync authoring, non-catalog generated visual families, visual
  drillthrough action links, cross-report drillthrough, and conditional
  formatting authoring return `error.code = "unsupported_feature"` and do not
  write fallback PBIR.
- PBIX/PBIT package commands are metadata doors, not binary writers. PBIX can
  additionally be opened as an exact managed Desktop document and queried
  read-only through the local model engine on opted-in Windows machines.
  `package inspect` classifies archive entries, `package extract` extracts only
  safe metadata/source entries by default, `package import` succeeds only when
  real allowlisted PBIP/PBIR/TMDL source files exist inside the archive,
  `package source-pack` first refuses unknown files and files in dot-directories,
  then scans every included file for credentials, PII-suspect row literals, and
  data-bearing profile v2 documents;
  non-dummy or unverified partition sources are also refused. The separate
  `package work-pack` uses the same allowlist and scans, but requires every
  partition to be a recognized credential-free materialized live source
  accepted by `handoff check --target work`; it writes source metadata only,
  never imported rows, caches, PBIX files, or local settings. Its default output
  is the sibling `<project>-work.pbit`. Finally,
  `package export-plan` emits the Desktop handoff. Opaque binary
  export/compile/pack requests are intentionally refused.
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
  `powerbi-cli.manifest.copy.json` sidecars and root `profile*.json`/`*.profile*.json`
  metadata files. A work-pack additionally contains
  the generated `powerbi-cli.work-pack.json` class marker. Other files—including every file
  below `.git`, `.vscode`, `.powerbi-cli`, or another dot-directory—cause a
  deterministic refusal listing and no archive is written.
- Programmatic visual authoring currently covers first-slice PBIR visual
  discovery with `report visuals catalog` and generated PBIR visual creation
  with `report visuals add` for card, tableEx, lineChart, areaChart,
  stackedAreaChart, clusteredBarChart, clusteredColumnChart, barChart,
  columnChart, hundredPercentStackedColumnChart,
  lineClusteredColumnComboChart, scatterChart, pieChart,
  donutChart, matrix (emitted as PBIR `pivotTable`), and slicer generated
  patterns, plus PBIR
  `queryState` generation, `report visuals set-bindings` replacement/clear
  operations for existing visuals, and guarded `report visuals delete` for
  simple visual containers that contain only `visual.json`. `report visuals
  clone` copies one simple existing visual container as template reuse, patches
  only name, position, and clone annotations, and preserves visual type,
  bindings, formatting, filters, and raw PBIR already inside `visual.json`.
  It validates table, column, and measure names against local TMDL and returns
  readback commands. Generated `--title` text is emitted as PBIR container chrome
  under `/visual/visualContainerObjects/title` (`show = true`), with annotation
  metadata retained for readback. Generated visuals omit `general.altText`
  because Microsoft powerbi-report-authoring-cli rejects that formatting
  property. Raw columns are
  refused in card Values, chart Y, matrix Values, and scatter X/Y/Size roles;
  define a measure until a Desktop-authored aggregation binding is available.
  Reusing the same model field twice in one visual is also refused because no
  Desktop-authored duplicate queryRef numbering convention is available.
  Scatter color grouping uses the canonical PBIR `Series` role. User-facing
  aliases such as `legend` are accepted on input but never written to
  `queryState` because Desktop silently leaves that field well unbound.
  Pie and donut use exactly one Category column plus one or more Y measures and
  emit the Desktop-authored default descending sort by the first Y field. Matrix
  and combo charts use explicit role contracts; combo requires Category plus
  column measures in Y and line measures in Y2. Add
  `sortDirection=Descending` (or `sort=descending` in CLI binding text) to at
  most one projected measure when the category order must follow a measure.
  Ascending and multi-key sorts remain deliberately unsupported. Matrix
  uses ordered Rows, optional Columns, and one or more Values measures; matrices
  with multiple row levels expose the native `+/-` expand/collapse controls. Slicer
  uses exactly one Values column and emits Basic (default), Dropdown, or Between
  mode under `/visual/objects/data`; Between also emits
  `/visual/objects/slider.show = true` for a visible draggable range band. It
  requires a height of at least 104 so Desktop has room to render both handles
  and the band, and never generates persisted selection state.
  The four binding families retain `manual-desktop-canvas-refresh` evidence:
  `testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json` records
  refreshed Desktop canvases with exact expected values plus live slicer
  interaction. Current title-bearing generated bytes are
  `desktop-golden-pending` until Desktop open/refresh/save re-verification;
  automated `desktop-canvas-refresh` proof and broader typed formatting remain
  open.
  `report visuals catalog` exposes one closed role-map row per generated visual
  type: required and optional roles, measure-only roles, per-role projection
  limits, mutually exclusive roles, runtime-parity rules, proof level, and
  fixture provenance. Only the pie, donut, pivotTable, and slicer rows cite
  independent Desktop-authored reference files; other rows identify their
  repository-generated proof level. `report visuals repair-bindings --dry-run`
  can propose a typed `setBindings` op for mechanical, proven repairs such as
  scatter `Details` to `Category` and bare scatter value-axis columns to Sum
  aggregations. It never writes, invents fields, or drops ambiguous bindings.
  `report visuals catalog --formatting --json` lists the complete embedded
  `formatting-catalog.v1` consumed by `report visuals set-object`: exactly eleven
  object/property pairs with their encoding, PBIR container, wildcard visual
  scope, and dated Desktop/pilot reference. The strict catalog is deterministic;
  new entries require a Desktop-authored fixture or dated pilot observation.
  `report visuals formatting list/show` inventories existing PBIR formatting
  object cards and property names with raw payloads omitted unless
  `--include-raw` is passed. `report visuals formatting extract/apply` copies
  raw per-visual PBIR formatting bundles between same-type visuals while
  replacing only `/visual/objects` and removing any forbidden root-level
  `/objects`; apply refuses copied literal text unless `--allow-literal-text` is
  passed.
  `report visuals formatting set-text` patches typed title text and visibility,
  and removes rejected legacy/shared `general.altText` metadata with
  `--clear-alt-text`, while preserving sibling formatting properties. Alt-text
  authoring is refused until Microsoft exposes a validator-supported PBIR
  location. More visual families, richer typed formatting mutations,
  `Default`/reset interaction semantics, slicer selection/sync and additional
  mode authoring, filter
  sort and arbitrary expression-level filter mutations, and conditional
  formatting still need Desktop-authored golden fixtures.
- Programmatic DAX measure authoring covers `model measures
  list/show/add/update/delete` over generated TMDL table files. Local validation
  proves file structure and readback, not DAX engine semantics.
  Add/update accepts either inline `--expression` or bounded
  `--expression-file`, plus static `--format-string` or a DAX
  `--format-string-definition` persisted as `formatStringDefinition`.
  `model dax dependencies` and `model dax lint` add offline static reference
  checks for measures and calculated columns: missing fields, ambiguous
  references, self references, simple measure cycles, and scalar `IF()`
  variables passed directly to known table-argument functions. They do not
  parse or execute the complete DAX language. On Windows, `model dax execute`
  provides a separate bounded live-engine path: the exact PBIP or PBIX document must already be
  open, `POWERBI_DESKTOP_ORACLE=1` and `--allow-data-read` are both required,
  only `EVALUATE` or `DEFINE ... EVALUATE` query forms are accepted, and the
  query text is never returned. Rows and cell text are capped because result
  data can be sensitive. PBIP live preflight ignores only each selected
  artifact's root `.pbi/` runtime directory, which Desktop creates beside the
  source definition; PBIX preflight verifies the package, report payload, and
  embedded DataModel before contacting Desktop. Offline validation, packaging,
  workflow, and handoff keep rejecting PBIP runtime files. Updates refuse blocks with unsupported Desktop-authored TMDL metadata
  instead of silently dropping it; Power BI Desktop or an explicit engine bridge
  remains the compatibility oracle.
- Read-only live semantic-model extraction covers `model live export-tmdl` on
  Windows. The exact PBIP/PBIX document must already be open, both
  `POWERBI_DESKTOP_ORACLE=1` and `--allow-model-read` are required, and the
  pinned Microsoft Modeling MCP must pass its locked handshake. The command
  connects only to the exact locally discovered engine port, exports into a
  fresh sibling quarantine, rejects links/reparse points, unexpected files,
  invalid UTF-8, oversized definitions, and credential-like text, and publishes
  only after the MCP process tree is reaped. The output contains only a
  `definition/` TMDL tree; it is not a report export or full PBIX-to-PBIP
  conversion.
- Programmatic semantic-model authoring covers `model tables list/show/add/add-calculated/rename/delete`
  and `model columns list/show/add/update/delete` with stable percent-encoded
  table/column handles, guarded output modes, and readback/validate commands.
  Rename rewrites relationship, DAX, and variation references only when
  `--rename-references` is explicit; otherwise it refuses with the reference
  list. Column updates refuse unknown Desktop-authored properties instead of
  dropping annotations or extended properties. `diff --scope model.tables` and
  `diff --scope model.columns` provide semantic table/column changes.
- Calculated-table authoring covers `model tables add-calculated` with bounded
  DAX input and `partition = calculated` TMDL output. Named-expression
  authoring covers `model expressions add/update/delete`; updates preserve
  newline style and fail closed on unknown Desktop metadata. Run `model dax
  lint` and project `lint` after writes; Desktop remains the DAX/M oracle.
- Programmatic static-table authoring covers `model tables add-static` for a
  new disconnected single-string-column selector or a small 1-10-column string
  lookup dimension backed by a generated inline `#table` partition. Cells are
  bounded, short, and screened for credential-like text; the first column is a
  unique key. Automatic relationships and arbitrary fact-table ingestion remain
  outside this guarded surface.
- Programmatic DAX calculated column authoring covers `model calculated-columns
  list/show/add/update/delete` with explicit data types, guarded output modes,
  readback commands, and `diff --scope model.calculatedColumns`. Updates refuse
  unsupported Desktop-authored TMDL metadata instead of silently dropping it.
  Input `--data-type date` is normalized to TMDL `dateTime` with a default
  `Short Date` format string, matching scaffolded date columns.
- Programmatic relationship authoring covers `model relationships
  list/show/add/update/delete` with endpoint validation, guarded output modes,
  readback commands, and `diff --scope model.relationships`. Endpoint rewiring
  is currently modeled as delete+add for clearer audit trails. Add/update can
  author `one|many` endpoint cardinalities, `active`/`inactive` state, and
  `oneDirection`/`bothDirections`/`automatic` cross-filtering behavior.
  Measure, calculated-column, and relationship writes retain the original TMDL
  file through post-write validation. A failed validation restores that file and
  returns `projectModified: false` plus rollback details.
- Programmatic partition inspection covers `model partitions list/show` with
  source kind, strict generated `#table(...)` shape/model-column/row-arity
  checks, redacted source previews, and offline safety findings. Full source and
  TMDL block readback requires `--include-source` and is refused for `review` or
  `unsafe` partitions. A table-level
  `annotation PowerBICli_SourceKind = ModelDerived` explicitly marks unknown M
  as model-derived: work handoff accepts it when no error finding remains, while
  offline handoff still requires review and rejects it.
- Guarded refresh-time ranking covers `model partitions add-grouped-rank` for
  a table with exactly one safe generated dummy partition. It resolves existing
  group/order/rank columns to their Power Query source names, sorts the rows,
  buffers each group, gives eligible rows a 1-based `Int64` index, gives
  ineligible rows zero, recombines them, and finishes with an explicit
  `Table.TransformColumnTypes`. The rank column must already be an `int64`
  placeholder in the generated table. Live, unknown, unsafe, multi-partition,
  and already transformed sources are refused; Desktop refresh is still the
  semantic oracle.
- Programmatic advanced semantic-model readback covers
  `model advanced inventory` plus `model roles|perspectives|cultures|expressions
  list/show` for TMDL metadata already present in a project. Mutating those
  advanced surfaces remains blocked until object-specific writers and fixtures
  exist.
- Source-template authoring covers `source-template list/show/add/apply` for
  credential-free SQL Server, PostgreSQL, ODBC, Excel, CSV, folder, and
  SharePoint/OneDrive rebind metadata stored
  as sidecar JSON. PostgreSQL templates record current Npgsql compatibility guidance;
  ODBC templates accept only a bare DSN name (no `;`/`=` attributes) and record
  that the named DSN must already exist there. CSV, folder, and SharePoint
  templates render `Csv.Document`, `Folder.Files`, and `SharePoint.Files`
  expressions with explicit TMDL-derived column type conversions.
  `source-template apply` is the
  explicit materialization step that replaces one safe generated dummy partition.
  With `--replace-existing` and an exact `--confirm <partition-handle>`, it can also
  intentionally retarget a recognized credential-free SQL, PostgreSQL, ODBC,
  external-file, or SharePoint partition; unknown, web, credential-bearing, and unconfirmed
  sources remain refused. Excel templates use `Excel.Workbook(File.Contents(...))`,
  promote the selected sheet/table headers, explicitly convert imported columns to
  their TMDL model types, and require an absolute workbook path when applied.
  `handoff rebind-plan` maps
  templates to partitions and can write a self-contained Markdown runbook with
  `--out <file.md>` (existing files require `--force`). Credential detection
  redacts JSON/Markdown excerpts and suppresses runbook creation. CSV and
  generic M templates are accepted only when their direct connector root and
  transformation calls stay within the workflow/source-profile closed grammar;
  credential-like text, hard-coded paths, unknown functions, and computed calls
  are refused with a pointer into the M text.
- `handoff rebind-check` is the offline, credential-free gate after a work-machine
  rebind. It checks every selected partition for a concrete supported connector,
  validates SQL/PostgreSQL/ODBC/SharePoint syntax, probes only local file/folder
  paths for existence and readability, and reports per-partition findings plus
  strict native validation. It never opens a database, SharePoint, or Desktop
  connection; use the returned `desktop open` command for separate refresh and
  canvas proof.
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
  Declarative v2 `pages[].drillthrough` uses the same `SetDrillthrough` kernel
  during `report build`, validates its target column, and defaults the page to
  hidden. `backButton:true` currently produces a structured
  `spec.feature_pending` warning for
  `pbi-t4-pbir-catalog-expansion-sn2.8`; it does not author an unproven
  action-button visual.
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
  Declarative v2 root/page/visual `filters[]` use the same AddFilter validation
  and PBIR emission path during `report build`, so a spec build and the
  equivalent CLI filter commands produce byte-identical artifacts.
- Programmatic visual formatting authoring covers raw formatting bundle
  extract/apply plus typed `report visuals formatting set-text` and
  `set-color`. `set-color` patches only static literal `title.fontColor` and
  wildcard/static `dataPoint.fill`. `report visuals formatting
  conditional-formatting list/show` can inventory existing conditional-formatting
  signals in PBIR. Conditional-formatting authoring and data-bound color
  selectors remain Desktop-fixture gated.
- Programmatic report slicer handling covers generated Basic/Dropdown/Between slicers
  through `report visuals add`/dashboard specs and inspection/state clear through
  `report slicers list/show/clear`. List output hides raw
  slicer visual JSON by default, returns both `slicer:` and underlying
  `visual:` handles, summarizes field bindings and slicer state, and warns when
  slicer metadata may persist selected semantic-model values. `clear` removes
  persisted selection filters matching the slicer binding with guarded output
  modes while preserving bindings, layout, and formatting. Generated slicers
  contain no `general.filter` or cached selection state. Additional modes,
  default selections, selection mutation, and sync groups remain Desktop-fixture
  gated; the generated Basic/Dropdown binding baseline is
  `manual-desktop-canvas-refresh` proven by the checked-in 2026-07-10 canvas
  proof record.
- Programmatic report interaction authoring covers `report interactions
  list/show/set/disable/reset` for explicit PBIR page `visualInteractions` overrides.
  `disable` upserts an explicit `NoFilter` row; `set` upserts DataFilter,
  HighlightFilter, or NoFilter with guarded output modes, stable source/target
  visual resolution, duplicate-row refusal, readback, wireframe, inspect, and
  validate commands. Missing rows still mean Power BI default interaction
  behavior, not `NoFilter`. `reset` removes one matching explicit row and
  reports that the absent row restores the target visual's documented default;
  the local proof level is `unit-smoke` and Desktop canvas confirmation remains
  open.
- Programmatic report bookmark handling covers `report bookmarks list/show` for
  raw PBIR `definition/bookmarks/*.bookmark.json` readback plus `bookmarks.json`
  order/group metadata. Metadata-only mutation is supported for display-name
  edits, flat reorder, and guarded delete. Capturing bookmark state, creating
  new stateful bookmarks, updating captured visual/filter/slicer state, and
  group reorder remain unsupported until Desktop-authored golden fixtures exist.
  List output hides raw bookmark JSON by default and marks bookmark state that
  may persist filter, slicer, highlight, or selected semantic-model values.
- Programmatic report design/layout authoring covers `report design-plan`,
  `report layout auto`, deterministic `report wireframe export` JSON/SVG/HTML,
  and `report drilldown set-hierarchy`. Design-plan is a
  read-only profile with exact next commands; auto-layout uses the deterministic
  twelve-column design grid and eleven named templates (`overview`,
  `time-series`, `ranking`, `distribution`, `comparison`, `detail-table`,
  `drillthrough-detail`, `exception-list`, `matrix-focus`, `scatter-focus`, and
  `kpi-strip-trend-breakdown`). Templates expose named slots with preferred
  visual families, emit SVG-free JSON previews and overlap/minimum-size
  invariants, and support standard (1280x720), wide (1920x1080), or explicit
  page-size/grid overrides. Wireframe SVG/HTML previews are written outside
  the project with `--out` or can be reviewed inline with `--dry-run`; they
  embed their CSS and never fetch external assets. Layout mutations support `--dry-run`, `--out-dir`, and
  guarded `--in-place`; legacy `--preset overview|analysis|detail|grid`
  remains an alias for the corresponding templates; auto-layout rewrites only
  visual `position` blocks;
  drilldown hierarchy replaces a chart's Category
  projections with two or more resolved model columns, marks the first field
  active as the initial level, and explicitly enables its visual-header drill
  controls. Line, area, bar, column, and combo charts
  are supported when their numeric field wells are already bound. Scatter is
  refused because Microsoft's report validator permits only one Category
  projection for that visual.
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
- `handoff check` defaults to an offline/dummy target and fails on real
  connectors. Use `handoff check <project> --target work` for a canonical PBIP
  whose partitions already use recognized SQL Server, PostgreSQL, ODBC, Web,
  or file connectors. Work-target validation accepts those connectors but still
  fails on credentials, Power BI caches/binaries, local settings, embedded data
  files, and unknown partition sources. The result reports `target`,
  `sourceMode`, `safeForOfflineHandoff`, and `safeForWorkHandoff` explicitly.
  Structurally valid literal tables with PII-suspect rows remain `review`.
- `handoff rebind-check` verifies that a rebinding is materialized and
  credential-free without evaluating Power Query. It returns `safe`, `review`,
  or `unsafe`, stable partition handles, registered finding codes, and a
  `refresh.status` of `not-run` to make the Desktop boundary explicit.
- Dashboard specs and strict PBIR validation reject slicers shorter than Power
  BI's 76-pixel minimum, preventing a common source of clipped controls before
  the report reaches Desktop.
- Credential matching is case-insensitive and separator-tolerant for anchored
  key/value syntax (`password`, `pwd`, `pass`, account/access/SAS/API keys,
  `sig`, user identifiers, secrets/tokens), recognizes Bearer authorization
  headers plus GitHub/AWS token formats, and redacts matched values as `***`.
  Bare prose is not enough: German UI text such as `Passwort ändern` and words
  containing `pass` do not match without credential assignment syntax.
- `lint` now includes a small BPA-style report/model pass: DAX static findings,
  duplicate page/visual titles, and validator-rejected `general.altText`
  placements with an explicit `--clear-alt-text` remediation. Missing alt text
  is valid until Microsoft exposes a supported PBIR location. `lint --rules`
  lists the single versioned registry used by lint, DAX/M checks, and report
  audit; `lint --explain <rule-id>` returns one rule's family, default severity,
  summary, remediation, optional sanitize action, and example finding without
  requiring a project. M lint raises the error-level
  `m.duplicate_step_name` rule when a partition or named expression defines a
  let-step name more than once (including quoted names and the final step before
  `in`); findings include both source positions and ignore comments/string
  literals. M lint also reports `m.untyped_expansion` and `m.unbuffered_reuse`
  warnings for unsafe expansion and reused table values without buffering. The
  registry includes a typed, currently empty design family so future design
  lint cannot introduce ad-hoc ids.
- Model completeness lint adds warning-only checks for measures without an
  explicit static or dynamic format, malformed custom format strings, visible
  relationship keys, both-direction fact-to-dimension relationships, and
  columns unused by visuals, measures, or relationships. Run lint or triage
  for the combined scorecard, or model dax lint for the DAX format checks;
  every finding carries a stable handle and a fix hint.
- Structural validation reports an empty PBIR visual container as a missing
  `visual.json` with an explicit remove-or-restore repair, instead of allowing a
  later deep-inspection `file_not_found` failure.
- Native validation errors and warnings are structured findings with a stable
  registry code, unchanged human message, source `path`, severity, and an RFC
  6901 `pointer` (the empty pointer denotes a whole-file/TMDL finding). Every
  emitted code is explainable with `lint --explain <code> --json` and listed by
  the validation capability contract.
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
- `desktop open` accepts PBIP projects and PBIX documents, creates the single CLI-owned interactive Desktop session, and
  returns its exact observed PID, creation time, receipt path, and cleanup command.
  `desktop close` is idempotent and closes only that recorded PID and verified
  descendants. A missing, exited, or PID-reused session receipt never triggers a
  title-wide or executable-wide kill. Opening a new managed session first closes
  the prior owned session. Never use raw `Start-Process` for CLI-managed testing.
  `desktop open --preflight strict|normal|skip`, `desktop open-check`, and
  `desktop screenshot` are one-shot opt-in Windows oracle commands. They always
  attempt bounded identity-checked cleanup and expose any
  unresolved ownership in the response. `--timeout-ms` is one watchdog budget for the bounded version probe,
  pre-launch process baseline, file-association launch, and window/title polling.
  `proof.level` uses the canonical `unit-smoke` level; launch and exact normalized
  project-stem matches are reported separately as `proof.observedStage`. Window
  candidates must be `PBIDesktop*`; `AnnualSales` never matches project `Sales`.
  When several open reports share the same title, the observer prefers the
  association-launch PID and then a new post-baseline Desktop PID. It reports
  `desktop_title_ambiguous` instead of selecting an arbitrary pre-existing
  window when neither identifies the launch.
  Cleanup never targets baseline/pre-launch processes, exact-title peers, or
  unrelated processes sharing the Desktop executable; it reports a reason for
  every explicitly owned PID it targets. Screenshot output must be a PNG outside the project
  directory. Capture uses a same-directory temporary file, verifies foreground
  ownership by the selected Desktop PID or a descendant process, and
  publishes/replaces the requested PNG only after success.
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
- `desktop refresh-check` and `desktop canvas-check` are cataloged forward-compatible
  Desktop proof commands. They currently return `error.code = "unsupported_feature"`
  without launching Desktop or writing evidence; refresh completion and canvas
  rendering proof will be implemented by the T9 Windows work.
- Validation checks file structure, parseable JSON, page references, TMDL table
  presence, relationship endpoints, and offline hazards. It is not a Power BI
  Desktop open proof.
- `.pbix`, `.pbit`, `.abf`, `.pbi/`, embedded data files, and
  `localSettings.json` are treated as unsafe for the home/offline workflow.

### Pilot results

The August 2026 production pilot is the field-evidence source for the current
boundaries and priorities. Read the captured hand-patched idioms, proof notes,
and follow-up beads in [`docs/pilot-lessons.md`](docs/pilot-lessons.md); the
September sequencing and compiler work are tracked in
[`docs/bridge-plan-2026-09.md`](docs/bridge-plan-2026-09.md).

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
