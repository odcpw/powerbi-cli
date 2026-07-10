# powerbi-cli Goal

Date: 2026-06-23

## Objective

Build `powerbi-cli` into an agent-first Power BI dashboard workbench that can
author **any dashboard from any data shape an agent can describe safely**.

The product is not a single-domain dashboard generator. Any one dashboard is only
one regression fixture. The real goal is a general-purpose CLI that lets an
agent take schema metadata, dummy/profile data, a report intent, and optional
style/template material, then produce an offline-safe Power BI Project that can
later be opened at work, rebound to live data, refreshed, and trusted.

The target workflow is:

```text
bring schema, metadata, and dummy/profile rows home
-> let agents inspect, profile, model, and design a dashboard
-> build PBIP/PBIR/TMDL metadata without credentials or real data
-> validate, lint, inspect, fixture-freeze, and Desktop-proof the project
-> bring the project to the work computer
-> replace dummy/source-template partitions with live corporate sources
-> refresh in Power BI Desktop
```

The CLI must make this possible across domains: safety, finance, sales,
operations, logistics, HR, support, projects, compliance, surveys, inventory,
manufacturing, and whatever else a schema can express.

## Current Proof Status

As of 2026-06-25, automated acceptance includes
`tests/desktop_acceptance_everything.rs`. It builds one offline-safe acceptance
project and invokes every command advertised by the live capabilities catalog,
covering schema/profile/spec, package, semantic-model, report, validation,
fixture, diff, and handoff surfaces. The resulting project contains 6 tables,
13 measures, 4 relationships, 4 pages, and 16 bound visuals.

That generated project also has manual Power BI Desktop Store `2.155.756.0`
canvas/refresh evidence in `docs/desktop-acceptance-everything.md`: all four
pages refreshed and rendered without remaining load, custom-visual,
incomplete-data, or refresh errors. The earlier flat-operations and
scatter/bubble archetypes retain their separate manual proof records under
`testdata/desktop-proof/`.

The everything-acceptance test itself records Desktop proof as pending manual
computer use. Normal CI therefore proves command-catalog coverage and local
project invariants, not automated Desktop rendering. The live `desktop
open-check` command remains launch-level proof and must continue to say so until
canvas/refresh automation is implemented.

## 2026-06-23 Parity Tranche Status

The current implementation pass moved the first nine parity points from plan to
tested command surface without changing the product thesis:

1. **PBIX/PBIT/PBIR doors**: `package inspect/extract/import/export-plan`
   handles archive metadata/source extraction and Desktop handoff. It refuses
   binary export/compile/pack rather than pretending to write opaque Power BI
   binaries.
2. **DAX support**: measures and calculated columns remain writable TMDL
   objects, and `model dax dependencies/lint` now adds static reference and
   simple cycle analysis. It explicitly reports that DAX engine validation is
   not performed offline.
3. **Advanced semantic model readback**: `model advanced inventory` and
   `model roles|perspectives|cultures|expressions list/show` inventory existing
   TMDL enterprise metadata. Mutations remain feature-gated until dedicated
   writers and fixtures exist.
4. **PBIR visual surface**: generated visuals keep using proven PBIR patterns
   and now emit default alt text so new reports satisfy the first accessibility
   lint rule.
5. **Conditional formatting**: `report visuals formatting
   conditional-formatting list/show` inventories existing PBIR conditional
   formatting signals. Authoring remains Desktop-fixture gated.
6. **Bookmarks**: bookmark readback is joined by metadata-only
   `set-display-name`, flat `reorder`, and guarded `delete`. Captured state
   creation/update is still unsupported.
7. **Desktop oracle honesty**: the launch-level oracle and cleanup behavior are
   kept honest. The CLI still does not claim automated canvas/render/refresh
   proof.
8. **Style/master format**: `report style inspect/extract/diff/apply` provides
   a schema-independent master-format workflow over report theme material and
   per-visual formatting without copying bindings or data roles.
9. **Lint/BPA foundation**: top-level `lint` now includes static DAX findings,
   duplicate title checks, and missing visual alt-text checks.

This tranche is not a license to bypass the no-fallback rule. Every command
above either writes a deliberately narrow, tested metadata mutation or returns
an explicit unsupported-feature diagnostic.

## Product Thesis

Agents should be able to author dashboards the way they author documents with
`ooxml-cli`: by discovering the live command contract, making deterministic
semantic edits, inspecting stable handles, validating the output, and receiving
exact next commands after every mutation.

For Power BI, that means `powerbi-cli` should become a compiler/workbench over
five things:

- **Data shape**: tables, columns, types, relationships, grain, sample/profile
  statistics, and source-template placeholders.
- **Semantic model**: measures, calculated columns, relationships, date tables,
  roles, perspectives, translations, calculation groups, and partitions.
- **Report intent**: pages, visual archetypes, KPIs, drill paths, filters,
  slicers, interactions, bookmarks, tooltips, and narrative structure.
- **Visual design system**: themes, style bundles, formatting, conditional
  formatting, layout grids, template reuse, accessibility text, and brand rules.
- **Proof and handoff**: validation, Desktop oracle checks, golden summaries,
  rebind plans, and offline-safety guarantees.

## Non-Negotiables

- The CLI is for agents first and humans second.
- The tool must be data-domain agnostic. No command, manifest field, or visual
  heuristic may assume safety data, sales demos, or any other specific domain.
- Supported features must emit real PBIP/PBIR/TMDL metadata.
- Unsupported or unproven Power BI features must fail with stable
  `unsupported_feature` diagnostics, not guessed fallback JSON.
- Power BI Desktop is the compatibility oracle.
- Desktop proof must distinguish launch/title proof from canvas, refresh, and
  visual-render proof.
- The core CLI must compile and run on Windows, Linux, and macOS.
- Desktop automation may be Windows-only and opt-in.
- Home/offline projects must not contain credentials, real data, `.pbix`,
  `.pbit`, `.pbi/cache.abf`, `localSettings.json`, or unsafe connector text.
- Do not create a monolith. New features must land in focused modules with
  tests: CLI contract, schema/profile manifests, project resolution, PBIR, TMDL,
  validation, Desktop oracle, fixtures, report design, style, and mutation
  kernels.
- Clean up after Desktop oracle runs. Do not leave Power BI Desktop windows or
  background processes hanging after automated checks.

## Core Abstractions

### Schema Manifest

The schema manifest describes what exists:

- tables;
- columns;
- data types;
- keys;
- relationships;
- grains;
- source-template placeholders;
- optional dummy rows;
- optional known measures;
- optional privacy/offline-safety metadata.

This is enough to scaffold a model, but not enough to design a good dashboard.

### Data Profile

The data profile describes what the data is like without requiring real data:

- row counts;
- null rates;
- distinct counts;
- min/max values;
- rough distributions;
- categorical top values;
- time coverage;
- expected refresh cadence;
- representative dummy rows;
- safe anonymized examples.

Profiles let agents choose sensible charts, aggregations, bins, sort orders,
drill paths, and filters without seeing sensitive records.

### Report Intent

The report intent describes what the dashboard should help someone understand
or decide:

- audience;
- business questions;
- KPIs;
- comparison groups;
- time periods;
- target metrics;
- required drilldowns/drillthroughs;
- filter dimensions;
- alert/conditional-formatting rules;
- preferred visual archetypes;
- pages and narrative flow;
- handoff requirements.

Intent should be machine-readable enough for `report build --spec`, but friendly
enough that another agent can write or revise it.

### Dashboard Spec

The dashboard spec is the compiled, explicit report plan:

- pages;
- visuals;
- visual bindings;
- measures/calculated columns to create;
- filters/slicers;
- drilldown and drillthrough paths;
- interactions;
- layout coordinates or layout intent;
- style bundle references;
- proof requirements.

It should be deterministic and replayable.

### Style Bundle

The style bundle captures reusable report design:

- theme;
- page backgrounds;
- visual formatting;
- titles;
- color tokens;
- typography;
- axes, legends, labels, and number formats;
- conditional formatting rules once fixture-proven;
- accessibility defaults;
- brand/template provenance.

Agents should be able to extract style from a known-good report and apply it to
an unrelated report whose schema is different.

## Skill Sequence To Run

This sequence is the reference workflow for planning the next serious pass. It
should be run before broadening feature coverage or doing a large refactor.

### 1. Multi-Angle Review

Use `modes-of-reasoning-project-analysis`.

Purpose: inspect the whole project from deliberately different perspectives so
the design does not collapse into "we can generate one fixture."

Recommended modes for this repo:

- Systems-Thinking: does the architecture support arbitrary-dashboard authoring?
- Failure-Mode: where can generated reports silently fail in Desktop or during
  work-machine rebind?
- Edge-Case: where do handles, paths, quoting, DAX, M, PBIR shapes, data types,
  and unknown schemas break?
- Adversarial-Review: where could unsafe data, credentials, or false proof
  claims slip through?
- Perspective-Taking: what does a future agent need when it has only the repo
  and a new schema?
- Counterfactual: which current decisions would block generic dashboards later?
- Deductive: are command contracts, feature statuses, and proof claims
  internally consistent?
- Inductive: what conventions across implemented commands should become
  mandatory framework rules?
- Option-Generation: what small primitives unlock many dashboard types?
- Debiasing: where are we overfitting to current examples or Desktop launch
  smoke tests?

Expected artifact:

```text
MODES_OF_REASONING_REPORT_AND_ANALYSIS_OF_PROJECT.md
```

The report must separate:

- kernel findings with independent evidence;
- supported findings;
- disputed findings;
- unique insights;
- concrete next implementation actions.

### 2. Idea Winnowing

Use `idea-wizard`.

Purpose: convert the multi-angle findings into buildable product slices.

Run the 30-to-5 idea pass, then expand to the next 10. The best ideas should be
judged by whether they make arbitrary-dashboard generation more reliable,
ergonomic, inspectable, and useful for agents.

Expected output:

- top 5 implementation slices;
- next 10 later slices;
- dependency structure;
- explicit test and proof requirements per slice.

### 3. Agent Ergonomics Review

Use `agent-ergonomics-and-intuitiveness-maximization-for-cli-tools`.

Purpose: make the first command an agent naturally tries either work or return a
useful, exact next command.

Apply this to:

- `capabilities`;
- `features list`;
- `doctor`;
- `schema validate`;
- `profile infer`;
- `report plan`;
- `report build`;
- `report design-plan`;
- `inspect --deep`;
- `validate --strict`;
- `handoff check`;
- Desktop proof commands;
- every mutation output envelope.

Expected artifact:

```text
agent_ergonomics_audit/
```

The audit should produce concrete patches, not only a scorecard.

### 4. Golden Artifact Strategy

Use `testing-golden-artifacts`.

Purpose: freeze known-good PBIP/PBIR/TMDL outputs so agents can refactor and
extend the tool without accidentally changing Desktop-sensitive metadata.

Goldens must be **archetype-based**, not domain-based.

Required golden families:

- tiny one-table dashboard;
- star schema dashboard;
- wide operational table dashboard;
- time-series dashboard;
- KPI overview dashboard;
- table/matrix-heavy dashboard;
- scatter/bubble dashboard;
- categorical distribution dashboard;
- drilldown dashboard;
- drillthrough dashboard;
- slicer/filter-heavy dashboard;
- style/template application dashboard;
- conditional-formatting dashboard once fixture-proven;
- Desktop-saved round-trip fixture once save automation exists.

Domain examples may include sales, finance, operations, logistics, safety, and
support. No single domain should define the product contract.

Every golden needs:

- source project folder or generation command;
- normalized summary JSON;
- Desktop version/provenance where applicable;
- `validate --strict` expectation;
- known unsupported surfaces;
- update rules.

### 5. Conformance Harness

Use `testing-conformance-harnesses`.

Purpose: test the CLI contract itself, not just individual Rust functions.

Required conformance cases:

- global `--json` works before or after the command;
- stdout is data and stderr is diagnostics;
- every read command returns stable handles;
- every mutator requires `--dry-run`, `--out-dir`, or guarded `--in-place`;
- every mutator returns readback, inspect, validate, diff, handoff, or Desktop
  proof commands as appropriate;
- unsupported Power BI features return `unsupported_feature`;
- generated projects pass `handoff check`;
- fixture summaries are deterministic and path-free;
- Desktop-unavailable environments return structured oracle-unavailable output,
  not false compatibility claims;
- generic report specs do not leak fixture/domain assumptions into generated
  names, labels, or measures unless the spec asked for them.

## Strategic Diagnosis

The project has proven that PBIP/PBIR/TMDL generation is viable and that a
sample report can open and render in Desktop. That is only the first foothold.

The next hard problem is turning the current primitives into a generic
dashboard authoring pipeline:

```text
schema/profile/intent
-> design plan
-> dashboard spec
-> PBIP/PBIR/TMDL build
-> inspect/lint/validate
-> fixture summary
-> Desktop canvas/refresh proof
-> handoff/rebind plan
```

The central gap is not one more handcrafted dashboard. The central gap is a
durable `report build` layer that composes existing primitives and works across
unknown schemas.

## Priority Implementation Slices

### P0: Declarative Dashboard Build

Add a plan-driven report builder, tentatively:

```text
powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir <project>
powerbi-cli report plan --schema <schema.json> --profile <profile.json> --objective <dashboard-goal> --out <dashboard.json>
powerbi-cli report spec validate <dashboard.json>
```

This should compose existing primitives rather than bypass them:

- scaffold;
- measures;
- calculated columns;
- relationships;
- pages;
- visuals;
- bindings;
- filters;
- slicers;
- layout;
- themes/style bundles;
- handoff checks;
- fixture summaries.

The first dashboard spec should support:

- report title and audience;
- pages;
- visual type;
- title and alt text;
- bindings by table/column/measure;
- measures to create;
- calculated columns to create;
- relationships to create or verify;
- drilldown hierarchy;
- drillthrough target pages;
- page/report/visual filters;
- slicers;
- layout intent or explicit coordinates;
- theme/style preset or extracted style bundle;
- source-template placeholders;
- proof requirements.

Definition of done:

- one command builds multiple distinct dashboard archetypes from specs;
- generated output is equivalent to manually composing the current CLI
  primitives;
- output JSON includes exact readback, validate, fixture, handoff, and Desktop
  proof commands;
- behavior is covered by golden and conformance tests;
- no domain-specific assumptions are embedded in the builder.

### P0: Schema And Profile Layer

The CLI needs to reason about arbitrary data before it can design arbitrary
dashboards.

Add:

```text
powerbi-cli schema validate <schema.json>
powerbi-cli schema normalize <schema.json> --out <canonical.json>
powerbi-cli profile infer --schema <schema.json> --rows <dummy.csv|json> --out <profile.json>
powerbi-cli profile validate <profile.json>
powerbi-cli profile summarize <profile.json> --json
```

The profile layer should identify:

- candidate fact tables;
- candidate dimensions;
- date/time columns;
- numeric measures;
- categorical dimensions;
- high-cardinality fields;
- key/relationship candidates;
- grain conflicts;
- missing or unsafe dummy rows;
- likely visual archetypes.

This is advisory, not magic. The agent should be able to override all inferred
choices in the dashboard spec.

### P0: Desktop Canvas And Refresh Oracle

Upgrade `desktop open-check` from launch/title proof to actual report proof.

Needed signals:

- Desktop started and opened the `.pbip`;
- no unresolved issue modal/banner;
- report canvas is not blank;
- expected pages exist;
- expected visuals are present;
- dummy partitions refresh;
- visuals show non-blank data when dummy rows exist;
- screenshot artifacts are captured when requested;
- Desktop process/window is closed after the check.

Add:

```text
powerbi-cli desktop refresh-check <project|pbip>
powerbi-cli desktop canvas-check <project|pbip>
powerbi-cli desktop screenshot <project|pbip> --page <page>
```

The exact command names may change after ergonomics review, but the proof level
must become explicit, for example `desktop-canvas-refresh`.

### P0: Desktop-Authored Visual Catalog

Harden `report visuals catalog` from generated patterns into fixture-backed
truth.

For each visual family, record:

- supported visual type aliases;
- required field roles;
- optional roles;
- cardinality limits;
- measure-only roles;
- category/date hierarchy behavior;
- formatting objects proven safe to set;
- unsupported role combinations;
- Desktop fixture provenance.

Start with generic dashboard workhorses:

- card/KPI;
- tableEx;
- matrix;
- line chart;
- clustered bar chart;
- clustered column chart;
- stacked bar/column chart;
- area chart;
- scatter/bubble chart;
- slicer;
- text box;
- gauge;
- decomposition tree if fixture-proven.

### P1: Dashboard Design Planner

Add a command that helps agents transform unknown data into a plausible
dashboard plan without immediately writing PBIR:

```text
powerbi-cli report design-plan --schema <schema.json> --profile <profile.json> --intent <intent.md|json> --json
```

It should return:

- detected model shape;
- candidate KPIs;
- candidate dimensions;
- candidate time axes;
- suggested pages;
- suggested visuals;
- required measures;
- drill candidates;
- filter/slicer candidates;
- warnings about weak/ambiguous schema signals;
- exact commands to create or validate the compiled dashboard spec.

This is the agent's thinking aid. It must be inspectable and override-friendly,
not a black box.

### P1: Style Bundle Layer

Implement first-class style bundles:

```text
powerbi-cli report style extract --project <template> --out style.json
powerbi-cli report style apply --project <target> --bundle style.json --out-dir <target-styled>
```

The bundle should compose existing raw and typed pieces:

- report theme;
- visual formatting bundles;
- title/alt text policy;
- static colors;
- page backgrounds;
- axis/legend/data-label defaults once fixture-proven;
- filter pane/card styling once fixture-proven;
- conditional formatting only after Desktop-authored fixtures.

The style layer must be schema-independent: a style extracted from a finance
report should be applicable to an operations report if visual archetypes match.

### P1: Source Template And Rebind Generalization

The locked-down corporate workflow is central. Source templates must not be SQL
only forever.

Add credential-free templates for:

- SQL Server;
- PostgreSQL;
- generic ODBC/OleDB shape where safe;
- Excel;
- CSV/folder;
- SharePoint/OneDrive paths as placeholders;
- generic M templates with explicit safety constraints.

Templates should remain sidecar metadata at home unless the user explicitly
chooses to write executable work-machine M. Handoff output must state what is
still placeholder vs executable.

### P1: Filter, Slicer, Drill, And Interaction Completeness

Expand only from Desktop fixtures.

Next supported surfaces:

- filter update without delete/add;
- advanced/range/TopN/relative date filters;
- slicer add/update/state;
- interaction default/reset semantics;
- bookmark CRUD and grouping;
- multi-field drillthrough;
- chart-family drilldown coverage;
- tooltip pages.

Each feature must have:

- fixture;
- validator rule;
- readback;
- mutation command;
- conformance case;
- `features list` status update.

### P2: Semantic Model Completeness

Continue from current measures, calculated columns, relationships, partitions,
and source templates.

Add:

- table/column CRUD beyond scaffold;
- calculated tables;
- named expressions;
- date table helpers;
- roles/RLS;
- perspectives;
- translations/cultures;
- calculation groups/items;
- deeper DAX formatting and lint rules beyond the current static
  dependency/reference pass;
- optional DAX execution through Desktop/Fabric bridge.

### P2: Durable Batch Operations

Only after primitives are trustworthy, add:

```text
powerbi-cli apply --ops ops.json
powerbi-cli plan validate ops.json
powerbi-cli plan replay ops.json --out-dir <project>
powerbi-cli plan diff before.json after.json
```

Batch operations must be inspectable and replayable. They should not become a
way to smuggle raw unvalidated PBIR/TMDL edits into the project.

## Dashboard Archetypes To Support

The CLI should support arbitrary dashboards by composing proven archetypes:

- KPI overview;
- time-series trend;
- period-over-period comparison;
- cohort comparison;
- category ranking;
- decomposition/tree analysis;
- detail table/matrix;
- scatter/bubble relationship;
- distribution/histogram;
- geographic/map view when fixture-proven;
- drilldown hierarchy;
- drillthrough detail page;
- exception/alert dashboard;
- style/template-cloned dashboard.

Each archetype needs a spec shape, visual catalog support, validator coverage,
golden output, and Desktop proof.

## Fixture Strategy

No single fixture should carry the whole regression burden. Split the roles
across generic archetypes:

- `regional-sales`: non-ASCII column/measure coverage, drillthrough chains,
  TopN-by-measure filters, and multi-page slicers;
- `flat-ops`: single-table operational dashboard example;
- `scatter-bubble`: bubble/scatter dashboard example;
- `catalog-proof`: minimal visual-family Desktop canvas-refresh proof;
- `sales`: tiny star-schema smoke test reused across most command tests.

Add other fixtures so the product does not overfit:

- sales star schema;
- finance P&L or budget-vs-actual dashboard;
- support ticket dashboard;
- logistics/shipment dashboard;
- HR headcount/attrition dashboard;
- inventory dashboard;
- project portfolio dashboard;
- survey/results dashboard;
- single flat CSV dashboard;
- multi-fact model dashboard.

Every fixture should be buildable from:

```text
examples/<domain>.schema.json
examples/<domain>.profile.json
examples/<domain>.dashboard.json
```

The generic proof loop should be:

```text
powerbi-cli report build --schema examples/<domain>.schema.json --profile examples/<domain>.profile.json --spec examples/<domain>.dashboard.json --out-dir build/<domain> --force --json
powerbi-cli validate --strict build/<domain> --json
powerbi-cli handoff check build/<domain> --json
powerbi-cli fixture normalize build/<domain> --out testdata/golden/<domain>.summary.json --json
powerbi-cli fixture verify build/<domain> --expected testdata/golden/<domain>.summary.json --json
powerbi-cli desktop canvas-check build/<domain> --json
```

Until `desktop canvas-check` exists, do not claim automated visual-render proof.

## Test Gates

Every serious implementation pass must run:

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
git diff --check
```

The CI workflow enforces that exact clippy command after `cargo check` on the
Linux, macOS, and Windows matrix jobs.

Feature-specific gates:

- CLI contract changes require conformance tests.
- Generated file changes require fixture normalization tests.
- Schema/profile/spec changes require schema validation and migration tests.
- Report builder changes require at least two different dashboard archetype
  goldens, so the builder cannot accidentally become domain-specific.
- PBIR visual, filter, slicer, bookmark, interaction, drill, or formatting
  changes require Desktop-authored goldens or explicit `unsupported_feature`
  status.
- Handoff/source-template changes require unsafe-data negative tests.
- Desktop oracle changes must prove process/window cleanup.

## Agent Output Contract

Every new command should preserve this contract:

- JSON mode works from global or command-local position.
- Success output includes stable object handles.
- Mutations include `changed`, `dryRun`, output paths, and exact next commands.
- Errors include stable `error.code`, explanation, and suggested commands.
- Diagnostic chatter goes to stderr, not stdout.
- Output should be deterministic enough for snapshot testing.
- Generated plans/specs should explain assumptions and uncertainty.
- When inference is weak, commands should ask for more input through structured
  missing-field diagnostics rather than guessing silently.

## Questions The CLI Should Help Agents Answer

For any new dataset, the tool should guide an agent toward answers for:

- What are the fact tables and dimensions?
- What is the grain of each table?
- What are the obvious measures?
- What are the time dimensions?
- Which columns are useful filters vs high-cardinality noise?
- What relationships are missing or ambiguous?
- Which dashboard archetypes fit the data?
- What measures/calculated columns need to be created?
- What drill paths make sense?
- What source templates are needed for work-machine rebind?
- What proof remains missing before this can be trusted?

This replaces hand-authored domain intuition with repeatable agent workflow.

## What Future Agents Should Read First

1. `goal.md`
2. `README.md`
3. `docs/roadmap.md`
4. `docs/pbir-desktop-oracle.md`
5. `docs/reviews/agent-first-review-synthesis.md`
6. `skills/powerbi-cli/SKILL.md`
7. `powerbi-cli --json capabilities`
8. `powerbi-cli features list --json`

If these disagree, trust the freshly built binary for current syntax, then
update the stale document or skill as part of the patch.

## Definition Of Done For The Next Major Pass

The next major pass is done when:

- the multi-angle analysis report exists and has been synthesized into this
  backlog;
- `report build --spec` or its final equivalent can build at least three
  materially different dashboard archetypes from declarative inputs;
- no single fixture report is treated as the goal; several materially
  different archetypes exist side by side;
- schema/profile/spec validation exists and catches bad generic inputs;
- Desktop canvas/refresh proof is automated or explicitly still marked
  unavailable;
- a Desktop-authored visual/filter/style golden corpus exists for the features
  claimed as supported;
- conformance tests cover the agent contract;
- `skills/powerbi-cli/SKILL.md`, `README.md`, and `docs/roadmap.md` reflect the
  data-agnostic CLI goal;
- Power BI Desktop windows/processes are closed after automated oracle runs;
- the repo is formatted, lint-clean, test-clean, and pushed.

## 2026-06-23 Implementation Record

This pass moved the goal from a domain-specific sample toward a generic
schema/profile/spec compiler loop:

```text
schema validate
-> profile infer/validate
-> report spec validate
-> report build
-> validate --strict
-> handoff check
-> fixture verify
```

The first implemented planner slice deliberately keeps inference shallow and
deterministic. `report plan` now emits an explicit dashboard spec from
schema/profile candidates plus a user objective, records decisions and warnings,
and must be followed by `report spec validate`, `report build`, strict
validation, handoff, and Desktop canvas/refresh proof before claiming
compatibility. The current positive goldens include generic sales and materially
different archetypes; broadening semantic inference still requires more
unrelated archetype and Desktop goldens.
