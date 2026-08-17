# `src/main.rs` isomorphic split notes

## Baseline

- Baseline commit: `229c117`.
- `cargo test --release --no-fail-fast`: 24 suite results; 436 passed, 0 failed, 4 ignored. Exact per-suite pass counts: `163, 1, 30, 12, 14, 3, 6, 8, 13, 3, 5, 0, 7, 7, 3, 11, 21, 5, 3, 23, 86, 4, 3, 5`; ignored counts are `2` in the unit binary, `1` in `microsoft_integrations`, and `1` in `microsoft_report_exact`.
- `cargo clippy --release --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean.
- `powerbi-cli.exe --json capabilities`: 244,512 bytes, SHA-256 `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`, empty stderr.
- Warm `cargo build --release` baseline: five runs `0.243s, 0.178s, 0.179s, 0.176s, 0.201s`; median `0.179s`. The corrected compile gate permits a delta of `max(10%, 2 seconds absolute)`, so the effective upper ceiling is `2.179s`.
- Pre-existing timing note: initial cold/default and diagnostic serialized suite attempts exposed intermittent Windows process-marker timeouts in two `mcp` tests and one `microsoft` test. Each passed in isolation (the Microsoft test passed 5/5 after warm-up), and the accepted untouched exact-command baseline was fully green under default settings. No source was changed to mask this host-level flakiness.

## Family map

| Family | Destination | Symbols / responsibility | Status |
|---|---|---|---|
| Inspect command orchestration | existing `src/inspect.rs` | `inspect_command`, `parse_inspect_args` | extracted; full gate green under corrected compile threshold |
| Native PBIP/PBIR/TMDL validation | new `src/validation.rs` | validation command/backend, project checks, `ValidationReport` | extracted; full gate green |
| Shared JSON input | new `src/json_io.rs` | `read_json_value` | extracted; full gate green apart from documented default-thread host flakes |
| Dashboard visual scaffolding and binding adaptation | new `src/dashboard_scaffold.rs` | page/visual schema records, effective pages, visual JSON, scaffold binding resolution | extracted; full gate green |
| Schema-driven project scaffolding | new `src/scaffold.rs` | scaffold command/spec validation, cleanup, PBIP/PBIR/TMDL/M emission, deterministic names and writers | extracted; full gate green |

The scaffold-specific binding adapter stays with dashboard scaffolding: it has one caller (`visual_json`) and directly depends on the scaffold manifest's private binding and column types. Moving it into general `pbir_bindings.rs` would create a reverse dependency from general PBIR plumbing back to the scaffold schema.

## Corrected seam dispositions

### Inspect command orchestration — confirmed

- Commit: `54f0fab` (`refactor: extract inspect command from main (isomorphic)`).
- Mechanical change: moved `inspect_command` and `parse_inspect_args` into the existing focused `src/inspect.rs`; retained the crate-root import path through `pub(crate) use inspect::inspect_command`; the only necessary body adjustment was `inspect::deep_inspect` to the same-module `deep_inspect` path.
- Behavior gate: an exact-command warm run produced all 24 green suite results with the baseline vector (436 passed, 4 ignored). Cold runs reproduced only the three process-timing flakes already observed on untouched baseline; no new failure or count drift appeared.
- Quality and golden gate: clippy clean, rustfmt clean, and capabilities remained exactly 244,512 bytes with SHA-256 `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9` and empty stderr.
- Compile gate: the first five-run warm build sample had median `0.245s`. A separate 15-run stabilization sample was `0.250, 0.225, 0.248, 0.210, 0.222, 0.233, 0.240, 0.224, 0.226, 0.220, 0.231, 0.213, 0.211, 0.230, 0.243s`, median `0.226s` and trimmed mean `0.228s`. The stabilized median is only `0.047s` above the `0.179s` baseline and is therefore within the corrected `2.179s` ceiling.
- Disposition: `SEAM_CONFIRMED`. The earlier refutation was solely a misapplication of the superseded literal +10% threshold; no behavior, quality, golden-output, or meaningful compile-time drift was observed.

## Refuted seams

None.

## Gate evidence

Gate results are recorded after each family commit. The accepted pass-count vector and capability hash must remain identical to the baseline above; build timing uses the median of five warm invocations to control sub-second scheduler noise.

| Family commit | Tests | Clippy | Fmt | Capabilities | Warm build | Verdict |
|---|---|---|---|---|---|---|
| `54f0fab` inspect command | 24 suites; 436 passed; 4 ignored (green warm exact-command run) | clean | clean | byte-identical; baseline SHA-256 | `0.245s` first median; `0.226s` stabilized median vs corrected `2.179s` ceiling | green; accepted |
| `refactor: extract shared JSON input from main (isomorphic)` | 24 suites; 436 passed; 4 ignored with `RUST_TEST_THREADS=1`; repeated default-thread exact-command runs reproduced only the three baseline-listed Windows child-process flakes, which each passed in isolation | clean | clean | 244,512 bytes; SHA-256 `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`; empty stderr | five runs `0.354, 0.245, 0.254, 0.233, 0.238s`; median `0.245s` vs corrected `2.179s` ceiling | green; host flakes match baseline ledger |
| `refactor: extract native validation from main (isomorphic)` | exact default-thread command green; 24 suites; 436 passed; 4 ignored; exact baseline vector | clean | clean | 244,512 bytes; SHA-256 `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`; empty stderr | five runs `0.250, 0.185, 0.190, 0.184, 0.190s`; median `0.190s` vs corrected `2.179s` ceiling | green; validation body SHA-256 identical before/after (`0c4ea5945b398657fb191100157f23653f0cd4c88172acafd6603ce77b0daeb4`) |
| `refactor: extract dashboard scaffolding from main (isomorphic)` | exact default-thread command green; 24 suites; 436 passed; 4 ignored; exact baseline vector | clean | clean | 244,512 bytes; SHA-256 `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`; empty stderr | five runs `0.247, 0.178, 0.185, 0.177, 0.184s`; median `0.184s` vs corrected `2.179s` ceiling | green; normalized source blocks identical before/after (types SHA-256 `f69bdac31d1da4c39c1aac394b362e0ad1d3abd3024a5e35ad83fa6dd4d0045b`, functions SHA-256 `c4dfbf3f11f2582bb74353acc9f19ac22bf6cb56d1da9d8eae1faeab46bbb4e6`) |
| `refactor: extract schema-driven scaffolding from main (isomorphic)` | exact default-thread command green; 24 suites; 436 passed; 4 ignored; exact baseline vector | clean | clean | 244,512 bytes; SHA-256 `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`; empty stderr | five runs `0.258, 0.186, 0.184, 0.182, 0.184s`; median `0.184s` vs corrected `2.179s` ceiling | green; normalized source blocks identical before/after (constants SHA-256 `a916075a3e8b862dd70ec8b31a19e4e1344ca32661bd99fe3dfe9c26c806769b`, types SHA-256 `8e87060e04d570ee4384574858af4603704a1f949e099119085d8a5533f4482d`, functions/tests SHA-256 `20c3b0cd1cd48e6531b2caef551023deb2a93829dbd9352e91da585bb2171c20`) |

Validation extraction note: removing the validation block made the existing `m_literal_tests` module visible to Clippy's `items_after_test_module` lint. The unchanged test module was mechanically relocated to the end of `main.rs`, then moved with the scaffold family into `src/scaffold.rs`. No lint suppression or test-body edit was introduced.

## Final shape

- `src/main.rs`: 113 physical lines, containing crate attributes, module declarations, compatibility re-exports, and `fn main` only (down from the 3,370-line work-order baseline).
- New focused modules: `src/json_io.rs`, `src/validation.rs`, `src/dashboard_scaffold.rs`, and `src/scaffold.rs`; inspect orchestration moved into existing `src/inspect.rs`.
- Refuted seams: none. All five mapped families were extracted and accepted.
