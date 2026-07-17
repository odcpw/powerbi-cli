# PBIR Desktop Oracle Notes

Date: 2026-06-23, updated 2026-07-17

This document records the compatibility facts learned while making
CLI-generated PBIP projects open and render in Power BI Desktop. Do not
rediscover these by trial and error.

## Rule Of Record

Power BI Desktop is the oracle. Microsoft schemas and validators are necessary
but not sufficient. A generated report is not proven compatible until Desktop:

1. opens the `.pbip`;
2. shows the report canvas, not just the window title;
3. refreshes dummy partitions;
4. renders expected visuals with data;
5. does not show unresolved report-definition issues.

The current manual Desktop proof records are:

```text
testdata/desktop-proof/flat-ops.desktop-proof.json
testdata/desktop-proof/scatter-bubble.desktop-proof.json
testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json
testdata/desktop-proof/desktop-acceptance-2026-06-24.visual-objects.json
```

Desktop proof observed, across these projects: cards, line charts, clustered
bar charts, populated tables, and page/report filters visible in the filter
pane, all rendering correctly after refresh. Earlier Desktop proof sessions
that used a since-removed private fixture found the same categorical-filter
and root-objects issues documented below; that evidence record is retained in
a private repository.

## Current CLI Proof Contract

`desktop open-check` is intentionally honest about its proof strength:

- `proof.level = "unit-smoke"` is the canonical level of the current command
  implementation. Runtime observations do not invent additional proof levels.
- `proof.observedStage = "desktop-launch"` means local validation passed and
  Desktop was started for the `.pbip`.
- `proof.observedStage = "desktop-window"` means a `PBIDesktop*` process exposed
  a title whose normalized project stem exactly matched the `.pbip` stem. The
  committed proofs record plain titles such as `WorkshopOperations` and
  `FacilityPortfolio`; the matcher also accepts exact ` - Power BI Desktop`
  hyphen/en-dash/em-dash suffix variants. It never lets `AnnualSales` satisfy
  project `Sales`.
- `proof.claimedCompatibility` remains `false` until a future
  `desktop-canvas-refresh` proof observes rendered pages, refresh completion,
  and absence of Desktop issue banners/dialogs.
- `proof.signals.windowObserved` is true or false after polling actually runs,
  and null when observation was not attempted. `titleMatched` is true or false
  when a titled window was observed, and null otherwise.
- `proof.signals.observedWindowTitle` records the selected non-empty title;
  `proof.signals.observation` records watchdog budget, launch/total elapsed
  time, poll count, completion reason, timeout state, and candidate PIDs.
- `proof.unprovenSignals` continues to list signals the command did not
  establish. Canvas, blank-canvas, refresh, and issue-dialog signals remain
  unproven by both Desktop commands.
- `oracle.desktopVersion` is populated only after Windows/platform and explicit
  oracle opt-in checks, using the bounded setup watchdog. Plain `doctor`
  detection does not launch a version-probe subprocess.

This distinction exists because a process/window-title launch can still leave a
blank canvas or unresolved table/relationship issues. Agents may use launch
proof as a smoke check, but must not call it Desktop compatibility proof.

### Window Watchdog And Screenshot Evidence

`desktop open-check --timeout-ms N` treats `N` as one watchdog budget for:

1. probing Desktop file-version metadata with the bounded command runner;
2. recording the pre-launch `PBIDesktop*`/`msmdsrv` process baseline;
3. launching the `.pbip` through Windows file association;
4. polling `PBIDesktop*` processes for `MainWindowTitle`.

Windows file association can return a short-lived shell/proxy PID instead of
the long-lived Desktop PID. The baseline/delta polling is therefore required
both for observation and for scoped cleanup. Every cleanup target is reported as
`proof.signals.cleanup.targeted[]` with its PID, ownership reason, and creation
time. A process is eligible only as the non-baseline association PID or an owned
descendant, an exact project-title match, or an executable-path match created
after the recorded launch timestamp. Creation time is checked again immediately
before `Stop-Process`; baseline, pre-launch, unknown-creation, and PID-reuse cases
are never killed. `closed=true` means every targeted PID was verified dead and no
ownership/stop error remained.

`desktop screenshot <project> --out <file.png>` performs the same preflight,
launch, exact-title observation, and cleanup workflow. It activates only the
matched `PBIDesktop*` PID and then verifies the actual foreground PID through
`user32!GetForegroundWindow` and `GetWindowThreadProcessId`. Capture is written
to a unique temporary file beside the destination; the previous PNG is replaced
only after a non-empty capture succeeds. The output path must end in `.png` and
resolve outside the PBIP project directory.

The JSON records `activationSucceeded`, `foregroundVerified`, and
`foregroundProcessId`. Failed foreground verification publishes no PNG and is
`oracle_failed`. The explicit `--allow-unverified-capture` escape hatch permits
capture anyway but risks recording unrelated sensitive screen content, leaves
`proof.passed=false`, and emits a warning. Every response contains `changes`;
successful PNG creation/replacement contributes one file change and all other
paths return `changes: []`.

The screenshot is evidence capture for manual or screen-agent review. The CLI
does not crop to the Power BI window, parse pixels, identify visuals, detect a
blank canvas, inspect issue banners/dialogs, or verify refresh. Consequently
`proof.claimedCompatibility` and
`screenshot.automatedCompatibilityProof` are always false.

Status and exit-code semantics:

- Preflight validation failure: exit 10, no Desktop launch.
- Non-Windows: exit 2 with `error.code = "unsupported_feature"`, before oracle
  opt-in evaluation.
- Oracle disabled or Desktop not found on Windows: exit 30
  (`oracle_unavailable`).
- Launch command fails or times out before a PID is confirmed, observer/capture
  machinery fails, or spawned-process cleanup fails: exit 40
  (`oracle_failed`).
- `open-check` launch succeeds but window/title observation exhausts the
  watchdog: exit 0 with `proof.level = "unit-smoke"`,
  `proof.observedStage = "desktop-launch"`, and
  `proof.status = "window-observation-timeout"` or
  `"window-title-timeout"`. This is partial launch proof, not oracle failure.
- `screenshot` launch succeeds but no exact project-title match appears, so no
  PNG is captured: exit 20 (`proof_incomplete`). This distinguishes missing
  evidence from an oracle subsystem failure.
- `screenshot` finds the exact project window but cannot verify foreground PID
  ownership: exit 40 (`oracle_failed`) and no PNG is published, unless the
  explicit risky override was passed.
- Matching window/title observation (and, for `screenshot`, a written PNG):
  exit 0. Compatibility is still not claimed.

## Drillthrough parameters need boundFilter + fieldExpr + paired Drillthrough filter entries

A live Desktop Store 2.155.756.0 session on 2026-07-10 exposed a false-positive
open check: Desktop opened a CLI-authored drillthrough page without an error,
but the page's **Add drill-through fields here** well stayed empty and source
visuals never offered a **Drill through** context-menu entry. A parameter name
without a linked field/filter did not bind anything: the captured failing
parameter had no `fieldExpr`, no `boundFilter`, and no paired filter entry.

The working reference is the MIT-licensed `microsoft/BCApps` Sales app page
`429930d2d08538d4d2bb`. Its operative shape links `boundFilter` to a same-named
`filterConfig.filters[]` entry and repeats the complete column expression in
`fieldExpr` and `field`:

```json
{
  "pageBinding": {
    "name": "8371b6f7ec60805c3604",
    "type": "Drillthrough",
    "parameters": [
      {
        "name": "Param_Filterad3bc94a0c4e98685abc",
        "boundFilter": "Filterad3bc94a0c4e98685abc",
        "fieldExpr": {
          "Column": {
            "Expression": { "SourceRef": { "Entity": "Item" } },
            "Property": "Item No. & Description"
          }
        }
      }
    ]
  },
  "filterConfig": {
    "filters": [
      {
        "name": "Filterad3bc94a0c4e98685abc",
        "howCreated": "Drillthrough",
        "type": "Categorical",
        "field": {
          "Column": {
            "Expression": { "SourceRef": { "Entity": "Item" } },
            "Property": "Item No. & Description"
          }
        }
      }
    ]
  }
}
```

The paired filter deliberately has no `filter` body. `report drillthrough set`
now emits this linkage with deterministic content-addressed names; `show`
surfaces `boundFilter` and `fieldExpr`, and `clear` removes both the binding and
Drillthrough-created filter entries.

The Desktop-authored reference sets `visibility = "HiddenInViewMode"` but does
not set root `type = "Drillthrough"`. The CLI retains that legacy root marker
for output compatibility because Desktop 2.155 accepted it; the linked binding
and filter are the Desktop-recognized behavior-bearing shape. The corrected
output has now passed the manual end-to-end proof recorded below. Visual
drillthrough action links and cross-report drillthrough remain golden-gated.

## Manual Canvas Proof Earned On 2026-07-10

Power BI Desktop Store 2.155.756.0 refreshed and rendered CLI-generated
`pieChart`, `donutChart`, `pivotTable` (matrix), and clean Basic `slicer`
surfaces with exact expected values; slicer interaction was also exercised.
The evidence is
`testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json`.
The corrected same-report drillthrough binding separately passed well
registration, level-correct context-menu discovery, filtered navigation, and
carried-filter checks in a manual Desktop canvas/refresh session (2026-07-10,
Desktop Store 2.155.756.0); that evidence record is retained in a private
repository.

These records establish `manual-desktop-canvas-refresh` evidence for the tested
binding/canvas baselines. The current generator now adds a Desktop-authored
title-container shape, so the changed complete visual bytes are
`desktop-golden-pending` until a new open/refresh/save pass. This remains manual
evidence, not an automated CLI oracle; implementing `desktop-canvas-refresh`
remains the open P0.

### Cartesian hierarchy projections do not require an active marker

A private-repo column chart used the CLI's cartesian projection grammar: three
ordered `Category` projections at increasing granularity, none with
`active: true`. In the 2026-07-10 Desktop Store 2.155.756.0 session,
all three drill levels worked through the context menu and cross-filtered the
line chart and matrix. That evidence record is retained in a private
repository; the shape it validated is unchanged and still emitted here.

The archived Desktop-authored pie, donut, matrix, and slicer fixtures do mark
their first Category/Rows/Columns/Values projection active, but the reference
directory contains no cartesian hierarchy fixture that contradicts the live
result. Therefore cartesian Category projections deliberately remain without
`active`; adding it would invent a shape despite working Desktop evidence.

## Sources Used

Primary sources:

- Microsoft PBIP project overview:
  <https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-overview>
- Microsoft report/PBIR project folder docs:
  <https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-report>
- Microsoft PBIR page schema:
  <https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/2.0.0/schema.json>
- Microsoft Power BI Report Authoring skill overview:
  <https://learn.microsoft.com/en-us/power-bi/developer/agentic/power-bi-report-authoring-skill-overview>
- Microsoft `skills-for-fabric` Power BI authoring skill:
  <https://github.com/microsoft/skills-for-fabric>
- Microsoft `@microsoft/powerbi-report-authoring-cli` validator, version
  `0.1.1`, fetched from npm during the Desktop proof pass.

Permissive comparator:

- `MinaSaad1/pbi-cli`, tag `v3.11.1`, MIT:
  <https://github.com/MinaSaad1/pbi-cli>

Reference-only/quarantined comparator:

- `maxanatsko/pbir.tools`, tag `v0.9.25`, custom non-commercial/no-derivatives:
  <https://github.com/maxanatsko/pbir.tools>
- `data-goblin/power-bi-agentic-development`, inspected at commit
  `9704f1d00f37f3d79a5d65b618571d0088ce6478`, GPLv3:
  <https://github.com/data-goblin/power-bi-agentic-development>
  - Useful reference signals: PBIR examples for bookmarks, slicers/syncGroup,
    visualInteractions, visualTooltip, drillFilterOtherVisuals, broader visual
    catalogs, and formatted visuals.
  - Use only as quarantined reference material. Do not copy examples/templates
    into this repo; recreate behavior from Microsoft docs and our own Desktop
    fixtures.
- `RuiRomano/workshops-pbig-2026`, inspected at commit
  `09c69857e5f84a6587895aed3de5cab0460bf2a2`:
  <https://github.com/RuiRomano/workshops-pbig-2026>
  - Candidate 2026 workshop material to re-check for future PBIP/PBIR fixture
    ideas; not currently a source for implementation.

For license handling, see `docs/clean-room-research.md`.

## Desktop Findings

### Scatter Legend UI uses PBIR `Series`

Desktop's scatter field well is labelled Legend, but the PBIR query-state role
that actually binds color grouping is `Series`. A generated scatter using
`queryState.Legend` can pass JSON/schema checks and open without a visible
error, while Desktop silently leaves Legend empty and renders no grouped
bubbles. Replacing only the role key with `Series` immediately restored the
legend and bubbles in Desktop Store 2.155.756.0.

Implemented guardrails:

- scatter catalog output exposes `Series`, not `Legend`;
- CLI input aliases `legend`, `series`, `color`, and `colour` normalize to
  `Series`;
- report builders and mutations validate Series as the optional grouping
  column;
- project validation rejects stale `queryState.Legend` on a scatter and tells
  the caller to use `Series`;
- the scatter archetype and regression fixtures use `Series`.

The same validation pass initially exposed a useful catalog omission: standard
Category/Y charts can carry raw `queryState.Tooltips`. A Desktop-rendered line
chart used this for raw values alongside a transformed log-scale Y measure.
Category/Y catalog roles now include optional column-or-measure Tooltips so the
validator rejects only genuinely unsupported stored roles.

### DAX `IF()` cannot select a table variable

`IF()` is scalar in DAX. A pattern such as
`VAR Chosen = IF(condition, VisibleRows, TopRows)` may survive reference-only
lint, but Desktop rejects the measure once `Chosen` is passed as a table to
`CONTAINS`, `TREATAS`, `COUNTROWS`, or another iterator/table consumer. Branch
around the calculation instead: perform the table-consuming expression once in
each scalar IF branch.

`model dax lint` and `validate --strict` now report
`dax.table_variable_scalar_if` when a variable assigned directly from `IF()` is
later used as the first argument of a known table-consuming function. This is a
conservative pattern rule, not a complete DAX parser; Desktop refresh remains
required.

### Bounded Desktop DAX query execution

`model dax execute` closes the gap between offline DAX lint and manual query
entry in Desktop. It attaches only to the one running `PBIDesktop*` process
whose command line contains the exact canonical PBIP path, follows that process
to its child `msmdsrv` workspace, reads the local port, and uses Desktop's
bundled `Microsoft.PowerBI.AdomdClient.dll` from a private temporary copy. The
temporary script, query, and DLL are removed after the bridge process exits.

The bridge is intentionally opt-in and read-only:

- Windows, an already-open exact PBIP, `POWERBI_DESKTOP_ORACLE=1`, and
  `--allow-data-read` are all required;
- Desktop is never auto-launched and the model is never written;
- the query must begin with `EVALUATE` or use `DEFINE ... EVALUATE`;
- XMLA/model-mutation payloads are refused before Desktop is contacted;
- query bytes, returned rows, text characters per cell, and runtime are bounded;
- output includes a stable query fingerprint and length, never the query text;
- returned rows are not redacted and must be treated as potentially sensitive.

Example:

```powershell
$env:POWERBI_DESKTOP_ORACLE='1'
powerbi-cli model dax execute `
  --project .\build\sales `
  --query 'EVALUATE ROW("Revenue", [Total Revenue])' `
  --allow-data-read --max-rows 10 --json
```

A synthetic live smoke test on 2026-07-17 used Power BI Desktop Store
2.155.756.0 and returned both a literal `ROW` query and report measures through
the bridge. Unit tests cover refusal, privacy metadata, query-form guards,
bounds, and the generated bridge contract. This proves bounded query execution;
it does not prove canvas rendering, refresh success, or every stored measure.

Microsoft's separate Power BI Desktop IPC Bridge preview should not be confused
with this DAX bridge. As documented on 2026-07-15, its discoverable methods cover
application state, report-page screenshots, and file reload, but not DAX query
execution. Microsoft also documents the established external-tools architecture:
Desktop hosts an Analysis Services model on a dynamically assigned local port,
and Analysis Services client libraries can execute DAX queries. This command
uses that model engine while the official IPC manifest lacks a query method. Its
exact process/workspace discovery is therefore a version-sensitive boundary;
prefer a future official IPC query method if Microsoft adds one.

### Enhanced PBIR visual formatting location

Observed on 2026-06-24 with Power BI Desktop Store build path:

```text
C:\Program Files\WindowsApps\Microsoft.MicrosoftPowerBIDesktop_2.155.756.0_x64__8wekyb3d8bbwe\bin\PBIDesktop.exe
```

Desktop rejects enhanced PBIR visual containers that put `objects` at the root
of `visual.json`.

Rejected shape:

```json
{
  "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.9.0/schema.json",
  "name": "VisualContainer...",
  "objects": {
    "general": []
  },
  "visual": {
    "visualType": "card"
  }
}
```

Desktop modal text:

```text
Your report has issues that could not be resolved
An additional property 'objects' was included in the root property of visuals/<visual>/visual.json.
```

Accepted shape:

```json
{
  "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.9.0/schema.json",
  "name": "VisualContainer...",
  "visual": {
    "visualType": "card",
    "objects": {
      "general": []
    }
  }
}
```

Implementation rule:

- Generated visuals must not emit root-level `objects`.
- Visual-specific formatting and alt text belong under `/visual/objects`.
- Visible visual-container chrome titles belong under
  `/visual/visualContainerObjects/title`. The archived Desktop-authored slicer
  proves literal `text` plus literal `show`; generated visuals emit the supplied
  title text with `show = true` there while retaining alt text and annotation
  metadata for accessibility/readback.
- Formatting/style bundle apply must not write root-level `objects`; if a
  bundle contains them, treat them as read-only compatibility material or skip/
  reject the write instead of generating Desktop-rejected PBIR.

### PBIR report definition version

For the current foldered PBIR report layout, generated reports must write:

```json
{
  "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/versionMetadata/1.0.0/schema.json",
  "version": "2.0.0"
}
```

`version: "1.0.0"` opened as a title-bar-only project and failed Desktop
upgrade with:

```text
The report has no pages. Reports must have at least one page.
```

Implemented guardrails:

- `scaffold` writes `definition/version.json` with `version: "2.0.0"`.
- `lint` emits `pbir.report_definition_version` if a generated PBIR report is
  not using the Desktop round-trip-proven version.

### Categorical filters

Do not write this simplified shape into `filterConfig.filters`:

```json
{
  "type": "Categorical",
  "filter": { "values": ["Enterprise"] },
  "howCreated": "powerbi-cli"
}
```

Desktop rejects it. The Microsoft validator reports missing `From` and `Where`
and rejects extra `values`.

Write full PBIR categorical filters:

```json
{
  "name": "PowerBICliReportDimCustomerSegmentFilter",
  "type": "Categorical",
  "field": {
    "Column": {
      "Expression": { "SourceRef": { "Entity": "DimCustomer" } },
      "Property": "Segment"
    }
  },
  "filter": {
    "Version": 2,
    "From": [
      { "Name": "d", "Entity": "DimCustomer", "Type": 0 }
    ],
    "Where": [
      {
        "Condition": {
          "In": {
            "Expressions": [
              {
                "Column": {
                  "Expression": { "SourceRef": { "Source": "d" } },
                  "Property": "Segment"
                }
              }
            ],
            "Values": [
              [{ "Literal": { "Value": "'Enterprise'" } }]
            ]
          }
        }
      }
    ]
  },
  "howCreated": "User"
}
```

Important detail: inside `filter.Where`, use `SourceRef.Source`, not
`SourceRef.Entity`. The `Source` value is the alias declared in `filter.From`.
The top-level `field` still uses `Entity`.

Power BI literal encoding used by `report filters add`:

- strings: `'text'`, with single quotes doubled inside the string;
- signed/unsigned integers: `123L`;
- doubles: `1.25D`;
- booleans: `true` or `false`.

Implemented guardrails:

- `report filters add` emits full PBIR categorical filters.
- `validate --strict` rejects legacy `filter.values` in `filterConfig.filters`.
- `validate --strict` requires categorical `filter.Version = 2`, non-empty
  `filter.From`, and non-empty `filter.Where`.
- `validate --strict` warns when a `Where` SourceRef uses `Entity` instead of
  `Source`, or references an alias not declared in `From`.

### Filter names

Desktop enforces filter names between 1 and 50 characters. The Microsoft npm
validator did not catch this; Desktop did.

Observed Desktop modal:

```text
Invalid filter name 'PowerBICliReportDimCustomerAccountManagerNameFilter' in report.
Filter names cannot be empty and must be between 1 and 50 characters.
```

Implemented guardrails:

- Generated filter names are always 50 chars or fewer and include separate
  short hashes for the raw scope/table/column/type identity and the complete
  condition. Sanitization collisions such as `Sales-A[Region]` versus
  `Sales A[Region]` therefore stay distinct, as do different conditions on the
  same target/type.
- Repeating the exact same generated condition retains the same deterministic
  name and fails loudly as a duplicate instead of manufacturing an ordinal.
- User-supplied `--name` values longer than 50 chars are rejected.
- `validate --strict` rejects missing, empty, non-string, or overlong
  `filterConfig.filters[].name`.

### Microsoft validator scope

`@microsoft/powerbi-report-authoring-cli validate` is useful and should stay in
the local Desktop proof workflow, but it is not enough by itself.

It caught:

- bad categorical filter body;
- `filter.values` extra property;
- missing `From`;
- missing `Where`;
- invalid `howCreated` value.

It missed:

- overlong filter names that Desktop rejected.

Therefore:

```text
our validate --strict
-> Microsoft validator when available
-> desktop open-check with exact project-title observation
-> desktop screenshot with foregroundVerified=true and cleanup.closed=true
-> manual Power BI Desktop refresh + visual review
```

For the automated portion of that loop, require `changes` to contain exactly one
PNG create/replace entry, inspect every `cleanup.targeted[].reason`, and retain
the prior PNG when any capture step fails. Do not use
`--allow-unverified-capture` in routine proof runs; its output can include
unrelated sensitive windows and is deliberately not a passed proof signal.

## Reproduction Commands

Generate the proof project:

```powershell
cargo run --quiet -- --json scaffold --schema examples\archetypes\regional-sales.schema.json --out-dir build\desktop-proof-v4 --force
cargo run --quiet -- report filters add --project build\desktop-proof-v4 --scope report --target "DimCustomer[Segment]" --value Enterprise --in-place --json
cargo run --quiet -- report filters add --project build\desktop-proof-v4 --page page:ReportSectionOverview --target "DimCustomer[Größenklasse]" --value Groß --in-place --json
```

Validate locally:

```powershell
cargo run --quiet -- validate --strict build\desktop-proof-v4 --json
```

Validate with Microsoft's npm validator:

```powershell
node <powerbi-report-authoring-cli>\dist\cli.js validate <workspace>\powerbi-cli\build\desktop-proof-v4 --format json
```

Expected result:

```json
{
  "data": {
    "result": "succeeded",
    "errorCount": 0,
    "warningCount": 0
  }
}
```

Open in Desktop:

```powershell
Start-Process -FilePath (Resolve-Path 'build\desktop-proof-v4\RegionalSales.pbip')
```

Click `Refresh now` if Desktop says some tables have incomplete or no data. The
dummy `#table(...)` partitions should then populate the visuals.

## What Needs To Be Added Next

The next work should turn this one-off proof into repeatable infrastructure.

Current additional manual Desktop proof records:

- `testdata/desktop-proof/flat-ops.desktop-proof.json`: generated card,
  clustered bar chart, and table opened in Desktop, refreshed, rendered, and
  cleaned up.
- `testdata/desktop-proof/scatter-bubble.desktop-proof.json`: generated
  scatter/bubble chart and table opened in Desktop, refreshed, rendered, and
  cleaned up.

These are useful evidence, but they are not yet automated CLI canvas proof.
`desktop open-check` reports exact-title matching as
`proof.observedStage=desktop-window`; the canonical proof level remains
`unit-smoke`, and canvas/refresh automation below still has not landed.

### P0: Desktop Proof Automation

- Delivered 2026-07-09:
  - [x] Enforce `--timeout-ms` as a total launch + window-observation
    watchdog and report partial launch proof honestly on observation timeout.
  - [x] Observe non-empty Desktop main window titles across the association PID
    and newly appearing `PBIDesktop*` processes; record title matching, timing,
    and process IDs.
  - [x] Add `desktop screenshot <project|pbip> --out <file.png>` for
    primary-display evidence outside the project, with scoped cleanup and
    explicit manual/agent-review framing.
- Hardened 2026-07-10:
  - [x] Require `PBIDesktop*` for every window candidate and exact normalized
    project-stem title matching.
  - [x] Verify foreground PID ownership before capture, preserve prior evidence
    on failure, and report PNG create/replace changes.
  - [x] Restrict cleanup to timestamped ownership evidence, recheck creation time
    before every kill, preserve baseline PIDs, and report per-PID rationale.
  - [x] Bound and defer version probing until after Windows and explicit opt-in
    checks; return `unsupported_feature` on other platforms.
- Upgrade `desktop open-check` from launch/title proof to canvas proof:
  - detect unresolved issue modals;
  - fail on blank report canvas;
  - refresh dummy partitions;
  - inspect the page tabs and data pane;
  - inspect captured screenshots or other UI signals rather than treating the
    artifact's existence as canvas proof.
- Add `desktop refresh-check <project|pbip>`:
  - opens Desktop;
  - refreshes dummy partitions;
  - fails if visuals remain blank when dummy rows exist.
- Extend the delivered primary-display screenshot command with optional
  page/window targeting only after a reliable public UI/bridge surface is
  available.
- Extend proof metadata beyond the delivered Desktop version, project/title,
  timing, process, cleanup, and screenshot-path fields: page count, refresh
  result, and any modal/banner text still remain.

### P0: Golden Fixtures

- Existing committed golden summaries:
  - `testdata/golden/sales.summary.json`: compact generated baseline.
  - `testdata/golden/sales-desktop-filter-contract.summary.json`: sales sample
    with report/page categorical filters and normalized PBIR filter contract
    fields.
  - `testdata/golden/archetypes/regional-sales.summary.json`: drillthrough
    chain, TopN-by-measure filters on two visuals, multi-page slicers, and
    non-ASCII column/measure round-trip.
  - `testdata/golden/archetypes/flat-ops.summary.json`: generated flat
    operations archetype with card, clustered bar chart, and table.
  - `testdata/golden/archetypes/scatter-bubble.summary.json`: generated
    scatter/bubble archetype with scatter chart and table.
  - `testdata/golden/archetypes/catalog-proof.summary.json`: generated pie,
    donut, matrix (`pivotTable`), clean Basic slicer, and line-chart control.
    Strict validation, hygiene audit, and handoff checks pass, and the generated
    visual families are `manual-desktop-canvas-refresh` proven by
    `testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json`.
    Automated canvas/refresh assertions and broader formatting coverage remain
    open.
- Add provenance notes for Desktop-authored refresh/save fixtures once canvas
  automation exists.
- Add Desktop-authored minimal fixtures for:
  - blank one-page report;
  - card;
  - tableEx;
  - line chart;
  - clustered bar/column chart;
  - slicer;
  - page filter;
  - report filter;
  - theme-applied report;
  - conditional-formatting report.
- Each fixture needs:
  - source `.pbip` folder;
  - normalized summary JSON;
  - expected `validate --strict` output;
  - notes on Desktop version used to create it.

### P0: Microsoft Schema/Validator Integration

- Decide whether `validate --strict` should:
  - vendor the specific Microsoft JSON schemas we rely on;
  - shell out to `powerbi-report-author validate` when present;
  - or expose a separate `validate --microsoft` optional oracle.
- Add local rules for Desktop-only constraints the Microsoft validator misses,
  starting with filter name length.
- Add JSON-pointer-like paths and stable diagnostic codes for PBIR validation
  failures. String-only messages are not enough for agent repair loops.

### P1: Filter Coverage

Existing:

- list/show/add/delete/clear;
- categorical `In` filters.

Add:

- `report filters update` for replacing values without delete+add;
- `report filters set-visibility` for filter pane behavior;
- advanced filters;
- range filters;
- TopN filters;
- relative date/time filters;
- include/exclude filters;
- sort/order metadata where Desktop emits it.

Every new filter type needs a Desktop-authored golden fixture first.

### P1: Visual Catalog Hardening

Existing generated visual families render for generic archetype samples after
refresh, but the catalog is still a generated-pattern catalog, not a full
Desktop-authored catalog.

The pieChart, donutChart, pivotTable/matrix, and Basic/Dropdown slicer generators
use the Desktop-authored MIT-licensed BCApps reference shapes archived in
`wo-refs/`. Their local oracle project is `catalog-proof`; all five pages
(including the known-good line control) were refreshed and manually inspected
in Desktop Store 2.155.756.0, as recorded in
`testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json`. The four
new families are therefore `manual-desktop-canvas-refresh`; automation and
wider formatting/PBIR readback coverage are still required.

Add:

- Desktop-authored role maps for card, table, matrix, line, bar, column, combo,
  slicer, text box, KPI, gauge, map, and decomposition tree where feasible.
- Visual-type-specific validation of field wells:
  - required roles;
  - measure-only roles;
  - column/category roles;
  - max/min projection counts;
  - unsupported combinations.
- `report visuals repair-bindings --dry-run` to propose fixes when a role is
  wrong.

### P1: Style And Formatting

Existing:

- raw formatting extract/apply;
- title/alt text;
- static title/dataPoint color.

Add:

- report-level style bundle:
  - implemented first slice: `report style inspect/extract/diff/apply`;
  - implemented material: report theme collection plus per-visual PBIR
    formatting payloads, applied by visual type and ordinal without copying
    field bindings or data roles;
  - still needed from Desktop-authored fixtures: page background, filter
    pane/card styling, common title/data-label/legend/axis typed defaults, and
    style lint.
- Conditional formatting helpers:
  - implemented first readback slice: `report visuals formatting
    conditional-formatting list/show`;
  - rules;
  - gradients;
  - measure-driven colors;
  - data bars for tables/matrices.

### P1: Schema Manifest Evolution

Large real-world schema manifests (hundreds of columns/measures across many
tables) will eventually be too big for a single JSON file, and agents will
want composition.

Add:

- required `schemaVersion`;
- `$include` or directory-based manifests;
- separate files for tables, rows, pages, visuals, measures, and style;
- manifest validation with precise diagnostics;
- `schema normalize` to produce one canonical manifest for reproducibility.

### P2: Model Authoring Completeness

Existing:

- measures;
- calculated columns;
- relationships;
- partitions/source templates.

Add:

- tables/columns CRUD beyond scaffold;
- calculated tables;
- named expressions;
- date table helpers;
- roles/RLS;
- perspectives;
- translations/cultures;
- calculation groups/items;
- deeper DAX formatting and lint rules beyond the current static
  dependency/reference pass;
- optional remote DAX execution through a credential-isolated Fabric/XMLA bridge.

### P2: Agent Batch Operations

Add:

- `apply --ops ops.json`;
- durable operation plans generated by dry-run;
- `plan validate`;
- `plan replay`;
- `plan diff`;
- multi-step rollback guidance where safe.

## Regression Checklist

When Desktop rejects a report:

1. Copy the exact Desktop error details.
2. Save the failing PBIP folder under `build/` or a temporary fixture.
3. Run `validate --strict`.
4. Run Microsoft `powerbi-report-author validate` if available.
5. Identify which validator missed the issue.
6. Add a local validator rule for the missed issue.
7. Add a focused regression test.
8. Regenerate the report from CLI commands.
9. Reopen in Desktop.
10. Refresh dummy partitions.
11. Verify actual visuals render.
12. Update this document with the new finding.
