# Overnight Hardening Report

Date: 2026-06-23

## Goal

Harden `powerbi-cli` as an agent-first, data-agnostic Power BI dashboard
authoring CLI. The target is not one fixture report; the target is a
repeatable workflow where an agent can take any safe schema/profile/dummy data,
write an explicit dashboard spec, build a PBIP project, validate/handoff-check
it, and later rebind it to live work data in Power BI Desktop.

## Review Inputs Actually Used

- `modes-of-reasoning-project-analysis`: used as the multi-angle analysis lens.
  NTM was not available on PATH, so the exact NTM swarm workflow could not run.
- `dueling-idea-wizards`: used as the adversarial idea lens. NTM was not
  available; `claude` was the only detected external model CLI.
- `testing-real-service-e2e-no-mocks`: applied to real Power BI Desktop instead
  of a mocked oracle.
- `agent-ergonomics-and-intuitiveness-maximization-for-cli-tools`: applied to
  validator exits, `next` commands, and first-command discoverability. The full
  skill scripts were not run.
- `testing-golden-artifacts`: applied to generated PBIP fixture summaries.
- `testing-conformance-harnesses`: applied to visual binding role contracts and
  shape-vs-compiled validation.
- `de-monolithize-your-codebase-isomorphically`: applied as an architecture
  lens. No broad split was attempted during this pass.
- `computer-use`: used to inspect real Power BI Desktop windows, refresh dummy
  partitions, verify rendered canvas content, and clean up Desktop processes.

Subagents:

- Fermat, systems/FMEA/de-monolith: found stale model-index validation for
  spec-local measures, schema/profile/report-build boundary drift, advisory
  profile ambiguity, duplicated raw JSON model vocabulary, and thin archetype
  coverage.
- Zeno, agent ergonomics: found validators returning `ok:false` with exit 0,
  unsafe `next` commands, unsupported `report plan` suggestions, weak live
  capability detail, and missing misuse tests.
- Hegel, golden/conformance: found only one positive macro golden, weak Desktop
  proof, missing visual role conformance, and missing assumption-breaker
  fixtures.

Additional Claude reviews were run independently for Desktop proof,
architecture, and agent ergonomics/conformance. They agreed on the main risks:
`desktop open-check` was launch-only, `main.rs` still carries legacy surface
area, the typed spec/raw JSON/PBIR models need consolidation, `profile` was not
enforced as build input, and agents needed a deterministic field inventory
before writing specs.

## Fixes Landed

- Invalid `schema validate`, `profile validate`, and compiled
  `report spec validate` now return exit code 10 and do not emit unsafe build
  follow-up commands.
- `profile infer` no longer points agents at unsupported `report plan`; it
  suggests `report spec validate` and `report build`.
- `report spec validate --spec <file>` without `--schema` is explicitly
  `shape-only`: `ok` is null, warnings explain what is unproven, and the next
  command tells agents to validate with `--schema`.
- `report build` now applies spec-local model measures before building the model
  index, so visuals can bind to measures declared in the dashboard spec.
- Visual binding validation now enforces the generated visual role contract:
  card max one Values binding, category/Y charts need Category and Y, scatter
  needs exactly one X and one Y, and category/legend roles must be columns.
- Ambiguous `Table[Name]` references now fail when a column and measure share a
  name; structured bindings are preferred.
- Added `report spec fields --schema <schema.json> [--profile <profile.json>]`
  so agents can discover exact column/measure references and structured binding
  objects before writing a dashboard spec.
- `capabilities` now advertises the generated visual role contract, the new
  `report spec fields` command, shape-only validation semantics, and the
  Desktop-proved archetypes.
- Added two non-domain archetype fixtures:
  `examples/archetypes/flat-ops.*` and
  `examples/archetypes/scatter-bubble.*`.
- Added golden summaries for both archetypes under
  `testdata/golden/archetypes/`.
- Added manual Desktop proof records under `testdata/desktop-proof/`.

## Desktop Proof

Power BI Desktop used:

- Store install path:
  `C:\Program Files\WindowsApps\Microsoft.MicrosoftPowerBIDesktop_2.155.756.0_x64__8wekyb3d8bbwe\bin\PBIDesktop.exe`
- Observed version: `2.155.756.0`.

Proofed generated archetypes:

- `flat-ops`: opened `WorkshopOperations.pbip`, clicked `Refresh now`,
  observed a rendered card (`Completed Items = 37`), clustered bar chart
  categories (`Assembly`, `Maintenance`, `Packaging`), and populated table
  rows. No Desktop or `msmdsrv` processes remained after cleanup.
- `scatter-bubble`: opened `FacilityPortfolio.pbip`, clicked `Refresh now`,
  observed rendered scatter/bubble points and table rows for `Central Hub`,
  `East Depot`, `North Plant`, and `West Yard`. No Desktop or `msmdsrv`
  processes remained after cleanup.

Important boundary: these records prove manual Desktop canvas/refresh behavior
for the two archetypes. The CLI command `desktop open-check` still only proves
launch/title and correctly reports `claimedCompatibility = false`.

## Tests Run

- `cargo check`
- `cargo fmt --check`
- `cargo test --test dashboard_build -- --nocapture`
- `cargo test --test cli_smoke -- --nocapture`
- `cargo test --test cli_contract -- --nocapture`
- Focused direct command run:
  `cargo run --quiet -- report spec fields --schema examples\sales.schema.json --profile examples\sales.profile.json --json`

Full `cargo test` should be run after this report update and before commit.

## Remaining P0

- Automate Desktop canvas/refresh proof inside the CLI, including issue-banner
  detection, blank-canvas rejection, screenshot/evidence capture, process-set
  cleanup, and non-interference with user-owned Desktop windows.
- Consolidate dashboard spec/model vocabulary so `main.rs`, `schema.rs`,
  `profile.rs`, `report_build.rs`, and PBIR/TMDL modules do not each maintain
  subtly different raw JSON contracts.
- Decide whether `--profile` is advisory only or a build-enforced compatibility
  input. Today it is surfaced and summarized, but the compiler still mainly
  trusts schema/spec.
- Add more archetype goldens: multi-fact star schema, time-series drilldown,
  matrix/table-heavy operational report, slicer/filter-heavy report, and style
  extraction/application.
- Build Desktop-authored golden fixtures for every visual/style feature before
  widening generated support.
