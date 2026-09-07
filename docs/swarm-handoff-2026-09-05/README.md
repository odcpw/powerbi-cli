# Swarm handoff 2026-09-05

Second Codex swarm session (gpt-6-astra medium). The tmux session died mid-run; the orchestrator
stopped all processes and preserved pane work as WIP commits.

## State at handoff

- `origin/main` before this push: `8146615`, the last fully green gate (69 test binaries).
- This push adds ~36 merge commits that are NOT yet verified by a full gate. The last complete
  gate (`173618a`) failed 5 test binaries:
  - planner refuses the `regional-sales` archetype (`report plan` in the e2e loop) — bead szr.7 thresholds/evidence
  - stale goldens: `catalog-proof` archetype (dashboard_build) and `slicer-rail` summary — bead 1qi.4 / 1qi.2
  - `report-compose-star.json` snapshot mismatch after planner changes — bead 1qi.11
  - schema/walker key-table drift — fix `2a307d8` merged afterwards, untested
- First step on the next machine: `cargo test --all-targets --no-fail-fast`, clear those failures,
  then close the merged beads (see `.beads`, each carries comments with commits and evidence).

## Branches with unmerged work (pushed as `ntm/powerbi-cli/cod_N`)

| branch | bead | state |
|---|---|---|
| cod_7 | 1qi.5 compile model sections | complete commit `88c8a6f`, not merged/gated |
| cod_1 | 2tn.6 measure patterns | WIP `515e6ff` (untested) |
| cod_4 | 2tn.3 date table | WIP `68cab87` (untested) |
| cod_5 | 1qi.13 tokens/defaults compile | WIP `132ed27` (untested) |
| cod_9 | sbl.7 walkthrough conformance | WIP `45e95f2` (untested) |
| cod_2 | mlf.6 design-plan --emit-ops | work lost (worktree clean); restart from bead |

## Operating notes that mattered

- Panes: `CARGO_BUILD_JOBS=2`, no workspace clippy in panes (orchestrator gate only), modules
  declared only in `src/lib.rs`, regenerate README/SKILL regions with
  `cargo run --quiet -- robot-docs render --json` after any catalog change, every new command
  path invoked in `tests/desktop_acceptance_everything.rs`.
- Merge one branch at a time and re-run the gate; parallel merges of branches touching
  `src/report_build.rs`, `src/report_spec_schema.rs`, `src/ops/mod.rs` and the catalogs conflict.
