# cod_8 wind-down handoff

- Bead(s) in progress: `pbi-t3-compiler-completeness-1qi.2` (cod_8-f.md). The `pbi-t12-docs-as-contract-szu.1` rework commits are complete.
- Branch: `ntm/powerbi-cli/cod_8`
- Last commit: `8d5850a` (`wip: pbi-t3-compiler-completeness-1qi.2 slicer rail compiler pending verification`)

## Complete

- Added v2 page and layout-rail slicer schema support, including rail width/side, page opt-in, slicer mode, single-select, and title fields.
- Added profile-aware page/rail slicer compilation with generated visuals, deterministic rail placement and stacking, cardinality-based Basic/Dropdown selection, and explicit pending warnings when profile cardinality is unavailable.
- Added typed generated slicer operations to report-plan/explain output and slicer-rail archetype fixtures/tests.
- Updated the feature catalog, contract capability metadata, README, and SKILL generated documentation regions.
- Committed all current compiler, fixture, test, catalog, and documentation changes as the WIP commit above without additional verification during wind-down.

## Incomplete

- The WIP commit has not yet been replaced with the substantive compiler bead commit.
- The final post-edit quality gates and the complete cod_8-f.md assignment remain to be run.
- The report/docs renderer should be rerun after any merge and its generated regions rechecked.

## Known failing tests

- No failing test was observed in the targeted suites already run before the final rail-stacking/schema edits. The post-edit gates are unverified; an in-progress `cargo check --all-targets` was interrupted by this wind-down order after it printed `Finished`.

## Next step

From this worktree, rerun the gates and focused suites, then replace the WIP with the substantive bead commit and continue cod_8-f.md:

```sh
export PATH="/home/oliver/.cargo/bin:$PATH"
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test report_slicer_rail --test report_spec --test report_spec_schema_explain --test report_spec_missing_input --test dashboard_build
cargo test --test fixture --test cli_contract --test robot_docs --test desktop_acceptance_everything
cargo test --test e2e --test input_safety --test cli_contract --test desktop_acceptance_everything
git diff --check
```

After every merge, refresh the generated documentation regions with:

```sh
PATH="$HOME/.cargo/bin:$PATH" cargo run --quiet -- robot-docs render --json
PATH="$HOME/.cargo/bin:$PATH" cargo run --quiet -- robot-docs render --check --json
```
