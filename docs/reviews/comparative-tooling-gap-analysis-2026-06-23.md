# Comparative Power BI Tooling Gap Analysis

Date: 2026-06-23

## Method

This pass compared `powerbi-cli` with the local research corpus under:

```text
<workspace>\powerbi-cli-research
```

No local `ultracode` executable was available, so the review used an
ultracode-style workflow: three independent `claude -p` reviewers, each focused
on one axis, followed by local verification before writing this synthesis.

Raw untracked review outputs were written to:

- `build/reviews/claude-ultracode-architecture.md`
- `build/reviews/claude-ultracode-agent-ux.md`
- `build/reviews/claude-ultracode-validation.md`

The selected skills for this pass were:

- `research-software`: compare local source-first, not README-only.
- `codebase-pattern-extraction`: extract reusable patterns across tools.
- `multi-model-triangulation`: use independent Claude reviews before synthesis.

Clean-room rules remain the guardrail. The repository already records the
license map and use boundaries in `docs/clean-room-research.md:13`,
`docs/clean-room-research.md:16`, and `docs/clean-room-research.md:32`.
Restricted/quarantined repositories can inform requirements, workflows, and
tests, but not copied code, JSON templates, examples, or prose.

## Executive Synthesis

`powerbi-cli` is aiming at the right niche: a cross-platform, offline-safe,
agent-first compiler/workbench for PBIP/PBIR/TMDL projects. The strongest
competitors either assume a live Windows/Desktop model session, focus on report
file hygiene, or serve Fabric/service workflows. That leaves a real opening for
deterministic schema/profile/spec-to-PBIP generation with strong proof and
handoff gates.

The main gap is not one missing chart. The main gap is system hardness as the
surface grows: contract drift, repeated command plumbing, incomplete universal
exit-code invariants, missing report audit/sanitize passes, and no automated
canvas/refresh Desktop oracle yet.

The tool should not become a clone of live Desktop automation or Fabric
deployment tooling. It should stay compiler-first, then add optional bridge
adapters for DAX validation, Desktop proof, rebind checks, and service metadata
when those environments are available.

## Comparator Matrix

| Tool | Approach | What We Should Learn | Gap In `powerbi-cli` | Risk If Ignored |
|---|---|---|---|---|
| `MinaSaad1/pbi-cli` | Agent-first Python CLI with a live semantic-model layer and a PBIR report layer. It requires Windows/Desktop for the model layer and advertises skills, report commands, visual breadth, and custom visuals (`README.md:64`, `README.md:98`, `README.md:138`, `README.md:158`, `README.md:203`, `README.md:226`, `README.md:230`). | Agent UX should be skill-backed, broad, and command-discoverable. Separate live semantic-model operations from file-based PBIR report work. | We have strong discovery, but no comparable skill set for report-building patterns, DAX validation, or custom visuals. | Agents may have primitives but not enough playbooks to build confidently from arbitrary schemas. |
| `akhilannan/pbir-utils` | PBIR validation, sanitize, metadata extraction, wireframe/UI, and report hygiene (`README.md:40`, `README.md:42`, `README.md:46`, `README.md:48`, `README.md:58`, `README.md:71`, `src/pbir_utils/api/models.py:92`). | Treat audit/sanitize/wireframe as first-class development tools, not side utilities. Rules should be configurable and CI-friendly. | `lint` exists, `wireframe export` exists, but there is no generalized rule engine or safe sanitizer action framework. | Generated reports can be valid enough to open but still messy, stale, inaccessible, or brittle. |
| `microsoft/powerbi-modeling-mcp` | Official local MCP bridge for semantic model operations, DAX queries, transactions, PBIP model connection, and read/write modes (`README.md:3`, `README.md:11`, `README.md:19`, `README.md:26`, `README.md:101`, `README.md:156`, `README.md:157`, `README.md:163`, `README.md:194`, `README.md:206`, `README.md:207`). | Semantic model mutation and DAX execution are a different layer from PBIR report editing. Transactions and read-only/read-write modes are important. | Offline TMDL edits are covered, but DAX semantic validation is not. No MCP/Desktop/Fabric bridge command exists. | Agents can author measures that parse structurally but fail in the real DAX engine. |
| `microsoft/semantic-link-labs` | Fabric/Python workflows for report BPA, metadata, broken reports, themes, rebind, and save-as-PBIP (`README.md:55`, `README.md:56`, `README.md:58`, `README.md:59`, `README.md:61`, `README.md:62`). | Work-machine handoff should eventually include rebind and broken-report checks when Fabric/service access exists. | Current handoff is file-safety and source-template oriented, not live rebind validation. | A project can be safe to carry but still fail at the work-machine rebind step. |
| `TabularEditor/TabularEditor` | Mature TOM/tabular model authoring ecosystem. | Treat semantic-model features as deep enough to deserve their own architecture and proof gates. | We cover measures, calculated columns, relationships, and partitions, but not roles, perspectives, translations, calculation groups, date tables, or BPA-style model rules. | The tool may build visually useful reports on weak semantic models. |
| `DaxStudio/DaxStudio` | DAX/query/performance reference tool. | DAX validation, query execution, dependency tracing, and performance feedback are not optional forever. | DAX is written as text blocks; local validation does not execute or analyze it (`README.md:198`). | Agents will over-trust syntactically stored DAX. |
| `maxanatsko/pbir.tools` | Quarantined. Report-path workflows, tree/model inspection, visual add, bulk set, backup, validate, publish, and agent workflows (`README.md:8`, `README.md:12`, `README.md:23`, `README.md:73`, `README.md:76`, `README.md:77`, `README.md:115`, `cli/workflows.md:30`, `cli/workflows.md:37`). | Workflow-level lesson only: tree/path browsing, backup-before-bulk, validate-after-mutation, and selector-based bulk edits are agent-useful. | We have handles and many mutations, but no general `report tree/find/cat/query` path language and no snapshot-before-bulk primitive. | Agents will need raw filesystem spelunking for broad changes, increasing corruption risk. |
| `pbi-tools/pbi-tools` | Quarantined. Enterprise source-control/extract/compile/deploy workflows around PBIX/PBIT/Desktop (`README.md:5`, `docs/usage.md:23`, `docs/usage.md:53`, `docs/usage.md:92`, `docs/usage.md:186`). | Source-control and deployment edges matter, but they are not the same product as offline authoring. | We do not extract PBIX, compile PBIX/PBIT, or deploy. That is acceptable for now. | Chasing these too early would pull the core away from deterministic offline PBIP generation. |
| `data-goblin/power-bi-agentic-development` | Quarantined. Agent workflow/skills expectations and broad Power BI task coverage. | Agents need workflows, not just commands. Use only high-level behavioral lessons. | Current `skills/powerbi-cli/SKILL.md` exists, but the project needs more task-oriented agent playbooks. | Future agents will rediscover ordering, proof, and fallback boundaries by trial and error. |

## What We Can Learn

### 1. Keep The Compiler/Workbench Boundary

The existing thesis is still correct. `README.md:3` frames the tool as
offline-safe PBIP/PBIR/TMDL authoring, `README.md:19` says it does not generate
PBIX directly, and `docs/clean-room-research.md:72` through
`docs/clean-room-research.md:82` identify the compiler-like workflow as the
distinct niche.

Do not chase a broad interactive Desktop control surface as the core product.
Use Desktop as an oracle and optional bridge. Keep schema/profile/spec inputs
deterministic, reproducible, and safe to carry between machines.

### 2. Add A Thin Shared Spine Before Adding More Features

Claude reviewer A found and local `rg` confirmed repeated helpers across many
modules:

- `take_value` appears across schema, profile, handoff, fixture, model, report,
  visual, theme, and source-template modules.
- `required_project`, `shell_arg`, `MutationMode`, `target_project`,
  `require_mode`, and `set_mode` are repeatedly redefined.
- `report_filter_mutations.rs:14`, `report_filter_mutations.rs:369`,
  `report_filter_mutations.rs:386`, `report_filter_mutations.rs:398`,
  `report_filter_mutations.rs:416`, `report_filter_mutations.rs:428`, and
  `report_filter_mutations.rs:448` already expose versions of these helpers,
  but the rest of the codebase does not consistently reuse them.

This is not a monolith problem. It is a "many files, one repeated contract"
problem. The next hardening pass should extract a small shared CLI support layer
for:

- argument cursors and required values;
- project resolution;
- mutation mode parsing;
- shell-escaped suggested commands;
- stable mutation envelopes;
- common `next` command builders.

This makes future command families cheaper and makes contract drift testable.

### 3. Make Contract Conformance Universal

`cli.rs:132` through `cli.rs:144` default to exit success when a JSON payload
has no `exitCode`. That is fine for many read-only commands, but dangerous for
commands that emit `ok:false`.

Verified current gaps:

- `src/lint.rs:44` emits `ok` but no `exitCode`.
- `src/report_design.rs:51` emits `ok` but no `exitCode`.
- `src/report_build.rs:807` and `src/report_build.rs:822` compute validation
  state in the build response, but the response does not carry a top-level
  `exitCode`.

The contract should not rely on every future command author remembering this.
Add a universal invariant: any payload with top-level `ok:false` and no explicit
reason to be informational must exit nonzero. Then add conformance tests that
walk the command catalog.

Also fix catalog drift:

- `version` is dispatchable in `src/cli.rs:85`, but is not currently surfaced
  as a catalog command.
- `--robot-triage` always emits JSON in `src/cli.rs:97`; either document that
  carve-out or route it through normal output-mode handling.
- `help_text` and robot docs should agree on schema/profile/spec/build command
  order. `contract.rs:123` through `contract.rs:127` and `contract.rs:222`
  through `contract.rs:226` show the intended flow.

### 4. Build Report Introspection Like A Filesystem

Other report tools point to the same need: agents need to browse, select, patch,
and re-check report objects without opening raw JSON by hand. The current tool
has `inspect --deep`, `report visuals list/show`, `report pages list/show`,
filters, slicers, interactions, bookmarks, themes, and wireframes. That is a
solid base, but it is still command-family-specific.

Add a generic report object layer:

- `report tree --project <pbip> --json`
- `report find --project <pbip> --kind visual --visual-type slicer --json`
- `report cat --project <pbip> --handle <handle> [--include-raw] --json`
- `report query --project <pbip> --selector <selector> --json`
- `report path explain --selector <selector> --json`

These should emit stable handles, JSON pointers, safety classifications, raw
payload availability, and suggested next commands. This lets agents operate on
the report as a graph of objects instead of a folder of vendor JSON.

### 5. Add Audit And Sanitizer Primitives

`pbir-utils` makes report hygiene first-class. `powerbi-cli` should do the same
with an agent-safe contract:

- `report audit --project <pbip> --rules <rules.json|builtin> --json`
- `report audit explain <rule-id> --json`
- `report sanitize plan --project <pbip> --actions <list> --json`
- `report sanitize apply --project <pbip> --actions <list> --dry-run|--out-dir|--in-place --json`

Initial built-in rules should cover:

- broken semantic field references;
- stale visual interaction references;
- blank or duplicate page titles;
- duplicate visual titles on a page;
- hidden visuals never reached by bookmarks;
- empty pages;
- slicers with persisted dummy data values;
- filters with unsafe literal values;
- unsupported or unknown PBIR objects;
- missing alt text on visible visuals;
- visuals outside page bounds;
- style bundle literal text leakage;
- report-level measures that should be semantic-model measures.

Sanitizers must be dry-run by default and should use snapshots for risky or
bulk operations.

### 6. Separate Offline DAX Storage From DAX Engine Validation

The current README is honest: DAX measures and calculated columns can be
authored as TMDL blocks, but local validation does not prove engine semantics
(`README.md:198` through `README.md:206`). Microsoft Modeling MCP and DAX Studio
show the missing bridge: query execution, validation, metrics, and dependency
analysis.

Recommended future command family:

- `model dax parse --project <pbip> --expression-file <file> --json`
- `model dax validate --project <pbip> --engine desktop|mcp|fabric --json`
- `model dax query --project <pbip> --query-file <file> --engine desktop|mcp|fabric --json`
- `model dax dependencies --project <pbip> --json`

Offline mode should never pretend to execute DAX. It can check references and
emit a bridge plan. Live validation should be opt-in, environment-aware, and
clearly marked as using Desktop/MCP/Fabric credentials.

### 7. Automate Desktop Canvas And Refresh Proof

The project already has the right proof language. `README.md:22` through
`README.md:31` and `goal.md:78` through `goal.md:82` identify Desktop as the
oracle and distinguish launch proof from canvas/refresh proof. `desktop.rs:250`
through `desktop.rs:295` reports launch-only proof with
`claimedCompatibility:false`.

The missing piece is automation:

- open PBIP in Desktop;
- detect issue banners;
- refresh dummy partitions;
- verify page count and expected visual containers;
- inspect rendered canvas pixels or accessible UI state;
- fail on blank pages;
- save/round-trip if needed;
- close the launched Desktop window/process.

There is also a current cleanup mismatch: `goal.md:89` requires Desktop oracle
runs not to leave windows/processes hanging, while `desktop.rs:318` starts
Desktop and no cleanup path is visible. Either make `open-check` explicitly
interactive/manual, or add `--close-after` / `--no-close-after` with default
cleanup for automated checks.

### 8. Treat Style As A Reusable Artifact

The current tool can extract/apply theme bundles and visual formatting bundles
(`README.md:286` through `README.md:289`). The next step is to make style
portable and auditable:

- `style bundle inspect`;
- `style bundle diff`;
- `style apply --map-fields <mapping.json>`;
- `style lint --rules corporate|accessibility`;
- fixture-backed conditional formatting authoring only after Desktop goldens.

Do not infer complex conditional formatting from memory. Build it from
Desktop-authored fixtures and preserve raw unknown structures unless a typed
mutator is proven.

## Prioritized Roadmap

### P0: Contract And Safety Spine

1. Add `cli_support` shared helpers and replace repeated argument/mutation-mode
   plumbing.
2. Add typed output envelope builders for read, validation, mutation, and proof
   responses.
3. Add conformance tests:
   - every dispatchable command appears in capabilities;
   - every capabilities command has a runnable help or structured refusal;
   - top-level `ok:false` exits nonzero unless explicitly informational;
   - `features list` commands exist or intentionally refuse;
   - help text, robot docs, robot triage, and capabilities agree on key flags.
4. Fix the known exit-code gaps in `lint`, `report design-plan`, and
   `report build`.
5. Add `version` to capabilities or remove it from the public dispatch surface.

### P0: Agent Report Introspection

1. Implement `report tree/find/cat/query`.
2. Make handles, JSON pointers, object paths, raw-availability, and safety
   summaries consistent across all report object families.
3. Add golden tests from existing generated projects and at least one
   Desktop-authored PBIP fixture.

### P0: Audit/Sanitize

1. Implement `report audit` with built-in rules and JSON findings.
2. Implement `report sanitize plan` before `apply`.
3. Require `--dry-run`, `--out-dir`, or confirmed `--in-place` for every
   sanitizer mutation.
4. Add snapshot creation before in-place bulk sanitizer operations.

### P0: Desktop Oracle Cleanup And Canvas Proof

1. Make `desktop open-check` cleanup behavior explicit.
2. Add `desktop canvas-check` as a separate command instead of overloading
   launch proof.
3. Record Desktop version, PBIP path, expected pages, expected visuals, issue
   banners, refresh result, screenshot/pixel/accessibility signals, and cleanup
   result.
4. Keep Linux/macOS behavior structured as oracle-unavailable, not failed
   generation.

### P1: Visual And Style Breadth

1. Expand the Desktop-authored visual fixture corpus.
2. Promote visual families only from Desktop-proven fixtures.
3. Add matrix, pie/donut, map, slicer creation, tooltip pages, bookmark
   mutations, conditional formatting, and richer interactions only behind
   fixture gates.
4. Add style bundle inspect/diff/lint.

### P1: Model/DAX Bridge

1. Keep offline TMDL mutation in core.
2. Add optional Desktop/MCP/Fabric bridge commands for DAX validation and query
   execution.
3. Add transaction/read-only/read-write concepts for live model bridges.
4. Add model BPA-style lint rules: hidden columns, measure formatting,
   relationship direction, date table, unused fields, dependency cycles.

### P1: Handoff/Rebind Proof

1. Extend `source-template` beyond SQL only when safe.
2. Add work-machine `handoff rebind-check` that can validate replacements
   against live Desktop/MCP/Fabric when available.
3. Add broken-report and missing-field checks after rebind.
4. Keep home/offline mode credential-free.

### P2: Service And Packaging Edges

1. Optional adapters for Fabric report metadata, rebind, publish, and save PBIP.
2. Optional custom visual SDK workflow.
3. Optional PBIX/PBIT extract/compile/deploy integration only if it stays at the
   edge of the product and does not pollute the offline compiler core.

## Non-Goals And Traps

- Do not vendor restricted repositories.
- Do not copy quarantined code, JSON, templates, examples, or prose.
- Do not make Windows Desktop automation a requirement for normal generation.
- Do not generate `.pbix` binaries in the core CLI.
- Do not broaden visual types with guessed PBIR.
- Do not claim DAX correctness without an engine.
- Do not let `report build` silently infer dashboards from vague intent until
  planner proof exists.
- Do not turn the codebase into one giant file, but also do not keep copying
  the same command plumbing across dozens of modules.
- Do not treat Fabric/service deployment as the first problem. It is a later
  handoff edge.

## Immediate Implementation Backlog

These are the highest-leverage next issues to create or implement:

1. `cli_support`: shared argument parsing, project resolution, mutation modes,
   shell escaping, and output envelopes.
2. Contract conformance test suite for dispatch/capabilities/help/robot docs.
3. Universal `ok:false` exit-code invariant.
4. `desktop open-check --close-after` and/or separate `desktop canvas-check`.
5. `report tree/find/cat/query` generic introspection.
6. `report audit` built-in rule engine.
7. `report sanitize plan/apply` with dry-run and snapshot semantics.
8. Desktop-authored fixture corpus expansion for visual families and slicers.
9. Optional `model dax validate` bridge plan using Desktop/MCP/Fabric.
10. Handoff rebind-check design for work-machine execution.

## Evidence Index

Local project evidence:

- Offline-safe authoring thesis: `README.md:3`, `README.md:15`,
  `README.md:19`.
- Desktop oracle honesty: `README.md:22` through `README.md:31`,
  `docs/pbir-desktop-oracle.md:35` through `docs/pbir-desktop-oracle.md:43`,
  `desktop.rs:250` through `desktop.rs:295`.
- No fake fallback policy: `README.md:36` through `README.md:42`.
- Current feature limits: `README.md:165` through `README.md:313`.
- Goal non-negotiables: `goal.md:78` through `goal.md:89`.
- Desktop canvas oracle requirement: `goal.md:450` through `goal.md:464`.
- DAX limitation honesty: `README.md:198` through `README.md:206`.
- Clean-room rules: `docs/clean-room-research.md:32` through
  `docs/clean-room-research.md:63`.
- Product niche: `docs/clean-room-research.md:65` through
  `docs/clean-room-research.md:82`.

Comparator evidence:

- `MinaSaad1/pbi-cli` agent/Desktop/report split:
  `README.md:64`, `README.md:98`, `README.md:138`, `README.md:158`,
  `README.md:203`, `README.md:221`, `README.md:226`, `README.md:230`.
- `akhilannan/pbir-utils` audit/sanitize/wireframe:
  `README.md:40`, `README.md:42`, `README.md:46`, `README.md:48`,
  `README.md:58`, `README.md:71`, `src/pbir_utils/api/models.py:92`.
- Microsoft Modeling MCP semantic/DAX/transaction/PBIP boundary:
  `README.md:3`, `README.md:11`, `README.md:19`, `README.md:26`,
  `README.md:101`, `README.md:156`, `README.md:157`, `README.md:163`,
  `README.md:194`, `README.md:206`, `README.md:207`.
- Semantic Link Labs report/service workflows:
  `README.md:55`, `README.md:56`, `README.md:58`, `README.md:59`,
  `README.md:61`, `README.md:62`.
- Quarantined `pbir.tools` workflow signals:
  `README.md:8`, `README.md:12`, `README.md:23`, `README.md:73`,
  `README.md:76`, `README.md:77`, `README.md:115`,
  `cli/workflows.md:30`, `cli/workflows.md:37`.
- Quarantined `pbi-tools` source-control/deployment edges:
  `README.md:5`, `docs/usage.md:23`, `docs/usage.md:53`,
  `docs/usage.md:92`, `docs/usage.md:186`.
