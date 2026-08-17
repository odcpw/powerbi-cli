# Workflow split notes

## Result

`src/workflow.rs` remains the façade and declares `plan`, `run`, `shared`,
and `verify`. Existing crate-visible paths are preserved with façade
re-exports.

- `src/workflow/plan.rs`: workflow plan command, options, and argument parser.
- `src/workflow/run.rs`: workflow run command, argument parser, and run-only
  staged-copy orchestration.
- `src/workflow/verify.rs`: workflow verify command and argument parser.
- `src/workflow/shared.rs`: staged-model guards, tree hashing, plan/receipt
  types, integrity validation, bounded I/O, and path/capability helpers shared
  by command families or external workflow consumers.
- `src/workflow.rs`: façade wiring/re-exports, the synthesize family whose
  module seam was refuted, the source-relative integration-lock macro anchor,
  and the original façade-level unit tests.

Every new module begins with a `//!` module comment. Unit test names and the
24-suite inventory are unchanged.

## Gate protocol and baseline

Baseline source commit: `229c117e88770f95945220e560584ccc569981b4`.
The reverted source tree at `956fc78` has the identical Git tree
`fcadf7351520904d5f839c034bf67fef465d9cf5` and was used to obtain a
same-worktree compile baseline.

The untouched default-parallel baseline exposed pre-existing Windows
child-process timing instability:

- full attempt 1: main unit suite 159 passed, 4 failed, 2 ignored;
- full attempt 2: main unit suite 162 passed, 1 failed, 2 ignored;
- full attempt 3: main unit suite 159 passed, 4 failed, 2 ignored;
- five additional main-suite samples produced either 160/3/2 or 161/2/2;
- each affected test passed in isolation.

The oscillating tests were
`mcp::tests::fake_server_timeout_cancels_and_reaps_without_deadlock`,
`mcp::tests::child_guard_drop_terminates_the_owned_process_tree`,
`microsoft::tests::bounded_runner_reaps_descendants_that_inherit_its_pipes`,
and
`mcp::tests::fake_server_handshake_handles_fragmentation_notifications_and_stderr_flood`.
All are outside `workflow` and use hard Windows child-process deadlines.

To make the requested full gate reproducible without changing its Cargo
command or assertions, baseline and post-commit test runs used
`RUST_TEST_THREADS=1 cargo test --release --no-fail-fast`. The successful
baseline took 85.299 seconds.

Exact successful pass-count vector (suite order emitted by Cargo):

| Suite | Passed | Failed | Ignored |
|---|---:|---:|---:|
| `unittests src/main.rs` | 163 | 0 | 2 |
| `artifact_parity` | 1 | 0 | 0 |
| `cli_contract` | 30 | 0 | 0 |
| `cli_smoke` | 12 | 0 | 0 |
| `dashboard_build` | 14 | 0 | 0 |
| `desktop_acceptance_everything` | 3 | 0 | 0 |
| `desktop_bridge` | 6 | 0 | 0 |
| `diff` | 8 | 0 | 0 |
| `fixture` | 13 | 0 | 0 |
| `m_lint` | 3 | 0 | 0 |
| `microsoft_integrations` | 5 | 0 | 1 |
| `microsoft_report_exact` | 0 | 0 | 1 |
| `microsoft_report_validation` | 7 | 0 | 0 |
| `model_calculated_columns` | 7 | 0 | 0 |
| `model_columns` | 3 | 0 | 0 |
| `model_measures` | 11 | 0 | 0 |
| `model_partitions_handoff` | 21 | 0 | 0 |
| `model_relationships` | 5 | 0 | 0 |
| `model_static_tables` | 3 | 0 | 0 |
| `parity_tranche` | 23 | 0 | 0 |
| `report` | 86 | 0 | 0 |
| `report_authoring` | 4 | 0 | 0 |
| `visual_authoring_goldens` | 3 | 0 | 0 |
| `workflow_synthesize` | 5 | 0 | 0 |
| **Total** | **436** | **0** | **4** |

Baseline clippy and format checks were clean. Baseline capabilities SHA-256:
`27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`.

The initial warmed no-op `cargo build --release` sample was 0.258 seconds,
which proved too dominated by Cargo/process startup noise to support a 10%
comparison. The compile gate was therefore measured as a full package rebuild:
the verified local `target` directory retained warmed dependency artifacts,
`cargo clean -p powerbi-cli --release` removed only this package's
regenerable release artifacts, and `cargo build --release` was timed on the
same machine. The byte-identical in-place baseline was 183.804 seconds, so the
10% ceiling was 202.184 seconds.

## Per-commit gate evidence

Every successful row below matched the exact 24-suite vector above, had
`cargo clippy --release --all-targets -- -D warnings` exit 0,
`cargo fmt --all -- --check` exit 0, and produced the baseline capabilities
SHA-256.

| Commit | Move | Test wall (s) | Compile wall (s) | Delta vs baseline | Result |
|---|---|---:|---:|---:|---|
| `b8e231a` | synthesize probe | 98.814 (green retry) | 206.066 repeat | +12.1% | **Refuted; reverted** |
| `956fc78` | revert synthesize probe | 249.435 | 183.804 | baseline | Pass |
| `26f2ae6` | verify | 188.327 | 164.468 | -10.5% | Pass |
| `df3bcc6` | plan | 207.487 | 143.812 | -21.8% | Pass |
| `773341d` | run | 243.066 | 155.882 | -15.2% | Pass |
| `1405ffb` | shared | 170.342 | 140.686 | -23.5% | Pass |

The synthesize probe's first controlled split build was 218.233 seconds
(+18.7%). A package-clean repeat was 206.066 seconds (+12.1%), still above the
202.184-second ceiling. Its tests, clippy, format, and capability hash were
otherwise identical. Per the work order, the family was stopped, its extraction
was reverted by `956fc78`, and the other families continued.

## Refuted seams and constraints

### Synthesize family: compile-performance seam refuted

The 798-line synthesize block is behaviorally/API-isomorphic as a child module,
but its repeat controlled release compile exceeded the allowed 10% regression.
It therefore remains intact in the façade. No alternate rewrite or semantic
change was attempted.

### Integration-lock constant: source-relative macro seam refuted

`INTEGRATION_LOCK_BYTES` uses
`include_bytes!("../integrations/microsoft/integration-lock.json")`.
Moving that statement into `src/workflow/shared.rs` changes the macro's source
directory and does not compile without rewriting the literal path. To honor the
verbatim-move/no-rewrite rule, this single macro anchor remains in the façade;
shared code accesses it through its parent module.

No shared mutable-state seam was found, and no helper was duplicated.
