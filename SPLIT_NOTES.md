# MCP split notes

## Result and family map

`src/mcp.rs` remains the façade so every existing `crate::mcp::X` path keeps
resolving. The dependency-directed extraction order was:

- `src/mcp/staged.rs`: staged-model replacement preparation, execution,
  readback/materialization proof, fingerprints, and exact model-tool payloads.
- `src/mcp/cleanup.rs`: bounded stdio pumps, child/process-tree monitoring,
  cleanup reports, and join/termination handling.
- `src/mcp/client.rs`: MCP session/protocol plumbing, handshake, operations,
  closed tool policy, protocol normalization/validation, and the remaining
  client/process integration unit tests.
- `src/mcp/staged.rs` also contains the staged subject tests, including exact
  call-order and byte-identical materialization assertions.
- `src/mcp.rs`: a 45-line façade containing shared import plumbing, module
  declarations, and crate-visible re-exports.

Each production family was moved mechanically behind façade imports with
only the minimum parent/child visibility needed. Every new file begins with a
`//!` module comment. A direct baseline-versus-final inventory found all 29
MCP unit-test names identical.

## Gate protocol and baseline

Baseline source commit: `ea4d3fd643827062559a5f3bf0ba3ce2c97ba618`.
The worktree contained the user-supplied untracked `WORK_ORDER.md` and empty
`codex-run.log`; both are preserved untouched.

The untouched baseline passed the exact requested command
`cargo test --release --no-fail-fast` in 174.217 seconds. Exact successful
pass-count vector (suite order emitted by Cargo):

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
`cargo fmt --all -- --check` were clean. Baseline capabilities output was
244,512 bytes with SHA-256
`27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`;
the raw bytes are retained outside the worktree for direct equality checks.

Compile timing uses a warmed dependency target and, for each sample,
`cargo clean -p powerbi-cli --release` followed by timed
`cargo build --release` on the same machine. Baseline samples were 70.684,
90.713, and 92.299 seconds. The median is 90.713 seconds, so the required
`max(10%, 2 seconds absolute)` ceiling is 99.784 seconds.

## Per-commit gate evidence

Every passing row below matched the exact 24-suite vector above, had
`cargo clippy --release --all-targets -- -D warnings` exit 0,
`cargo fmt --all -- --check` exit 0, and produced a 244,512-byte capabilities
file that was directly byte-compared equal to the baseline (not hash-only).

| Commit | Move | Test wall (s) | Compile wall (s) | Delta vs median baseline | Result |
|---|---|---:|---:|---:|---|
| `68552da` | staged model + subject tests | 165.400 | 78.044 | -14.0% | Pass |
| `249fdbf` | cleanup/process handling | 119.419 | 77.187 repeat | -14.9% | Pass |
| `1a96e4a` | client/protocol + remaining subject tests | 33.454 green retry | 91.130 repeat | +0.5% | Pass |

Before commit, the 1,231 production lines and 823 staged-test lines were
compared to their baseline source ranges and were text-identical.

The cleanup move was likewise text-identical after normalizing only the
required `pub(super)` markers and rustfmt's wrapping of one signature. Its
first post-commit attempt (`8cbf235`) stopped at test-target compilation:
`read_frames` and `capture_stderr` needed parent visibility for the preserved
façade tests. Those two minimum visibility changes were amended into
`249fdbf`, which then passed the full gate. The first controlled build of the
accepted hash was a suspicious 122.721 seconds (+35.3%); the required clean
repeat was 77.187 seconds (-14.9%) and is the accepted same-machine sample.

The client move's 1,198 production lines were text-identical after
normalizing only the required `pub(super)` markers and rustfmt's wrapping of
`exact_keys`; all 698 moved client/process test lines were text-identical.
The first full-suite run hit exactly the two documented host timing flakes:

- `mcp::client::tests::child_guard_drop_terminates_the_owned_process_tree`
  passed alone in 1.250 seconds;
- `mcp::client::tests::fake_server_timeout_cancels_and_reaps_without_deadlock`
  passed alone in 2.296 seconds.

The immediate full retry passed the exact 436/4 vector in 33.454 seconds.
The first controlled client-layout build was a suspicious 119.868 seconds;
the required clean repeat was 91.130 seconds (+0.5%), within the 99.784-second
ceiling. Capabilities remained directly byte-identical after every accepted
extraction commit.

## Refuted seams and constraints

No family seam was refuted. The only discovered boundary constraints were:

- cleanup helpers and state read by the parent/client needed the narrow
  `pub(super)` scope; the initial cleanup gate found the two test-only helpers
  described above before acceptance;
- client helpers used by staged or cleanup siblings likewise use
  `pub(super)`, while every pre-existing crate-visible item is re-exported by
  the façade;
- the two compile outliers were host-load noise proven by controlled clean
  repeats below the ceiling, not persistent seam regressions.
