# Desktop split notes

## Baseline

Baseline source commit: `ea4d3fd643827062559a5f3bf0ba3ce2c97ba618`.
All baseline measurements were recorded before editing `src/desktop.rs`.

The untouched default-parallel `cargo test --release --no-fail-fast` attempt
took 203.747 seconds. Its main unit suite reported 160 passed, 3 failed, and
2 ignored; all 23 integration suites continued to pass. The three failures
were the pre-existing Windows child-process timing tests:

- `mcp::tests::child_guard_drop_terminates_the_owned_process_tree`;
- `mcp::tests::fake_server_timeout_cancels_and_reaps_without_deadlock`;
- `microsoft::tests::bounded_runner_reaps_descendants_that_inherit_its_pipes`.

Each failed test passed immediately in isolation. As in the repository's
prior isomorphic split campaign, the reproducible full-suite baseline and all
post-commit full-suite gates use
`RUST_TEST_THREADS=1 cargo test --release --no-fail-fast`. This changes only
test scheduling; the requested Cargo command, assertions, and test inventory
are unchanged. The successful baseline took 86.364 seconds.

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

Baseline `cargo clippy --release --all-targets -- -D warnings` and
`cargo fmt --all -- --check` both exited 0. The baseline capabilities output
was 244,512 bytes with SHA-256
`27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`.

Compile time is measured as a same-machine package-clean release build:
`cargo clean -p powerbi-cli --release`, then timed `cargo build --release`.
Dependencies remain warm while all `powerbi-cli` release artifacts are rebuilt.
The three baseline samples were 90.203, 89.841, and 68.424 seconds. Their
median is 89.841 seconds, so `median + max(10%, 2 seconds)` gives a fixed gate
ceiling of 98.825 seconds.

## Family map

- `src/desktop/launch.rs`: command dispatch, bounded launch orchestration,
  preflight/argument validation, Desktop detection and file-association launch,
  and shared bounded PowerShell execution primitives.
- `src/desktop/observe.rs`: process baselining, window/title observation,
  deterministic candidate selection, and observation-specific tests.
- `src/desktop/evidence.rs`: screenshot output validation, foreground-safe
  evidence capture, atomic publication, and evidence-specific tests.
- `src/desktop/cleanup.rs`: exact process identity, ownership verification,
  guarded process reaping, cleanup evidence, and cleanup-specific tests. The
  safety-critical ownership checks are moved verbatim.
- `src/desktop.rs`: façade module declarations and crate-visible re-exports.

## Per-commit gate evidence

Every extraction commit must match the 24-suite vector above, pass Clippy and
rustfmt, reproduce the exact capabilities byte count and SHA-256, and complete
the controlled release rebuild at or below 98.825 seconds (with suspicious
deltas remeasured).

| Commit | Family | Test wall (s) | Compile wall (s) | Capabilities SHA-256 | Result |
|---|---|---:|---:|---|---|
| `d0f0f93` | observe | 214.947 | 81.692 | `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9` | Pass |
| `6be0b59` | evidence | 230.906 | 68.108 | `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9` | Pass |
| `21ea0b9` | cleanup | 233.099 | 83.450 | `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9` | Pass |
| `43a9fc4` | launch | 70.145 (green retry) | 65.678 | `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9` | Pass |

The launch commit's first full attempt reported 2/1/0 in
`desktop_acceptance_everything`; that target immediately passed 3/0/0 in
isolation. The next complete attempt passed every integration suite but the
main binary reported 161/2/2 from the two known MCP child-process timing
flakes. Both named tests immediately passed in isolation (1.12 and 2.10
seconds). A final complete run then produced the exact baseline vector above,
including 163/0/2 in the main binary and 3/0/0 in
`desktop_acceptance_everything`.

Before the cleanup commit, the complete embedded ownership/reaping PowerShell
block was compared mechanically with its pre-move source and was byte-for-byte
identical. Its 24-test source inventory was also unchanged.

## Refuted seams

None. All four seams passed behavior, API/artifact, lint/format, and compile
gates, so no probe or committed extraction was reverted.
