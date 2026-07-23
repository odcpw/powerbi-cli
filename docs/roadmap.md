# powerbi-cli Roadmap

`powerbi-cli` should become an agent-safe authoring workbench for Power BI
projects. The core product target is not PBIX binary generation; it is reliable
PBIP/PBIR/TMDL project authoring that can be opened in Power BI Desktop and
rebound to real data later.

## Operating Principles

- Agents are the primary users. Humans benefit from the same deterministic
  contract, but command names, JSON output, errors, handles, follow-up commands,
  dry-runs, and proof paths should be optimized for agent retry loops.
- Desktop is the compatibility oracle. A generated or edited project is not
  considered Power BI-compatible until Power BI Desktop opens it.
- The CLI must compile and run on Windows, Linux, and macOS for filesystem
  operations. Desktop proof is Windows-only.
- Every read and mutation should support JSON output for agents.
- Mutations should write explicit outputs or apply to explicit project folders;
  no hidden current-document state.
- Generated projects must not contain real data, credentials, `cache.abf`,
  `localSettings.json`, `.pbix`, or `.pbit`.
- Prefer manifest-driven and semantic commands over raw JSON/TMDL patching.
- No fake fallbacks: known but unproven Power BI features must return
  `error.code = "unsupported_feature"` and must not write guessed PBIR/TMDL.
  The live feature boundary is `powerbi-cli features list --json`.
- Do not grow a monolith. New command families should land behind focused
  modules with clear ownership: CLI dispatch/contract, schema manifests,
  project resolution and validation, PBIR report metadata, TMDL semantic model
  metadata, Desktop oracle proof, and typed mutation kernels.

## Agent Contract

The CLI should follow the `ooxml-cli` pattern: the binary is the live contract,
and agents discover capabilities before guessing.

Required contract rules:

- `--json` and `--format json` should be accepted anywhere in the invocation.
- `capabilities --json` must advertise command schemas, flags, argument types,
  stability, proof level, exit codes, diagnostic codes, examples, and generated
  follow-up command fields.
- Read commands return stable handles for every object an agent may later
  mutate: project, table, column, measure, relationship, partition, page,
  visual, filter, theme, and source template.
- Mutations require explicit output semantics: `--out-dir`, `--in-place` with a
  guard, or `--dry-run`.
- Mutation JSON includes `inspectCommand`, `validateCommand`, `readbackCommand`,
  and, when relevant, `handoffCheckCommand` or `desktopOpenCheckCommand`.
- Errors include stable diagnostic codes and suggested next commands. "See
  --help" is not enough.
- `features list --json` must advertise supported, read-only, planned, and
  explicitly refused Power BI feature surfaces, including proof level and
  refusal code.
- Current and future agent operating guidance lives in
  `skills/powerbi-cli/SKILL.md`.
- `capabilities --json` should advertise architecture guardrails so subagents
  can see that new Power BI features must not be added to `src/main.rs`.

## Command Surface

### Foundation

- `version`: report tool and contract version.
- `capabilities [--for <filter>]`: advertise the live command contract.
- `doctor`: detect Power BI Desktop on Windows, report format assumptions, and
  warn about unsupported proof levels.
- `schema validate|normalize`: validate or canonicalize a data-agnostic schema
  manifest before building a report.
- `profile infer|validate|summarize`: derive or check schema/profile metadata
  from schema manifests and embedded dummy/profile rows without connecting to
  live sources.
- `inspect <project|pbip>`: summarize PBIP, report, semantic model, pages,
  visuals, tables, columns, measures, relationships, and offline hazards.
- `validate <project|pbip>`: parse required files, validate known schemas, check
  references, detect unsafe files, and report Desktop-proof status if available.
- `package inspect|extract|import|export-plan`: inspect PBIX/PBIT archives,
  extract safe metadata/source files, import real PBIP/PBIR/TMDL source entries
  when present, and emit the Desktop export handoff. Binary export/compile/pack
  remains refused unless a future implementation can prove valid Power BI
  binary writing without opaque fallbacks.

### Project Authoring

- `scaffold --schema <json> --out-dir <dir>`: create a fresh offline-safe PBIP
  project from a schema manifest.
- `clone-template <template.pbip|dir> --schema <json> --out-dir <dir>`: preserve
  a Desktop-authored report shell while replacing or augmenting model metadata.
- `diff <before> <after>`: compare two PBIP projects at the semantic level.
- `apply --ops <json> --out-dir <dir>`: apply a generated operation plan.

### Semantic Model

- `model tables list/show/add/update/delete`
- `model columns list/show/add/update/delete`
- `model measures list/show/add/update/delete`
- `model dax dependencies|lint`
- `model relationships list/show/add/update/delete`
- `model partitions list/show/set-dummy/set-m/set-sql-template`
- `model advanced inventory`
- `model roles list/show/add/update/delete`
- `model perspectives list/show/add/update/delete`
- `model cultures list/show/add/update/delete`
- `model expressions list/show/add/update/delete`
- `model translations list/show/add/update/delete`

The first semantic milestone should focus on tables, columns, measures,
relationships, dummy partitions, static DAX reference checks, and readback of
advanced TMDL already present in a project. Mutating roles, perspectives,
cultures, expressions, translations, and refresh policies can wait until the
object-specific writers and fixtures exist.

### Report Authoring

- `report spec validate`: check a declarative dashboard spec against the schema
  and visual catalog before writing files.
- `report build --schema <schema> [--profile <profile>] [--spec <spec>]`:
  compile schema/profile/spec inputs into an offline-safe PBIP project through
  proven scaffold/report primitives.
- `report plan --schema <schema> --profile <profile> --objective <goal>`:
  deterministic starter dashboard planner that emits an explicit
  `powerbi-cli.dashboard.v1` spec plus decisions, warnings, and compile
  summary. Keep broader semantic inference shallow until additional unrelated
  archetype and Desktop goldens prove it.
- `report pages list/show/add/update/reorder/set-active/delete-empty`
- `report design-plan`
- `report layout auto`
- `report visuals list/show/formatting list/formatting show/formatting extract/formatting apply/add/clone/update/delete/set-position`
- `report visuals bind`: bind a visual to fields or measures using PBIR queries.
- `report drilldown set-hierarchy`: replace Category projections on existing
  Category/Y charts with two or more resolved model columns.
- `report visuals format`: set title, labels, legend, colors, display units,
  sort, and interactions.
- `report filters list/show/add/delete/clear` (`list/show/add/delete/clear`
  implemented first as raw readback, categorical authoring, exact-handle
  deletion, and owner-scoped clear with data-value safety warnings;
  advanced/range/TopN filters, update, and sort remain planned)
- `report slicers list/show/clear/add/update` (`list/show/clear` implemented
  first as slicer visual readback plus guarded persisted-selection clear with
  data-value safety warnings; add/update/richer state authoring remain planned)
- `report bookmarks list/show/set-display-name/reorder/delete/add/update`
  (`list/show` plus metadata-only display-name edits, flat reorder, and guarded
  delete are implemented; captured-state create/update and grouped reorder
  remain planned)
- `report interactions list/show/set/disable` (`list/show` and first guarded
  set/disable slice implemented; Default/reset semantics remain planned)
- `report themes show/extract/apply/presets/apply-preset`
- `report style inspect/extract/diff/apply`
- `report visuals formatting conditional-formatting list/show/add/update/delete`
  (`list/show` implemented as static readback; authoring remains
  Desktop-fixture gated)

Report authoring must be driven from Desktop-authored golden fixtures. PBIR
visual binding is too strict to invent by memory.

### Handoff

- `handoff write`: regenerate `POWERBI_HANDOFF.md` with rebind instructions.
- `handoff check`: verify a project is safe to take home.
- `handoff rebind-plan`: produce work-machine instructions mapping dummy
  partitions to real source templates without storing credentials.

### Proof

- `desktop open-check <project|pbip>`: Windows-only Power BI Desktop open proof.
- `desktop save-check <project|pbip> --out-dir <dir>`: prove Desktop can open and
  save the project without corrupting it.
- `desktop export-snapshot <project|pbip> --out-dir <dir>`: capture a
  Desktop-saved version for golden comparison.

These commands should be optional. CI should run them only on Windows machines
that explicitly opt in with Power BI Desktop installed.

## Development Phases

Implementation should be ordered by agent trust: contract, inspection,
validation, proof, then mutation breadth.

### Phase -1: Agent Contract Hardening

- Accept global flags such as `--json` anywhere.
- Expand `capabilities` into a machine-readable command schema.
- Add stable diagnostic codes and a documented exit-code dictionary.
- Standardize success and error output envelopes.
- Add `unsupported_feature` as a stable diagnostic and route known
  fixture-gated report features through it instead of generic unknown-command
  errors.
- Require generated follow-up commands on every mutation.
- Keep `skills/powerbi-cli/SKILL.md` aligned with the live binary.

### Phase 0: Contract And Portability

- Rename the binary to `powerbi-cli`.
- Keep dependencies pure Rust and cross-platform for non-Desktop commands.
- Add Windows/Linux/macOS CI for `cargo fmt`, `cargo check --all-targets`, and
  `cargo test --all-targets`.
- Add path-normalization tests for PBIP forward-slash references.
- Add golden/snapshot tests for current scaffold, inspect, and validate output
  before modularizing.
- Split the current monolith into CLI, output, manifest, project, TMDL, PBIR,
  scaffold, inspect, and validate modules without changing behavior.

### Phase 1: Deep Inspect, Handles, Wireframe

- Add `inspect --deep` summaries for tables, columns, measures, relationships,
  partitions, pages, visuals, bindings, filters, themes, proof state, and
  offline hazards.
- Return stable handles from every inspected object.
- Add `report wireframe export` as JSON first, HTML/SVG later if useful.
- Add `diff` over normalized `inspect --deep` summaries early; semantic diffs
  are how agents verify mutations.

### Phase 2: Validation, Lint, Handoff

- Vendor the Microsoft JSON schemas needed for PBIP, PBIR, page, visual, and
  semantic model definition files.
- Add strict schema validation with clear file/line-ish diagnostics.
- Add TMDL sanity checks for the subset we generate.
- Add M expression smoke checks for generated dummy partitions where feasible.
- Expand offline hazard checks to include common credential/cache locations.
- Add `lint` with a small built-in rule pack for broken references, empty pages,
  oversized pages, missing handles, unsafe files, unused objects, and style
  drift.
- Implemented first BPA-lite lint slice: static DAX findings, duplicate
  page/visual titles, and validator-rejected `general.altText` placements with
  explicit cleanup guidance. Generated visuals omit the rejected property.
- Add `handoff check` and `handoff rebind-plan` because this is the core
  locked-down corporate workflow.

### Phase 3: Desktop Oracle And Golden Corpus

- Implemented first oracle/golden infrastructure slice:
  `fixture normalize` emits deterministic path-free summaries,
  `fixture verify` compares projects against committed summaries with
  JSON-pointer diffs, includes path-free explicit visual interaction summaries,
  `testdata/golden/sales.summary.json` freezes the current generated sales
  fixture, and `desktop open-check` provides an opt-in Windows Power BI Desktop
  launch proof gated by `POWERBI_DESKTOP_ORACLE=1`.
- Create `testdata/desktop-golden/` with small PBIP projects saved by Power BI
  Desktop:
  - blank report with one dummy table
  - one page with a card
  - one page with a table visual
  - one page with a line chart
  - simple star schema with two relationships
  - slicer and page filter
  - theme-applied report
- For each fixture, store an expected normalized summary JSON.
- Normalize volatile fields such as timestamps, lineage tags when necessary,
  and Desktop-generated IDs.
- Implement `desktop open-check` and `desktop save-check` as opt-in Windows
  proof commands.
- Freeze expansion of new visual binding families until each family has a
  Desktop-authored fixture and normalized summary.
- Captured first manual Desktop proof for a CLI-generated project: Desktop
  opened the project, refreshed dummy `#table(...)` partitions, and rendered
  report pages with KPI cards, tables, line chart, clustered bar chart,
  fields, and filters. The findings and reproduction commands are recorded in
  `docs/pbir-desktop-oracle.md`.

### Phase 4: Semantic Model Commands

- Implemented table/column/measure/relationship/partition list and show
  commands for the current core model objects.
- Implement add/update mutations using operation outputs and readback summaries.
- Implement `model measures list/show/add/update/delete` first among mutators.
- Implement partition source-template commands next.
- Require Desktop open-check fixtures before marking mutation families stable.
- Add negative tests for bad DAX, bad M, missing columns, and ambiguous names.
- Implemented static DAX dependencies/lint over stored measures/calculated
  columns. This catches missing/ambiguous references, self references, and
  simple measure cycles, but does not claim engine validation.
- Implemented advanced semantic-model readback inventory for roles,
  perspectives, cultures, and expressions already present in TMDL. Mutation
  remains planned.
- Keep advanced semantic-model mutations and calculation groups behind the core
  model workflow.

### Phase 5: Report Layout Commands

- Implemented first slice: `report pages list/show`, `report visuals
  list/show`, and guarded `report visuals set-position`.
- Implemented second page slice: `report pages
  add/update/reorder/set-active/delete-empty` with guarded output modes,
  active-page readback, validation for `pageOrder`/`activePageName`, and
  conservative deletion of only simple empty pages.
- Implemented visual catalog and first visual creation slices:
  `report visuals catalog` exposes generated visual types, aliases, binding
  roles, template-only visual types, and planned visual families.
  `report visuals add` creates card, tableEx, lineChart, areaChart,
  stackedAreaChart, clusteredBarChart, clusteredColumnChart, barChart,
  columnChart, lineClusteredColumnComboChart, and scatterChart containers from
  the same generated PBIR pattern used by scaffold, with optional validated
  bindings and guarded output modes. The combo slice binds column measures to Y
  and line measures to Y2; one projected measure may request explicit
  descending sort. Ascending and multi-key sorts remain fixture-gated.
- Implemented visual template reuse slice: `report visuals clone` copies one
  simple visual container whose directory contains only `visual.json`, patches
  only cloned name/position/clone annotations, and preserves the visual type,
  bindings, formatting, filters, and other PBIR already inside that file. This
  lets agents duplicate known-good template visuals, including slicer-shaped
  visuals, without inventing new visual family PBIR.
- Implemented guarded visual deletion slice: `report visuals delete` removes
  only a proven `visuals/<name>/visual.json` container, refuses unknown files in
  the visual directory, does not edit page metadata, and requires exact
  `--confirm <visual-handle>` for in-place deletion.
- Implement field-bound visual creation only from golden PBIR patterns.
- Add visual snapshot summaries that agents can inspect without reading raw PBIR.

Current first slice: schema manifests can now declare visual bindings by role,
table, and column/measure. `scaffold` emits PBIR `queryState` for those
bindings and `validate`/`inspect` count bound visuals. This is enough for early
experimentation, but visual-role coverage still needs Desktop-authored golden
fixtures before it should be called stable. New visual families should remain
frozen until proven.
- Implemented second binding slice: `report visuals set-bindings` replaces or
  clears PBIR `queryState` on existing visuals with guarded output modes,
  canonical TMDL table/column/measure validation, readback, wireframe, inspect,
  validate follow-up commands, and the same visual-role/cardinality guard used
  by generated visual creation. It remains scoped to existing card/table and
  standard category/value chart visuals; slicer mutations, filters, and
  formatting remain fixture-gated.
- Implemented first formatting readback slice: `report visuals formatting
  list/show` inventories existing PBIR formatting object containers and
  property names while omitting raw literal values unless `--include-raw` is
  passed. Implemented first visual formatting portability slice:
  `report visuals formatting extract/apply` writes auditable raw PBIR formatting
  bundles and applies them to same-type target visuals by replacing only
  `/visual/objects` and `/objects`; apply refuses copied literal text unless
  `--allow-literal-text` is explicit. Typed formatting mutations remain
  fixture-gated.
- Implemented first conditional-formatting readback slice: `report visuals
  formatting conditional-formatting list/show` inventories existing PBIR
  conditional-formatting signals without authoring or inferring new rule
  shapes.

### Phase 6: Binding, Style, And Handoff

- Add source template support for CSV and generic M; SQL Server, PostgreSQL,
  ODBC, and Excel are implemented.
- Store source templates without credentials.
- Generate rebind checklists and diffs from dummy partitions to work-source
  partitions.
- Implemented first slice: `handoff check` rejects Power BI caches/binaries,
  local settings, embedded data files, real connector partitions, and
  credential-like partition source text.
- Implemented second slice: `source-template list/show/add/apply` stores
  credential-free SQL rebind metadata as sidecar JSON, and `handoff rebind-plan`
  maps dummy partitions to those templates, while `source-template apply`
  materializes one guarded live connection on the work machine without credentials.
  Home-authored TMDL remains offline-safe until that explicit work-machine step.
- Extended source templates with typed Excel workbook sheet/table sources. Applying
  an Excel template promotes headers, emits explicit Power Query conversions from
  the table's TMDL column types, and materializes an absolute workbook path.
  Existing recognized credential-free SQL, PostgreSQL, ODBC, or external-file
  sources can be retargeted only with `--replace-existing` plus the exact partition
  handle; unknown, web, credential-bearing, and unconfirmed sources remain refused.
- Implemented first theme slice: `report themes show/extract/apply` creates and
  applies raw report-level theme bundles from `themeCollection` and already
  present registered theme JSON resources. Per-visual raw formatting bundle
  extract/apply is implemented. `report themes presets` and
  `report themes apply-preset` now provide built-in registered-resource theme
  presets; additional style drift lint and richer typed formatting copy remain
  planned.
- Implemented first master style-bundle slice: `report style
  inspect/extract/diff/apply` combines report theme material and per-visual
  formatting payloads, applies them by visual type and ordinal, and refuses
  copied literal text unless `--allow-literal-text` is explicit. Field bindings
  and data roles are never copied by style apply.
- Implemented first filter slice: `report filters list/show` inventories raw
  report/page/visual PBIR filter containers, returns stable filter handles, and
  warns when filter metadata may contain selected semantic-model values.
- Implemented guarded filter add, deletion, and clear slices: `report filters
  add` writes one categorical filter to report/page/visual
  `/filterConfig/filters` after TMDL column validation; `report filters delete`
  removes one explicit report/page/visual filter by stable handle; `report
  filters clear` removes filters by exact filter handle, report scope, one page
  owner, one visual owner, or explicit `--all`. Mutations require `--dry-run`,
  `--out-dir`, or guarded `--in-place`; broader advanced/range/TopN filters,
  update/sort, and expression-level edits remain fixture-gated.
- Implemented first bookmark slice: `report bookmarks list/show` inventories
  raw PBIR bookmark files plus bookmark order/group metadata, returns stable
  bookmark handles, and warns when captured bookmark state may contain selected
  semantic-model values.
- Implemented metadata-only bookmark mutation slice: `report bookmarks
  set-display-name/reorder/delete` edits display names, flat order metadata,
  and guarded deletion. Captured bookmark state create/update and grouped
  reorder remain fixture-gated.
- Implemented first slicer slices: `report slicers list/show` inventories PBIR
  slicer visuals, returns stable slicer and visual handles, summarizes field
  bindings/state, and warns when slicer visual metadata may contain selected
  semantic-model values. `report slicers clear` removes persisted selection
  filters matching one slicer binding with guarded output modes while preserving
  bindings, layout, and formatting.
- Implemented first interaction slice: `report interactions list/show`
  inventories explicit PBIR page `visualInteractions`, resolves source/target
  visuals to stable handles, flags stale references, and documents that missing
  rows mean Power BI default interaction behavior rather than `NoFilter`.
- Implemented guarded interaction mutation slice: `report interactions
  set/disable` upserts explicit `visualInteractions` rows for live source/target
  visual pairs with `--dry-run`, `--out-dir`, or `--in-place`, refuses duplicate
  rows and stale endpoints, returns readback/wireframe/inspect/validate
  commands, and leaves `Default`/reset behavior fixture-gated.
- Implemented typed formatting mutation slices: `report visuals formatting
  set-text` patches title text/visibility and clears rejected alt-text metadata,
  while `set-color` patches static literal
  `title.fontColor` plus wildcard/static `dataPoint.fill`; data-bound color
  selectors and conditional-formatting authoring remain Desktop-fixture gated.
- Implemented first design/layout slices: `report design-plan` profiles local
  TMDL/PBIR metadata and emits agent-ready visual commands; `report layout auto`
  rewrites only visual `position` blocks; `report drilldown set-hierarchy`
  replaces existing Category projections on Category/Y charts with two or more
  resolved model columns.

### Phase 7: Filters, Slicers, Bookmarks, Interactions

- Add report/page/visual filter update/sort, advanced/range/TopN filter helpers,
  and expression-level edits on top of the implemented readback/add/delete/clear
  handles; add slicer authoring and richer slicer state edits on top of the
  implemented selection clear.
- Add slicer add/update/state/clear operations on top of the implemented
  readback handles.
- Add bookmark captured-state create/update and grouped reorder validation
  against pages and visuals on top of the implemented readback and metadata
  mutation handles.
- Add visual interaction reset/default controls and Desktop-authored
  round-trip fixtures on top of the implemented set/disable handles.
- Build on the implemented same-report `report drillthrough set/show/clear`
  linked `pageBinding` + Drillthrough filter slice with Desktop re-verification,
  then add Desktop-authored goldens for visual drillthrough action links,
  multi-field drillthrough, and cross-report drillthrough.
- Build on the implemented `report drilldown set-hierarchy` slice with
  Desktop-authored goldens for chart-family coverage and transient UI
  expand/collapse state. Keep tooltip pages, bookmark captured-state mutation,
  slicer authoring/sync, interaction reset/default semantics, non-catalog visual
  generation, and conditional-formatting authoring behind
  `unsupported_feature` until Desktop-authored goldens exist.

### Phase 8: Agent Batch Operations

- Add `diff` and `apply --ops` once individual commands are stable.
- Make operation JSON durable enough for another agent to inspect and replay.
- Include generated proof commands in mutation outputs.

### Phase 9: Optional Bridges

- Maintain the delivered bounded Windows Desktop DAX execution command and add
  further bridge commands only where they prove a distinct compatibility claim.
- Add Fabric/service commands only as an authenticated optional extension.
- Keep the core CLI cross-platform and offline-safe.

## Test Strategy

### Always-On CI

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo test --all-targets`
- Scaffold/inspect/validate smoke tests on Windows, Linux, and macOS.
- Golden summary tests for checked-in PBIP fixture folders.

### Format Tests

- JSON schema validation for every generated JSON-like file.
- TMDL text golden tests for model objects.
- M literal and table-generation unit tests for escaping, dates, datetimes,
  booleans, nulls, and identifiers with spaces or quotes.
- Relationship tests for one-to-many, inactive, and bidirectional cases.

### Desktop Oracle Tests

- Windows-only test target or script, opt-in via an environment variable such as
  `POWERBI_DESKTOP_ORACLE=1`.
- Open generated PBIP in Power BI Desktop.
- Fail on any visible "Issues were found" dialog.
- Check the title bar contains the expected report name and "Power BI Project".
- Inspect UI/accessibility for expected tables, measures, and relationships.
- Save a Desktop round-trip copy and compare normalized project summaries.

### Regression Workflow

When Desktop rejects a generated file:

1. Capture the exact error message.
2. Add a failing fixture or focused unit test for that error.
3. Patch the smallest generator/validator rule.
4. Regenerate the project.
5. Run local validation.
6. Reopen in Desktop.
7. Only then update docs or capability claims.

## Near-Term Backlog

The immediate work is no longer "can we generate something?" The generic
archetypes (`flat-ops`, `scatter-bubble`, `catalog-proof`) open, refresh, and
render in Desktop under manual oracle inspection. The next work is turning
that manual proof into repeatable guardrails and then expanding feature
coverage only from Desktop-authored or Desktop-proved fixtures.

1. **Upgrade `desktop open-check` into canvas proof.**
   Current `desktop open-check` is mostly launch/title proof. It should detect
   issue modals, blank canvases, page tabs, data pane tables, expected visual
   count, and optionally capture screenshots. It should fail if Desktop opens a
   title but the report canvas is empty.
2. **Add `desktop refresh-check`.**
   Open a PBIP, click/trigger refresh, wait for dummy partitions to load, and
   verify seeded visuals no longer show only `(Blank)` when dummy rows exist.
3. **Extend Desktop proof metadata.**
   `desktop open-check` now reports detected Power BI Desktop file version when
   available and distinguishes `desktop-launch` from future
   `desktop-canvas-refresh` compatibility proof. Canvas automation still needs
   report name, page count, screenshot paths when captured, refresh result, and
   any modal text.
4. **Commit Desktop golden fixtures.**
   `testdata/golden/sales-desktop-filter-contract.summary.json` now freezes a
   sales sample plus Desktop-proven report/page categorical filters, and
   `testdata/golden/archetypes/regional-sales.summary.json` freezes a
   drillthrough chain, TopN-by-measure filters, multi-page slicers, and
   non-ASCII column/measure round trip.
   `testdata/golden/archetypes/flat-ops.summary.json` and
   `testdata/golden/archetypes/scatter-bubble.summary.json` freeze generated
   non-domain archetypes, with manual Desktop proof records under
   `testdata/desktop-proof/`. Next add a `testdata/desktop-golden/` corpus with
   Desktop-authored normalized summaries for card, tableEx, line chart,
   clustered bar/column, slicer, page filter, report filter, theme, and
   conditional-formatting examples. Include the Desktop version used to create
   each fixture.
5. **Decide Microsoft validator integration.**
   Choose one of: vendor Microsoft JSON schemas, optional shell-out to
   `@microsoft/powerbi-report-authoring-cli`, or a separate
   `validate --microsoft` proof. Keep local rules for Desktop-only constraints
   the Microsoft validator misses, such as 1-50 char filter names.
6. **Add structured PBIR diagnostic codes.**
   Validation errors should carry codes and JSON-pointer-ish paths, not only
   strings. Agents need stable repair targets.
7. **Expand filter authoring from fixtures.**
   Add `report filters update`, advanced filters, range filters, TopN,
   relative date/time, include/exclude, pane visibility, and sort/order
   metadata. Every new filter type needs a Desktop-authored fixture first.
8. **Harden the visual catalog with Desktop-authored role maps.**
   Current generated patterns render for the proof report. Next, build a
   fixture-backed catalog for card, table, matrix, line, bar, column, combo,
   slicer, text box, KPI, gauge, map, and decomposition tree where feasible.
   Validate required roles, measure-only roles, max/min projections, and
   unsupported combinations.
9. **Harden style bundles.**
   `report style inspect/extract/diff/apply` now combines theme material and
   per-visual formatting and applies it without copying bindings/data roles.
   Next, add fixture-backed page background, filter pane/card styling,
   title/data-label/legend/axis typed defaults, style lint, and conditional
   formatting once fixtures exist.
10. **Version and compose schema manifests.**
    Add required `schemaVersion`, `$include` or directory-based manifests,
    `schema validate`, and `schema normalize`. Large real-world schema
    manifests will eventually be too big for a single JSON file, and agents
    will want composition.
11. **Broaden semantic model authoring.**
    Add tables/columns CRUD beyond scaffold, calculated tables, named
    expressions, date-table helpers, roles/RLS, perspectives, translations,
    calculation groups/items, and broader DAX format/lint. Bounded Desktop DAX
    execution is delivered; authenticated Fabric/XMLA execution remains optional.
12. **Add durable batch operations.**
    Add `apply --ops`, `plan validate`, `plan replay`, and `plan diff` so one
    agent can produce a mutation plan and another can inspect or replay it.
