# Compiler surface ergonomics — 2026-09-05

Bead: `pbi-t3-compiler-completeness-1qi.14`. Scoped full audit/apply pass,
on `ntm/powerbi-cli/cod_3`, using rubric 1.0.0 of the agent-ergonomics skill.
This supplements the older pass-1/pass-2 artifacts without changing their scores.
Two independent read-only surface reviews were followed by bounded fixes and
peer review. No new commands, architecture changes, tracker writes, network
calls, or Desktop claims. One commit, targeted tests and cargo check override
the skill's generic multi-commit/full-suite/tracker/push procedures. Existing
in-tree audit workspace and installed Rust toolchain were reused; no CASS mining
or tool installation was needed. The final verification record is the assigned
orchestrator report `cod_3-f.md`.

## Surface inventory and outcomes

| Existing surface | Shape / follow-up review | Output safety / diagnostic review |
| --- | --- | --- |
| `report spec schema` | Versioned schema reader; focused discovery remains bounded | Read-only, no project modes needed; strict schema tests already cover both versions |
| `report spec explain` | Fixed nested plan's false `ops.v1` identity: annotated entries now use `powerbi-cli.report.spec.explainPlan.v1`; fields remain unchanged | Read-only; uncompiled-section warning now registered; generic schema error gap remains below |
| `report spec upgrade` | Existing normalized v1→v2 response/next contract retained | External-file writer supports dry-run; existing-output refusal lacks pointer |
| `report spec normalize` | Deterministic normalized artifact and provenance; next entries are templates | External-file writer, not a project mutation; overwrite/mode gap remains below |
| `report build` / spec validate | Fixed dropped `--profile` in dry-run, validation, and scorecard follow-ups; existing `scorecard.v1` / proofPlan stay unchanged | New project build supports dry-run/out-dir, not in-place; proofPlan only proposes proof and does not claim Desktop execution |
| `report layout auto` | Existing mutation/readback and named-grid response retained | Three project modes; fixed RFC 6901 escaping of unknown grid settings |
| `report wireframe export` | Explicit mode-specific catalog mapping: default JSON v1, SVG/HTML artifact v2 | External-artifact dry-run/out, no in-place; fixed escaped grid-setting pointers |
| `robot-docs guide/render/verify` | Fixed root-losing next and recovery commands; selected render sections preserved | Guide/check/verify read-only; render updates selected generated documentation regions, not PBIP |
| `desktop harvest-reference` | Existing archive and pending-proof responses retained | External artifact dry-run/out; discovered persisted-state pointer is now structured, not buried in prose |
| `handoff rebind-check` | Fixed placeholder recovery when actual project is known | Read-only; unmatched selector now points to `/table` or `/partition`; finding codes already registry-checked |
| `report visuals set-object --batch` | Added full responseShapes descriptor and mode mapping | Three transactional modes; unknown fields/conflicting tags/missing fields now refuse before project access; null points to `/value`, malformed JSON uses root pointer `""` |
| Internal OpPlan | Distinguished durable ops.v1 input from annotated explain output; no general ops command advertised | Seven existing ops error codes registered; validation/kernel mismatch errors gain remediation and runnable explain commands |

The focused-capability test covers all thirteen advertised command paths above
(robot-docs counted as three), a 20,000-byte full-focused cap, an 8,000-byte
compact cap, deterministic output, and leading/trailing `--json` on those
**discovery queries**. It does not claim to execute every command in both global
flag positions. Internal OpPlan is intentionally not a capability command.

## Scoring and applied recommendations

Scores are conservative source-review rubric anchors, not benchmarks or a claim
of repository-wide polish. Dimensions in order: intuitiveness, ergonomics,
ease-of-use, parseability, error pedagogy, intent inference, safety/recovery,
determinism, self-documentation, composability, regression resistance.
Baseline for each scoped surface: `500,500,500,500,500,500,500,500,500,500,500`.
Read-only safety is not inflated to 1000; no unsupported >700 scores are used.

| Surface | Post-pass dimension changes from baseline | Evidence / expected user benefit |
| --- | --- | --- |
| spec explain | parseability 700; self-documentation 700; regression resistance 700 | `src/report_spec_explain.rs::build_plan_json`, five explain snapshots; no false replay promise |
| build / spec validate | ergonomics 700; regression resistance 700 | `tests/report_build_response.rs::build_and_validation_followups_retain_the_supplied_profile`; preserves input context in one retry |
| robot-docs | ergonomics 700; error pedagogy 700; regression resistance 700 | `tests/robot_docs.rs::follow_up_commands_preserve_explicit_root_and_selected_sections`; no accidental retry against cwd |
| layout / wireframe | error pedagogy 700; regression resistance 700 | Both module tests `unknown_grid_setting_escapes_rfc6901_pointer_tokens`; exact malformed-key location |
| harvest-reference | error pedagogy 700; regression resistance 700 | `persisted_filter_literal_refusal_carries_exact_rfc6901_pointer`; structured persisted-state location |
| rebind-check | ergonomics 700; error pedagogy 700; regression resistance 700 | `unmatched_selector_points_to_argument_and_recovers_against_the_actual_project`; real project recovery |
| set-object batch | parseability 700; error pedagogy 700; safety/recovery 700; regression resistance 700 | `batch_field_errors_are_pointer_precise_before_any_output_mode_writes`; earlier deterministic refusal and preserved source |
| OpPlan | error pedagogy 700; self-documentation 700; regression resistance 700 | `plan_errors_have_registered_remediation_and_executable_explanations`; `lint --explain ops.*` recovery is executable |
| schema / upgrade / normalize | no score uplift claimed | Reviewed; unresolved gaps retained below |

Prioritized changes were small patches at existing response/validation seams,
not renamed commands or guessed PBIR support. Fresh-eyes review found the
remaining encoded-value gap below; it was not hidden by the structural fixes.
The second peer review caught batch error precedence: operation-tag refusal
must precede payload-field checks. That was corrected without weakening the
existing non-SetObject test; the first compiled run reproduced the old failure.
The documentation verifier also caught an incorrect standalone `explain`
recovery command; all new rule recovery commands use the verified existing
`lint --explain` path. Neither failure was bypassed by changing a gate.
The skill's ambition self-prompt was run: a second apply round added grid,
harvest, rebind and mode-specific catalog fixes. The generic ten-commit bar is
inapplicable because the assignment explicitly requires one commit. No broad
CLI-completion or two-clean-full-swarm claim is made.

## Remaining defects and owner handoffs

| Defect / constraint | Owning bead | Reason retained |
| --- | --- | --- |
| normalize overwrites existing outputs (including source); undocumented `--out-dir` means a file and no dry-run exists | `pbi-t2-dashboard-spec-v2-dsd.4` | Needs an explicit artifact-overwrite/alias compatibility policy shared with schema normalize; do not silently change that policy in this pass |
| spec explain aggregates schema-validation errors under generic validation_failed without precise input pointers | `pbi-t2-dashboard-spec-v2-dsd.2` | Needs the compiler's native structured schema diagnostic records, not a fabricated `/schema` pointer; the uncompiled-section warning was registered in this pass |
| upgrade existing-output refusal lacks structured pointer | `pbi-t2-dashboard-spec-v2-dsd.6` | CLI output-path pointer convention should be settled with the artifact-writer policy |
| batch value preflight checks non-null and catalog pair, not exact property-specific PBIR encoding | `pbi-t11-pilot-backlog-iaw.5` / `pbi-t1-operation-ir-4ve.2.8` | Must enforce the same proven value grammar in both direct kernel and batch paths; structural validation is not credited as this fix |
| generated single-quoted arguments double apostrophes (PowerShell); literal POSIX execution loses apostrophes | `pbi-t0-contract-repairs-2nw` | Requires a cross-platform shell contract and shared helper tests, not local inconsistent quoting |
| Unshipped designs cannot be run, advertised, or frozen through changes to owner acceptance text | owners below | Orders prohibit new commands and all `.beads` edits; orchestrator must reconcile/approve the design handoff |

These gaps mean this report does **not** certify the broad bead's final
all-unshipped-command acceptance criterion or authorize closing its owners.

Final synchronization was against main `c18d113`. That merge added slicer-rail
to the shared archetype registry but omitted
`tests/snapshots/report-spec-explain-slicer-rail.json`, so
`explain_snapshots_cover_every_checked_in_archetype` fails with a missing-file
error on the merged tree. All six explain tests passed before that merge;
the other five still pass after it. This unrelated missing artifact belongs
to `pbi-t3-compiler-completeness-1qi.2` and is deliberately not generated or
bypassed here, per AGENTS.md.

## Design handoff: proposed final usage and schema names

These are concrete implementation contracts for owner approval, not advertised
capabilities. They deliberately distinguish file artifacts from project
transactions and retain canonical `ops.v1` only for replayable typed operations.
Existing plan entry points and flags remain valid until owners implement these
contracts. Avoid claiming a name freeze before that approval.

| Owner | Proposed usage (append `--json`) | Response schema |
| --- | --- | --- |
| `pbi-t3-compiler-completeness-1qi.11` | `report compose --schema S [--rows R] --intent I [--style STYLE] (--dry-run \| --out-dir DIR)` | `powerbi-cli.report.compose.v1` |
| `pbi-t8-batch-ops-o0z.1` | `ops apply --project P --ops FILE (--dry-run \| --out-dir DIR \| --in-place)` | `powerbi-cli.ops.apply.v1` |
| `pbi-t8-batch-ops-o0z.2` | `ops diff BEFORE AFTER` | `powerbi-cli.ops.diff.v1` |
| `pbi-t8-batch-ops-o0z` | `ops schema` | `powerbi-cli.ops.schema.v1` (describes `powerbi-cli.ops.v1`) |
| `pbi-t2-dashboard-spec-v2-dsd.2` | Existing `report spec explain --schema S [--profile P] --spec FILE` | `powerbi-cli.report.spec.explain.v1`; nested `powerbi-cli.report.spec.explainPlan.v1` |
| `pbi-t2-dashboard-spec-v2-dsd.5` | `report spec extract --project P (--dry-run \| --out FILE)` | `powerbi-cli.report.spec.extract.v1` |
| `pbi-t2-dashboard-spec-v2-dsd.5` | `report spec diff BEFORE AFTER` | `powerbi-cli.report.spec.diff.v1` |
| `pbi-t6-planner-v2-szr.4` | `report plan explain --schema S [--profile P] (--intent I \| --objective TEXT)` | `powerbi-cli.report.plan.explain.v1` |
| `pbi-t5-design-system-mlf.6` | `report design-plan --project P --emit-ops [--out FILE]` | `powerbi-cli.report.designPlan.v1`; embedded/file plan `powerbi-cli.ops.v1` |
| `pbi-t9-desktop-oracle-rqn.7` | Existing `desktop harvest-reference --project P --visual HANDLE --out FILE [--desktop-version V] [--license-note TEXT] [--dry-run]` | existing `powerbi-cli.desktop.harvestReference.v1` |

Owner conformance criteria: common CliRun harness; leading/trailing JSON flags
on each real command; versioned response snapshots; focused capability bounds;
execute exact returned argv with spaced/apostrophe paths under the selected
shell contract; deterministic repeat output; every refusal has registered code,
nonempty hint, executable recovery and real RFC 6901 pointer when input JSON
is involved; project mutations prove dry-run/source/out-dir/in-place isolation;
artifact writers prove no unintended source overwrite; general replay proves
deserialize/serialize and CLI/kernel byte parity. Compose must retain the
original schema/rows/profile/intent/style inputs in rerun commands and expose
proof plans without claiming Desktop proof on Linux.
