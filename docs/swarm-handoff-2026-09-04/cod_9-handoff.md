# cod_9 wind-down handoff

- Bead in progress: `pbi-t3-compiler-completeness-1qi.7` (the earlier
  `pbi-t4-pbir-catalog-expansion-sn2.16` rework is complete and was reported
  separately).
- Branch: `ntm/powerbi-cli/cod_9`
- Last commit: `04e9ff2` (`wip: pbi-t3-compiler-completeness-1qi.7 layout compiler in progress`)

## Complete

- Added v2 page-template resolution through the design-system grid, including
  deterministic named-slot coordinates and explicit-layout precedence.
- Added generated heading/subtitle textbox visuals with typography-token
  styling, slot-family warnings, duplicate-slot refusal, and pointer-rich
  unknown-slot diagnostics.
- Extended explain output with resolved template/slot coordinates and
  generated heading operations.
- Added focused `tests/report_build_layout.rs` coverage, including happy path,
  explicit override, refusal diagnostics, metamorphic coordinates, and explain
  output.
- Updated the v2 uncompiled-section boundary so these fields are no longer
  refused. All current edits are captured in the WIP commit above.

## Incomplete

- Feature catalog, command contract follow-up fields, README, SKILL, and
  `docs/roadmap.md` have not been updated for this bead.
- Robot-docs regions have not been regenerated.
- The WIP has not been converted into the required substantive bead commit.
- Section-divider shapes remain intentionally omitted with a `feature_pending`
  warning until their owning capability is available.

## Known failing tests

- None known from the targeted checks run before wind-down; no tests or builds
  were run after the WIP snapshot, per the wind-down order.
- Final merged-tree gates remain unverified.

## Next step

Update the feature/contract/docs catalogs and regenerate robot-docs, then run
the targeted layout and regression tests (followed by the orchestrator's
merged-tree gates), review the diff, and replace the WIP snapshot with the
substantive `pbi-t3-compiler-completeness-1qi.7` commit.
