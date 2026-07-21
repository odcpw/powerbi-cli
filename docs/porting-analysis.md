# Clean-Room Porting Analysis

Date: 2026-06-22

This document turns the external Power BI tooling research into concrete
implementation decisions for `powerbi-cli`. The target remains a cross-platform,
single-binary Rust CLI for deterministic offline PBIP/PBIR/TMDL authoring, with
Power BI Desktop used as the compatibility oracle.

## Current Baseline

`powerbi-cli` can already:

- create an offline-safe PBIP project from a JSON manifest;
- emit TMDL tables, columns, measures, relationships, and dummy M partitions;
- emit PBIR report/page/visual files with early visual bindings;
- inspect and validate generated projects;
- report agent-friendly capabilities and doctor diagnostics;
- build and test on Windows, Linux, and macOS.

The code is currently concentrated in `src/main.rs`. The next serious milestone
should split this into modules before feature growth makes the command surface
hard to maintain.

## Source Buckets

See `docs/clean-room-research.md` for pinned refs and license notes. The short
version:

- MIT/permissive references: `MinaSaad1/pbi-cli`, `akhilannan/pbir-utils`,
  `microsoft/powerbi-modeling-mcp`, `microsoft/semantic-link-labs`,
  `TabularEditor/TabularEditor`.
- Reference-only or quarantine: `DaxStudio/DaxStudio` because of reciprocal
  license risk; `pbi-tools/pbi-tools`, `maxanatsko/pbir.tools`, and
  `data-goblin/power-bi-agentic-development` because of AGPL/GPL/custom
  restrictions.

For quarantined projects, use only high-level behavior signals. Do not copy code,
JSON examples, templates, docs prose, prompts, or low-level structures.

## Product Position

The useful niche is not "another Power BI Desktop automation wrapper." The
defensible niche is:

- schema-first generation for locked-down corporate environments;
- dummy-data/offline-safe handoff with no credentials, caches, or PBIX payloads;
- deterministic builds agents can reproduce and diff;
- PBIP/PBIR/TMDL source control friendliness;
- Desktop round-trip proof before claiming compatibility;
- JSON-first output that another agent can inspect and replay.

The CLI should feel compiler-like: manifest in, project out, validate, inspect,
mutate through typed commands, and prove with Desktop when available.

## Agent-First Review Corrections

After independent Claude and Grok reviews on 2026-06-22, the plan is revised
around one sharper rule: agent contract and proof infrastructure come before
feature breadth.

Accepted corrections:

- `capabilities` must become a full machine-readable contract: command schemas,
  flags, argument types, stability, proof level, exit codes, diagnostic codes,
  examples, and generated follow-up commands.
- Global flags such as `--json` must be accepted anywhere in the invocation, not
  only before the subcommand.
- Every object that agents touch needs a stable handle returned by list/show or
  `inspect --deep` and accepted by mutators. Agents should not infer PBIR paths.
- Every mutation must support explicit output semantics and, when practical,
  `--dry-run`.
- Every mutation response must include generated next commands such as
  `inspectCommand`, `validateCommand`, `readbackCommand`,
  `handoffCheckCommand`, or `desktopOpenCheckCommand`.
- `inspect --deep`, `lint`, `report wireframe export`, `handoff check`, and
  `handoff rebind-plan` are earlier than most advanced model/report features.
- Desktop oracle proof is early infrastructure. Semantic-model text mutations
  can ship as beta with strict offline validation; visual binding and formatting
  should not expand beyond proven fixture families.
- The repo-local agent guide at `skills/powerbi-cli/SKILL.md` is a product
  artifact, not an afterthought.

Follow-up implementation review on 2026-06-22 confirmed the next build order:
finish modularizing the current scaffold/inspect/validate code, then add
`inspect --deep` with stable handles, `report wireframe export --format json`,
strict `validate`/`lint` diagnostics, normalized semantic/report diff, and only
then the first mutation kernel for `model measures` with dry-run/readback.

## Feature Classification

| Area | Comparator signal | Decision | Rationale |
|---|---|---|---|
| Agent-safe CLI contract | `pbi-cli`, `pbir-utils` | Port concept, rewrite in Rust | JSON output, explicit paths, dry-run summaries, and capability discovery are core to agent usability. |
| PBIP path discovery | `pbi-cli`, `pbir-utils` | Port concept, rewrite | Need robust resolution from `.pbip`, report folder, model folder, or project root. |
| Schema-to-project scaffold | Current project | Keep and expand | This is our strongest differentiator. |
| TMDL model generation | Current project, Microsoft Modeling MCP, Tabular Editor | Reimplement from Microsoft format and Desktop fixtures | Measures, calculated columns, relationships, roles, perspectives, calculation groups, cultures, and named expressions belong here. |
| DAX measures | All semantic-model tools | Build early | Measures are mandatory for real dashboards. Start with create/list/show/update/delete and static checks. |
| Calculated columns/tables | Semantic-model tools | Build after measures | Similar TMDL path, but higher risk because calculated tables affect partitions and model semantics. |
| Relationships | Current project, MCP, Tabular Editor | Build early | Star-schema authoring needs this. Validate table/column references aggressively. |
| Partitions and source templates | Current project, pbi-tools behavior signal | Build early for dummy and handoff | Credential-free SQL Server, PostgreSQL, ODBC, and Excel source templates plus `handoff rebind-plan` are implemented. Apply can replace generated dummy partitions and can retarget recognized credential-free existing sources only with exact-handle confirmation. CSV and generic M remain planned. |
| Roles/RLS | MCP, Tabular Editor | Readback implemented; mutations later | `model roles list/show` and `model advanced inventory` can inventory existing TMDL role metadata. Authoring RLS expressions needs object-specific mutation rules and engine/Desktop validation. |
| Perspectives/translations/cultures | MCP, Tabular Editor, Semantic Link Labs | Readback implemented; mutations later | `model perspectives|cultures|expressions list/show` cover existing enterprise metadata. Mutations are lower priority than first dashboard generation and need fixtures. |
| Calculation groups/items | MCP, Tabular Editor | Reimplement later with Desktop goldens | Powerful but easy to corrupt without oracle fixtures. |
| DAX static analysis and query execution | MCP, DAX Studio, `pbi-cli` | Static dependency/lint plus bounded Desktop execution implemented; remote Fabric/XMLA bridge deferred | Offline project authoring still cannot prove engine semantics. `model dax dependencies/lint` catches local reference problems and simple cycles; `model dax execute` requires two opt-ins and an exact already-open Desktop PBIP, then runs row/cell/time-bounded EVALUATE queries without model writes or query echo. |
| Report page CRUD | `pbi-cli`, `pbir-utils` | First page mutation slice implemented | `report pages add/update/reorder/set-active/delete-empty` covers low-risk page metadata and order. Visibility/background remain planned. |
| Visual CRUD and layout | `pbi-cli`, `pbir-utils` | First visual catalog/add/clone/delete/layout slices implemented; continue with fixtures | `report design-plan` profiles local TMDL/PBIR metadata and returns agent-ready report commands. `report layout auto` rewrites visual `position` blocks into deterministic page slots. `report visuals catalog` exposes generated visual types and role contracts. `report visuals add` creates guarded generated-pattern card/tableEx/line/area/stacked area/clustered bar/clustered column/stacked bar/stacked column/scatter containers. `report visuals clone` duplicates simple template visuals by copying only `visual.json` and patching name/position/clone annotations, preserving existing PBIR without expanding arbitrary visual-family generation. `report visuals delete` removes only simple visual containers with no unknown sidecars. Update and set-container remain planned. |
| Visual bindings | Current project, `pbi-cli` | First replacement and hierarchy slices implemented; expand from Desktop goldens | `report visuals add` and `set-bindings` can write `queryState` for supported card/table, category/value chart, and scatter/bubble visuals with TMDL name validation. `report drilldown set-hierarchy` replaces Category projections on existing line, area, bar, column, and combo charts with two or more resolved model columns, marks the first field active as the initial level, and enables the visual-header drill controls. Scatter hierarchy is refused because the official report validator caps its Category role at one projection. Do not expand advanced PBIR binding shapes without Desktop-authored fixtures. |
| Visual catalog | `pbi-cli` visual inventory | Rebuild from our Desktop fixtures | Initial catalog: card, table, matrix, clustered bar/column, line, combo, slicer, text box. Expand only after round-trip proof. |
| Formatting | `pbi-cli`, `pbir-utils` | First readback, raw bundle portability, and typed text/static-color slices implemented | `report visuals formatting list/show` inventories existing PBIR formatting object containers and property names with raw opt-in. `report visuals formatting extract/apply` copies raw per-visual PBIR formatting bundles between same-type visuals while replacing only `/visual/objects` and `/objects`. `set-text` patches title/alt text; `set-color` patches static literal `title.fontColor` and wildcard/static `dataPoint.fill`. Conditional-formatting readback is implemented separately. Data labels, legend, axes, display units, sort, number formats, selector-specific colors, and conditional-formatting authoring remain Desktop-fixture gated. |
| Conditional formatting | `pbi-cli`, agent workflow signals | Readback implemented; authoring from fixtures | `report visuals formatting conditional-formatting list/show` inventories existing PBIR signals. Measure-based colors, gradients, rules, and data bars need Desktop-authored fixtures before authoring. |
| Themes/style extraction | `pbir-utils`, user requirement | Build as first-class | Raw `report themes show/extract/apply` and presets are implemented. `report style inspect/extract/diff/apply` now packages report theme material plus per-visual formatting and reapplies it by visual type/ordinal without copying bindings or data roles. Style drift lint and richer typed formatting remain planned. |
| Filters and slicers | `pbi-cli`, `pbir-utils` | First filter add/delete/clear plus slicer readback/clear slices implemented | `report filters list/show/add/delete/clear` inventories raw report/page/visual PBIR filters, adds one categorical filter after TMDL column validation, deletes one explicit filter by stable handle, and clears filters by exact owner or explicit `--all` with guarded output modes. Validation recognizes Desktop-authored predicate-free field-well placeholders for both column and measure metadata. `report slicers list/show/clear` inventories PBIR slicer visuals with slicer/visual handles, binding/state summaries, data-value safety warnings, and guarded persisted-selection clear for one slicer visual. Filter update/sort, slicer add/update/richer state mutation, and advanced/range/TopN/relative-date helpers remain planned. |
| Bookmarks | `pbi-cli`, `pbir-utils` | Readback plus metadata mutation implemented | `report bookmarks list/show` inventories raw bookmark files plus order/group metadata with data-value safety warnings. `set-display-name/reorder/delete` covers metadata-only edits; captured-state create/update and group reorder remain planned because bookmarks interact with visibility, slicers, filters, and pages. |
| Visual interactions | `pbir-utils` | First interaction readback and guarded mutation slices implemented | `report interactions list/show` inventories explicit page `visualInteractions`, resolves source/target handles, and flags stale references. `report interactions set/disable` upserts DataFilter, HighlightFilter, or NoFilter rows for live source/target visual pairs; Default/reset semantics and broader Desktop-authored interaction fixtures remain planned. |
| Metadata extraction | `pbir-utils`, Semantic Link Labs | Build early | `inspect --deep` exposes core pages, visuals, fields, measures, relationships, partitions, themes, hazards, and explicit visual interactions. `fixture normalize` now turns that into path-free golden summaries. |
| Lint/rules | `pbir-utils`, Tabular Editor BPA | Build early | First BPA-lite slice implemented: DAX static findings, duplicate page/visual titles, and missing visual alt text. Broader rule packs should catch overfull pages, hidden unused objects, broken refs, unsafe files, and style drift. |
| Sanitize/fixups | `pbir-utils` | Build after lint | Dry-run first. Fix unused bookmarks/custom visuals/measures, empty pages, filter pane settings, display options. |
| Wireframe preview | `pbir-utils` | Build early as own implementation | HTML/SVG/JSON layout preview helps agents reason without Desktop. |
| Desktop oracle harness | Current project, Power BI Desktop | First launch/golden slice implemented | `desktop open-check` is opt-in and returns structured unavailable output in normal CI. `fixture normalize/verify` provides deterministic summaries and the first checked-in generated sales golden. Real Desktop-authored fixture corpus remains planned. |
| Diff/apply operation plans | Existing roadmap, pbi-tools signal | Build after typed commands | Needed for multi-agent review and replay. |
| PBIX/PBIT compile/extract | pbi-tools signal | Metadata doors implemented; binary writing deferred/refused | `package inspect/extract/import/export-plan` gives agents safe source-control doors for archives that contain PBIP/PBIR/TMDL metadata. Export/compile/pack is still refused because opaque binary writing would conflict with offline PBIP-first trust. |
| Fabric deploy/download | pbi-tools, Semantic Link Labs | Defer to separate authenticated plugin/bridge | Not needed for home/offline authoring and introduces credentials/governance concerns. |
| Live Desktop model mutation | `pbi-cli`, MCP | Optional bridge, not core | Windows-only and dependency-heavy. Core CLI must remain cross-platform. |
| Custom visuals | `pbi-cli` | Defer | Useful, but package/import semantics and trust surface are broader than native visuals. |

## Proposed Command Taxonomy

The CLI should grow into these command families.

### Foundation

- `version`
- `capabilities [--for <area>]`
- `doctor`
- `inspect <project|pbip> [--deep] [--json]`
- `validate <project|pbip> [--strict] [--desktop-oracle]`
- `lint <project|pbip> [--rules <file>]`
- `sanitize <project|pbip> [--dry-run] [--apply]`
- `diff <before> <after>`
- `apply --ops <json> --project <dir>`

### Project

- `scaffold --schema <json> --out-dir <dir>`
- `from-template --template <pbip|dir> --schema <json> --out-dir <dir>`
- `report themes extract --project <pbip|dir> --out <theme-bundle.json>`
- `report themes apply --bundle <theme-bundle.json> --project <dir>`
- `report themes presets`
- `report themes apply-preset --project <dir> --preset <preset-id>`
- `report visuals formatting extract --project <pbip|dir> --handle <visual> --out <formatting-bundle.json>`
- `report visuals formatting apply --project <dir> --handle <visual> --bundle <formatting-bundle.json>`
- `handoff write/check/rebind-plan`
- `source-template add/list/show/remove`

### Semantic Model

- `model tables list/show/add/update/delete`
- `model columns list/show/add/update/delete`
- `model calculated-columns list/show/add/update/delete`
- `model calculated-tables list/show/add/update/delete`
- `model measures list/show/add/update/delete/move`
- `model relationships list/show/add/update/delete`
- `model partitions list/show/set-dummy/set-m/set-sql-template`
- `model roles list/show/add/update/delete`
- `model perspectives list/show/add/update/delete`
- `model calculation-groups list/show/add/update/delete`
- `model calculation-items list/show/add/update/delete`
- `model cultures list/show/add/update/delete`
- `model translations list/show/add/update/delete`
- `model expressions list/show/add/update/delete`
- `model dependencies measures/objects`

### DAX And M

- `dax format`
- `dax lint`
- `dax references`
- `dax validate --desktop-bridge`
- `dax query --desktop-bridge`
- `m lint`
- `m source-summary`

Static commands must work offline. Execution commands require an explicit bridge
to Power BI Desktop or Fabric and should not be part of the default core proof.

### Report

- `report pages list/show/add/update/reorder/set-active/delete-empty/set-size`
- `report design-plan`
- `report layout auto`
- `report visuals list/show/add/clone/update/delete/set-position/set-container`
- `report visuals set-bindings`
- `report drilldown set-hierarchy`
- `report visuals bind`
- `report visuals where`
- `report visuals bulk-update`
- `report visuals calc list/add/delete`
- `report filters list/show/add/delete/clear` (implemented) and update/sort
- `report slicers list/show/clear` (implemented) and add/update/richer state
- `report bookmarks list/show` (implemented) and add/update/delete/reorder/group
- `report interactions list/show/set/disable` (implemented) and reset/default
- `report themes show/extract/apply/presets/apply-preset`
- `report format get/set/clear`
- `report conditional-format add/update/delete`
- `report annotations list/add/update/delete`
- `report wireframe export`

### Desktop Oracle

- `desktop open-check`
- `desktop save-check`
- `desktop export-snapshot`
- `desktop capture-golden`
- `desktop reload`

These are Windows-only optional commands. CI should run them only when
`POWERBI_DESKTOP_ORACLE=1` and Desktop is installed.

## Development Sequence

### Phase 0: Modularize

- Split `src/main.rs` into `cli`, `output`, `project`, `manifest`, `tmdl`,
  `pbir`, `validate`, `inspect`, and `fs` modules.
- Preserve current command behavior and JSON contracts.
- Add snapshot tests for existing scaffold output before moving code.

### Phase 1: Deep Inspect And Validation

- Add structured summaries for model objects, pages, visuals, filters, fields,
  bindings, measures, relationships, and hazards.
- Add validator categories: file structure, JSON parse/schema, TMDL sanity,
  PBIR references, visual bindings, offline safety, and style/theme consistency.
- Add `--strict` and machine-readable diagnostic codes.

### Phase 2: Golden Fixture Harness

- Create Desktop-authored PBIP fixtures for one visual or feature at a time.
- Normalize volatile IDs/timestamps/lineage fields.
- Store expected `inspect --deep --json` summaries.
- Require each new generator/mutator to pass scaffold, validate, inspect, and
  Desktop open-check where available.

### Phase 3: Semantic Model Commands

- Implement list/show/add/update/delete for tables, columns, measures, and
  relationships.
- Add calculated columns after base columns and measures are stable.
- Add partition source templates and handoff rebind plans.
- First source-template/rebind slice is implemented as sidecar metadata plus
  `handoff rebind-plan`; keep executable partition mutation behind further
  tests and Desktop proof.
- Add dependencies for measures and object references.

### Phase 4: Report CRUD

- Implement page CRUD, order, active page, size, visibility, and background.
- Implement visual list/show/add/clone/delete/set-position/set-container.
- Implemented visual catalog and first visual creation slices:
  `report visuals catalog` exposes supported generated visual types, aliases,
  roles, template-only types, and planned families. `report visuals add` creates
  guarded generated-pattern card/tableEx/line/area/stacked area/clustered bar/
  clustered column/stacked bar/stacked column/scatter containers with optional
  validated bindings.
- Implemented first visual clone slice: `report visuals clone` duplicates simple
  template visual containers containing only `visual.json`, patches only
  name/position/clone annotations, and preserves existing visual type, bindings,
  formatting, filters, and raw PBIR in the copied file.
- Implemented guarded visual deletion slice: `report visuals delete` removes
  only proven `visuals/<name>/visual.json` containers, refuses unknown files, and
  leaves page metadata untouched.
- Implemented first mutation slice for existing visual bindings:
  `report visuals set-bindings` replaces or clears `queryState` with guarded
  output modes and TMDL table/column/measure validation.
- Implement wireframe export so agents can inspect layout without Desktop.

### Phase 5: Visual Binding Catalog

- Build a manifest-driven catalog from our own Desktop fixtures.
- Start with card, table, matrix, bar/column, line, combo, slicer, and text box.
- For each visual type, define roles, required/optional fields, supported
  measures, sort defaults, and validation errors.

### Phase 6: Formatting, Themes, And Conditional Formatting

- Add typed formatting ops for titles, axes, legends, labels, colors, number
  formats, backgrounds, and layout chrome.
- Extend the first raw theme bundle and visual formatting inventory slices into
  style drift lint and typed per-visual formatting only after Desktop-authored
  fixtures.
- Add conditional-format operations only after Desktop fixture proof.

### Phase 7: Filters, Slicers, Bookmarks, Interactions

- Add report/page/visual filter CRUD.
- Add slicer add/update/state/clear operations on top of the implemented
  readback handles.
- Add bookmark CRUD, reorder/group edits, and validation against pages/visuals
  on top of the implemented readback handles.
- Add interaction set/disable controls for dense reports on top of the
  implemented readback handles.

### Phase 8: Batch Ops And Agent Review

- Add durable operation-plan JSON.
- Add dry-run summaries and patch previews.
- Add semantic diffs that hide volatile PBIR noise.

### Phase 9: Optional Bridges

- Extend the delivered bounded Windows Desktop DAX query bridge only where live
  proof warrants it; keep canvas/reload proof on the official Desktop IPC path.
- Add a Fabric/service bridge only as an authenticated optional extension.
- Keep core generation, inspection, validation, and mutation cross-platform.

## Testing Strategy

Always-on tests:

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo test --all-targets`
- scaffold/inspect/validate smoke tests on Windows, Linux, and macOS;
- golden summary tests for checked-in PBIP fixtures;
- fixture tests for TMDL escaping, M literals, path normalization, and safe file
  exclusions.

Oracle tests:

- opt-in Windows Desktop open-check;
- save-check and normalized diff;
- failure capture with the Desktop error text;
- one fixture per visual/format/filter/bookmark behavior.

Clean-room tests:

- no vendored external repos in source or testdata;
- no quarantined JSON fixtures or docs snippets;
- generated fixtures must come from our own Desktop round trips;
- every feature inspired by a quarantined tool must be expressed as a behavior
  test, not copied implementation.

## Immediate Backlog

1. Harden the agent contract: accept `--json` anywhere, enrich
   `capabilities`, define diagnostic codes, standardize output envelopes, and
   require generated follow-up commands.
2. Add snapshot/golden tests for current scaffold/inspect/validate output, then
   modularize the current Rust file without changing behavior.
3. Add `inspect --deep --json` with stable handles for pages, visuals, fields,
   model objects, bindings, proof state, and offline hazards.
4. Add `report wireframe export` so agents can reason about layout without
   Desktop.
5. Add `lint`, `validate --strict`, and `handoff check` with small built-in rule
   packs and machine-readable diagnostics.
6. Add Desktop oracle infrastructure: `desktop open-check`, `desktop save-check`,
   normalized round-trip summaries, and fixture capture scripts. First slice is
   implemented for `desktop open-check` plus `fixture normalize/verify`; save
   checks, Desktop-authored fixtures, and capture scripts remain planned.
7. Freeze new visual binding families until Desktop-authored golden fixtures
   exist for each family.
8. Implement `model measures list/show/add/update/delete` with `--dry-run`,
   explicit output semantics, readback commands, and metamorphic tests.
9. Implement relationships and partition/source-template commands for the
   work-machine rebind workflow.
10. Implement `report pages` and `report visuals` CRUD only after stable handles
    and wireframe summaries exist.

## Bottom Line

We should not chase every Power BI automation surface at once. The optimal path
is to become excellent at agent-discoverable offline source generation,
inspection, proof, and a small set of reliable mutations first, then add
optional live bridges where they prove compatibility or unlock DAX execution.
The comprehensive tool is an agent-first PBIP compiler, inspector, linter,
mutator, handoff checker, and Desktop-oracle harness before it is a service
deployment tool.
