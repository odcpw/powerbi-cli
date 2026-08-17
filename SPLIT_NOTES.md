# Contract split notes

## Baseline

- Source revision: `229c117` (`src/contract.rs` unchanged; `WORK_ORDER.md` and
  `codex-run.log` were pre-existing untracked inputs and are excluded from all
  commits).
- Release build: `cargo clean --release` equivalent cold build followed by
  `cargo build --release`; 286.308 s.
- Full capabilities stdout: 244,512 bytes; SHA-256
  `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`.
- Focused capabilities stdout baselines:
  - `--for version`: 6,415 bytes;
    `a729807e69452e92676db32194b28ef5246dfdaf8ac421ddb1ef98a652274943`
  - `--for "workflow synthesize"`: 7,970 bytes;
    `a72b34b6446e8d4b4515ad63a95fb3fd7274b7931ccad6b1b9943a3881daaae1`
  - `--for "integrations status"`: 7,204 bytes;
    `e9ded9a06c3c405df75c42b6e77be1e1b13b8988bda89a7ab402afa4cfd37fdb`
  - `--for "desktop open-check"`: 8,181 bytes;
    `f3d0850c3c1d5c30aa728cf95b20b2004db1125f8d85b5ecc5257cfae113a361`
  - `--for "model measures show"`: 6,687 bytes;
    `62a242080a6aaf495acc22d7c57ec7032fcd2be8716fa9462e5cd913f86fb2dc`
  - `--for "report pages clone"`: 8,024 bytes;
    `dbe878bd6e5107f886dc4e6e1f6d28e047deaaf1be9d3a924ef34387f6f9d8b7`
- Deterministic full-suite command: `cargo test --all-targets --
  --test-threads=1`; 24 suites, 438 passed, 0 failed, 4 ignored. Exact
  passed/ignored counts by suite: unit 163/2; artifact_parity 1/0;
  cli_contract 30/0; cli_smoke 12/0; dashboard_build 14/0;
  desktop_acceptance_everything 3/0; desktop_bridge 7/0; diff 8/0;
  fixture 13/0; m_lint 3/0; microsoft_integrations 5/1;
  microsoft_report_exact 0/1; microsoft_report_validation 8/0;
  model_calculated_columns 7/0; model_columns 3/0; model_measures 11/0;
  model_partitions_handoff 21/0; model_relationships 5/0;
  model_static_tables 3/0; parity_tranche 23/0; report 86/0;
  report_authoring 4/0; visual_authoring_goldens 3/0;
  workflow_synthesize 5/0.
- `cargo clippy --all-targets -- -D warnings`: pass.
- `cargo fmt --all -- --check`: pass.

The default parallel test runner reproducibly failed three unrelated bounded
Windows child-process timing tests before any source edit (160 passed, 3
failed, 2 ignored in the unit suite on two attempts). All three pass with one
test thread. The serialized command above therefore fixes the baseline and is
used unchanged after every extraction commit; it still executes every test in
all 24 suites.

## Extracted families

Gate evidence is appended to this section after each family extraction.

### `report`

- Moved the 69 contiguous `report ...` command descriptors, from `report
  build` through `report visuals set-bindings`, to `src/contract/report.rs`.
  The façade inserts the returned vector at the original position between
  `source-template apply` and `handoff check`.
- Post-commit gate: pass. The first serialized attempt hit the pre-existing
  `mcp::tests::child_guard_drop_terminates_the_owned_process_tree` marker
  timeout (162 passed, 1 failed, 2 ignored in the unit suite); its log was
  preserved and the identical full gate was rerun. The complete retry matched
  the baseline exactly: 24 suites, 438 passed, 0 failed, 4 ignored; clippy and
  fmt passed; clean release build 132.176 s (46.2% of the 286.308 s baseline,
  no regression); full capabilities SHA-256
  `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`
  at 244,512 bytes. All six focused payloads also matched their baseline
  byte lengths and SHA-256 values.

### `model`

- Moved the 33 contiguous `model ...` command descriptors, from `model tables
  add-static` through `model expressions show`, to `src/contract/model.rs`.
  The façade inserts the returned vector at the original position between
  `diff` and `source-template list`.
- Post-commit gate: pass. Exact baseline test vector (24 suites, 438 passed,
  0 failed, 4 ignored); clippy and fmt passed; clean release build 113.429 s
  (39.6% of baseline, no regression); full capabilities remained 244,512
  bytes with SHA-256
  `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`.
  All six focused payloads also matched their baseline byte lengths and
  SHA-256 values.

### `desktop`

- Moved the eight contiguous Desktop descriptors, from `desktop open`
  through `desktop bridge screenshot-all`, to `src/contract/desktop.rs`.
  The façade inserts the returned vector at the original position between
  `skill install` and `fixture normalize`.
- Post-commit gate: pass. Exact baseline test vector (24 suites, 438 passed,
  0 failed, 4 ignored); clippy and fmt passed; clean release build 148.266 s
  (51.8% of baseline, no regression); full capabilities remained 244,512
  bytes with SHA-256
  `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`.
  All six focused payloads also matched their baseline byte lengths and
  SHA-256 values.

### `integrations`

- Moved the four contiguous optional-tooling descriptors (`integrations
  status`, `integrations install`, `skill status`, and `skill install`) to
  `src/contract/integrations.rs`. The façade inserts the returned vector at
  the original position between `workflow verify` and `desktop open`.
- Post-commit gate: pass. Exact baseline test vector (24 suites, 438 passed,
  0 failed, 4 ignored); clippy and fmt passed; clean release build 123.138 s
  (43.0% of baseline, no regression); full capabilities remained 244,512
  bytes with SHA-256
  `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`.
  All six focused payloads also matched their baseline byte lengths and
  SHA-256 values.

### `workflow_pkg`

- Moved three order-sensitive chunks to `src/contract/workflow_pkg.rs`: five
  `package ...` descriptors, four `workflow ...` descriptors, and four
  `source-template ...` descriptors. Separate builders let the façade retain
  the original intervening core, integrations, Desktop, project, and model
  entries without reordering any command.
- Post-commit gate: pass. Exact baseline test vector (24 suites, 438 passed,
  0 failed, 4 ignored); clippy and fmt passed; clean release build 110.373 s
  (38.6% of baseline, no regression); full capabilities remained 244,512
  bytes with SHA-256
  `27b2564e0f382ced6598fea3cd3bfbeb000e961b341d3de6bdfe245c34de8fa9`.
  All six focused payloads also matched their baseline byte lengths and
  SHA-256 values.

## Refuted seams

None.
