# Integration Test Harness

The integration harness in `tests/common/` is the single process boundary for
invoking `powerbi-cli` from Rust tests. It keeps failures diagnosable from CI
logs, seeds tests from real archetype files, and centralizes snapshot and
performance conventions. It does not change the public CLI contract or claim
Power BI Desktop compatibility.

## CLI runs and structured logs

`run_powerbi(&[&str])` and `run_powerbi_owned(&[String])` return `CliRun` with
the exact `argv`, UTF-8-lossy `stdout` and `stderr`, `exit`, and `elapsed`.
Specialized tests use `cli_command(args)` to add a current directory, set or
remove environment variables, and then call `run()` or the byte-compatible
`output()` method. Performance tests that need a child resource bound can call
`run_with_peak_memory()`, which drains both output pipes while sampling the
child's resident set size. New tests should prefer `run()` unless they need
that explicit resource measurement.

Every invocation is retained for the current test thread. If an assertion
panics, the harness writes the complete records to stderr as JSON Lines. Set
`POWERBI_CLI_TEST_LOG=1` to print each record immediately, including successful
and intentionally rejected invocations:

```bash
POWERBI_CLI_TEST_LOG=1 cargo test --test e2e -- --nocapture
```

Each line has schema `powerbi-cli.test-run.v1` and fields `argv`, `stdout`,
`stderr`, `exit`, and `elapsedMs`. Output newlines are JSON-escaped, so each
record remains one grep-able physical line. Test fixtures must never place
credentials or real data in command output.

## Artifact parity and operation paths

`tests/common/artifact_tree.rs` owns the `powerbi-cli.artifact-tree.v1`
fingerprint used by `tests/artifact_parity.rs`, `tests/ops_equivalence.rs`, and
`tests/metamorphic.rs`. It sorts normalized relative paths, includes each path
length and file length in the SHA-256 input, and reports the two tree
fingerprints plus the first differing file (with lengths and digests only) when
a comparison fails. Keep this helper as the single implementation; do not add
a second WalkDir hash in a focused test.

`tests/ops_equivalence.rs` is a table of fixture, operation tag, and execution
rows. Each row builds two independent copies, runs the public CLI mutation into
one output directory, applies the serialized operation through the registered
transaction/kernel path into the other, and checks exit status, diagnostics,
JSON receipts, and the full artifact tree. The registry exposes its currently
compiled kernel tags to the runner, so adding a kernel without adding a row
fails the test. Add a new case by appending one OperationEquivalenceCase row;
the callback should keep CLI argv and operation JSON next to each other.

`tests/metamorphic.rs` uses the same table shape for
build(spec + X) == build(spec) then apply(X). A row authors the fragment with
DashboardSpecBuilder, builds both specs, applies the operation to the base
tree, and compares the generated PBIP/PBIR/TMDL artifacts. The
`powerbi-cli.manifest.copy.json` file is an immutable source-manifest sidecar, so
metamorphic comparisons explicitly exclude that one metadata path; changes to
generated report or model files still fail with the first-file diagnostic. T3
compiler beads append one row per newly compiled section and archetype.

Both runners use the shared CLI logger and direct-operation records, remain
offline, and use fresh temporary directories so a test cannot accidentally
mutate a checked-in fixture.

## Archetypes and spec builders

`load_archetype(name)` resolves the checked-in schema, profile, dashboard spec,
and normalized golden summary for `sales` and every fixture under
`examples/archetypes/`. `archetype_names()` is the closed inventory used by the
e2e loop. `ArchetypeFixture::build_into(path)` performs the canonical report
build without duplicating command assembly.

Use `spec_builder()` to mutate the fixture's current spec version or
`v2_spec_builder()` to author future `powerbi-cli.dashboard.v2` inputs. The
builder starts from checked-in JSON and supports focused visual, page-filter,
and style mutations. It authors test input only; selecting the v2 builder does
not imply that an unimplemented v2 compiler feature is supported.

## Offline e2e loop

`tests/e2e.rs` runs every archetype through schema validation, profile inference
and validation, report planning, planned and fixture spec validation, report
build, strict validation, handoff check, lint, triage, and golden fixture
verification. It uses embedded dummy rows only, never invokes Desktop or an
external service, and is suitable for Linux and Windows CI.

Run it alone with:

```bash
cargo test --test e2e
```

## JSON snapshots

`assert_json_snapshot(name, value)` recursively normalizes JSON object keys and
compares against `tests/snapshots/<name>.json`. Absolute POSIX and Windows paths
are rejected before comparison so machine-specific paths cannot enter contract
snapshots.

To accept an intentional change:

```bash
UPDATE_SNAPSHOTS=1 cargo test --test harness '<snapshot-test>'
git diff -- tests/snapshots/
```

Review every changed value. Snapshot updates are test-contract changes, not a
way to make unexplained failures pass.

## Nightly performance targets

`tests/perf.rs` contains ignored, deterministic Linux performance gates. The
scheduled workflow runs:

```bash
cargo test --locked --test perf -- --ignored
```

The first gate requires a generated 20-table, 10-page `report build` to finish
within three seconds. T2.4 owns the later 100-table schema with 50 `$include`
fragments target (under ten seconds and 512 MiB RSS); its test measures the
child process through `run_with_peak_memory()`. T3.11 owns the later
20-table/10-page `report compose` target (under three seconds). Those cases are
kept deterministic and ignored by default; there are no fake passing
placeholders in this harness.
