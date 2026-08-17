# `src/main.rs` isomorphic split notes

## Baseline

- Baseline commit: `229c117`.
- `cargo test --release --no-fail-fast`: 24 suite results; 436 passed, 0 failed, 4 ignored. Exact per-suite pass counts: `163, 1, 30, 12, 14, 3, 6, 8, 13, 3, 5, 0, 7, 7, 3, 11, 21, 5, 3, 23, 86, 4, 3, 5`; ignored counts are `2` in the unit binary, `1` in `microsoft_integrations`, and `1` in `microsoft_report_exact`.
- `cargo clippy --release --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean.
- `powerbi-cli.exe --json capabilities`: 244,512 bytes, SHA-256 `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`, empty stderr.
- Warm `cargo build --release` baseline: five runs `0.243s, 0.178s, 0.179s, 0.176s, 0.201s`; median `0.179s`; +10% ceiling `0.197s`.
- Pre-existing timing note: initial cold/default and diagnostic serialized suite attempts exposed intermittent Windows process-marker timeouts in two `mcp` tests and one `microsoft` test. Each passed in isolation (the Microsoft test passed 5/5 after warm-up), and the accepted untouched exact-command baseline was fully green under default settings. No source was changed to mask this host-level flakiness.

## Family map

| Family | Destination | Symbols / responsibility | Status |
|---|---|---|---|
| Inspect command orchestration | existing `src/inspect.rs` | `inspect_command`, `parse_inspect_args` | refuted by build-time gate; red commit retained as evidence |
| Native PBIP/PBIR/TMDL validation | new `src/validation.rs` | validation command/backend, project checks, `ValidationReport` | pending |
| Shared JSON input | new `src/json_io.rs` | `read_json_value` | pending |
| Dashboard visual scaffolding and binding adaptation | new `src/dashboard_scaffold.rs` | page/visual schema records, effective pages, visual JSON, scaffold binding resolution | pending |
| Schema-driven project scaffolding | new `src/scaffold.rs` | scaffold command/spec validation, cleanup, PBIP/PBIR/TMDL/M emission, deterministic names and writers | pending |

The scaffold-specific binding adapter stays with dashboard scaffolding: it has one caller (`visual_json`) and directly depends on the scaffold manifest's private binding and column types. Moving it into general `pbir_bindings.rs` would create a reverse dependency from general PBIR plumbing back to the scaffold schema.

## Refuted seams

### Inspect command orchestration — refuted

- Commit attempted: `4936228` (`refactor: extract inspect command from main (isomorphic)`).
- Mechanical change: moved `inspect_command` and `parse_inspect_args` into the existing focused `src/inspect.rs`; retained the crate-root import path through `pub(crate) use inspect::inspect_command`; the only necessary body adjustment was `inspect::deep_inspect` to the same-module `deep_inspect` path.
- Behavior gate: an exact-command warm run produced all 24 green suite results with the baseline vector (436 passed, 4 ignored). Cold runs reproduced only the three process-timing flakes already observed on untouched baseline; no new failure or count drift appeared.
- Quality and golden gate: clippy clean, rustfmt clean, and capabilities remained exactly 244,512 bytes with SHA-256 `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9` and empty stderr.
- Refutation: the first five-run warm build sample had median `0.245s`. A separate 15-run stabilization sample was `0.250, 0.225, 0.248, 0.210, 0.222, 0.233, 0.240, 0.224, 0.226, 0.220, 0.231, 0.213, 0.211, 0.230, 0.243s`, median `0.226s` and trimmed mean `0.228s`. The median is 26.3% above the `0.179s` baseline and exceeds its literal +10% ceiling of `0.197s`.
- Disposition: `SEAM_REFUTED`. Per the work order, extraction work stopped immediately; the remaining families were not attempted.

## Gate evidence

Gate results are recorded after each family commit. The accepted pass-count vector and capability hash must remain identical to the baseline above; build timing uses the median of five warm invocations to control sub-second scheduler noise.

| Family commit | Tests | Clippy | Fmt | Capabilities | Warm build | Verdict |
|---|---|---|---|---|---|---|
| `4936228` inspect command | 24 suites; 436 passed; 4 ignored (green warm exact-command run) | clean | clean | byte-identical; baseline SHA-256 | `0.245s` first median; `0.226s` stabilized median vs `0.197s` ceiling | refuted; stop |
