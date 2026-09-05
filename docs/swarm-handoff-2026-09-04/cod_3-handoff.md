# cod_3 handoff

- Bead in progress: `pbi-t11-pilot-backlog-iaw.5`
- Branch: `ntm/powerbi-cli/cod_3`
- Last commit: `1236aaa` (`wip: pbi-t11-pilot-backlog-iaw.5 atomic set-object batch draft`)

## Complete

- Added `report visuals set-object --batch <ops.v1.json>` dispatch in both the
  library and binary module lists.
- Added a focused batch adapter that reads bounded `powerbi-cli.ops.v1`
  input through `input_safety::read_ops`, accepts only `setObject` operations,
  preflights catalog object/property pairs, validates the plan, applies all
  entries through `SetObjectKernel` and `Transaction`, and renders per-entry
  changes/readback commands for dry-run, out-dir, and in-place modes.
- Added module unit tests for canonical/legacy tags, unsupported operation
  refusal, and formatting-catalog payload preflight.

## Incomplete

- Integration tests, the 4ve.4 equivalence-harness registration, snapshots,
  contract/catalog/help updates, README/SKILL/roadmap documentation, and
  robot-docs regeneration are not done.
- No completed iaw.5 verification or final bead commit exists.

## Known failing test

`cargo test --lib report_visual_object_batch` was run before wind-down and
failed `batch_reader_rejects_non_set_object_before_project_access`: the shared
`read_ops` refusal has no pointer, while the draft test expects `/ops/0/op`.
The other two batch unit tests passed. The failure is recorded as-is; no
additional test or build was run after the wind-down order.

## Next step

Fix the refusal assertion/pointer handling (or attach the pointer at the batch
adapter boundary), then add the required integration/equivalence tests and
snapshots, update the command contracts and docs, regenerate robot-docs, run
the targeted quality checks, and replace this WIP with the completed iaw.5
commit/report.
