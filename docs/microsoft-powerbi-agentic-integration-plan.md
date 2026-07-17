# Microsoft Power BI integration plan

Status: implemented and release-verified

Date: 2026-07-17
Plan schema: `powerbi-cli.integration-plan.v3`

## Outcome

`powerbi-cli` is one public Power BI authoring control plane:

- Rust owns deterministic PBIP/PBIR/TMDL work, source profiles, validation orchestration, packaging, evidence, and receipts.
- Microsoft's exact local tools provide the semantic-model, official report-validation, and Desktop capabilities that Microsoft uniquely controls.
- A downstream report repository consumes the public CLI; it is never embedded in public code or fixtures.

The product does not mirror every vendor command. It exposes a small typed workflow that is stable, inspectable, and useful to an agent.

## Product boundary

### Public repository

- Exact integration lock and explicit installer.
- Minimal local MCP client with a closed operation policy.
- Official report validation through `validate`.
- Desktop Bridge status, reload, and page screenshot support for an externally supplied exact PID.
- Deterministic `workflow plan`, `workflow run`, and `workflow verify`.
- Neutral conformance fixtures, public docs, doctor output, capabilities, and skill guidance.

### Consumer repositories

- Domain models, reports, source profiles, runtime assertions, screenshots, and release artifacts.
- Bugs and friction found by a consumer become neutral public reproductions before public fixes.
- A consumer never becomes a test dependency or fixture source for the public CLI.

## Microsoft toolchain

The committed integration lock is the only version authority. The current graph pins:

| Component | Version | Role |
|---|---:|---|
| `@microsoft/powerbi-modeling-mcp` | `0.5.0-beta.11` | local semantic-model engine |
| `@microsoft/powerbi-report-authoring-cli` | `0.1.4` | official report validator |
| `@microsoft/powerbi-desktop-bridge-cli` | `0.1.2` | Desktop process/page bridge |

The lock includes the complete npm graph, platform packages, integrity values, licenses, preview state, entrypoints, MCP protocol identity, server identity, and normalized tool-schema hash.

`integrations install --allow-network` is the only command that downloads tools. It installs the committed npm graph into a version-addressed private user cache, with npm checking every pinned package integrity, and publishes an atomic active receipt only after package identities and the complete installed-tree checksum verify. Normal commands resolve that cache and never run npm or npx.

The installed-tree checksum detects accidental drift and ordinary partial corruption. It is not a signature or an operating-system security boundary against a hostile process already running as the same user, which could rewrite the cache and both receipts (or replace the CLI executable itself).

## Local-process and data boundary

The Modeling MCP is a compiled Microsoft preview package; its public GitHub repository currently contains documentation and issue tracking, not the implementation source. `powerbi-cli` therefore treats it as a pinned black box and proves the consumed behavior through exact-package tests.

For the offline PBIP workflow:

- `powerbi-cli` starts the MCP executable on the same machine.
- Communication is newline-delimited JSON-RPC over piped stdin/stdout.
- The MCP receives the staged TMDL folder path and complete model definitions required by the typed operation.
- It does not refresh workbook/PostgreSQL data or receive database credentials.
- Export goes to a separate local workflow-owned directory.
- The process is shut down, reaped, and joined before native materialization begins.

The Microsoft package states that it may send usage information to Microsoft and exposes no telemetry-disable flag in the pinned version. A local stdio process is therefore not described as an OS-enforced zero-egress sandbox. Fabric connections are explicitly networked; offline PBIP and local Desktop workflows do not require Fabric.

## Small architecture

### `src/microsoft.rs`

- Parse and verify the committed integration lock.
- Install exact artifacts into the version-addressed private cache.
- Resolve only supported platform entrypoints.
- Launch one-shot Report CLI and Desktop Bridge commands with bounded output and an allowlisted environment.
- Normalize official diagnostics and readiness evidence.

### `src/mcp.rs`

- Own the bounded local JSON-RPC process lifecycle.
- Verify the exact MCP protocol, server identity, and tool-schema hash.
- Expose only typed operations used by the public workflow.
- Enforce exact request and response shapes.
- Own exact folder connection matching, model readback, fresh export, and child cleanup evidence.

### `src/workflow.rs`

- Parse and fingerprint one closed source profile.
- Resolve only the selected PBIP artifact closure.
- Create a new output stage and vendor-export quarantine.
- Apply exact partition source replacements.
- Prove source identity and the complete expected output tree.
- Run native and official validation.
- Write and verify a bounded receipt.

Existing report, model, Desktop, doctor, capability, feature, package, and project-I/O modules remain the public command families. No second CLI or generic workflow framework is introduced.

## Disposable offline model change

An offline model change has one implementation:

```text
capture source tree hash
  -> inspect and credential-scan complete M expressions
  -> connect exact staged definition folder
  -> require one canonical connection identity
  -> apply one or more typed partition updates
  -> read each changed partition back exactly
  -> export to a pre-created empty quarantine/definition directory
  -> verify the exported TMDL shape and bounds
  -> disconnect and terminate/reap/join the MCP process
  -> prove source and connected stage are still byte-identical
  -> materialize only the parsed partition source ranges natively
  -> require the complete stage tree to equal the precomputed expected hash
```

The known Microsoft `ExportToTmdlFolder` destination defect is contained by never passing a source SemanticModel root or an existing definition tree. Root-level TMDL, unexpected files, links/reparse points, path escape, missing core TMDL, schema drift, readback mismatch, timeout, or process-cleanup failure stops the workflow. Vendor output remains quarantined evidence and cannot become the stage.

The exact beta.11 conformance proof covers its observed `Update` success body (`{}`) and rejects alternate wrapper shapes.

## Source-profile workflow

### Profile

One versioned JSON profile contains:

- a stable profile ID;
- named resources with profile-relative or invocation-supplied paths and exact SHA-256 claims;
- typed `partition.replaceSource` entries;
- exact table and partition names;
- the expected current source hash;
- a complete M template file;
- one exact root connector: `Excel.Workbook` or `PostgreSQL.Database`;
- named resource placeholders used by that template.

Templates are complete M expressions under a narrow grammar. Excel requires one declared `File.Contents` resource; PostgreSQL requires none. Unknown/dynamic/computed-postfix calls and hard-coded file or URI paths are refused. Placeholders are closed and resource-specific; no arbitrary text substitution is available. Tracked profiles and canonical override/plan/output paths cannot contain credential-like assignment text or machine-specific absolute paths.

### `workflow plan`

- Requires a selected `.pbip`, profile, new plan path, and intended new output path.
- Resolves the `.pbip`, referenced Report, referenced SemanticModel, and registered resources only.
- Never treats a repository root as a recursive copy boundary.
- Fingerprints the selected closure, profile, templates, resources, integration lock, and policy.
- Writes only a new deterministic plan file.

### `workflow run`

- Requires the exact plan fingerprint confirmation.
- Recomputes every input fingerprint before creating the output.
- Copies only the selected artifact closure to a new output directory.
- Performs directory creation, create-only writes, copy readback, and marker removal relative to the opened output-directory capability, with no-follow component traversal; ambient pathname/FileId checks are publication checks only.
- Applies the disposable offline model change.
- Proves that only the named partition source ranges changed.
- Runs native and official report validation.
- Writes one new receipt containing hashes, exact backend versions, validation counts, cleanup evidence, and optional evidence paths/hashes.

The receipt never contains raw M/DAX payloads, query rows, credentials, access tokens, or embedded images. A failed output is marked and retained for diagnosis; it is never promoted over the source.

### `workflow verify`

- Reads the plan, output, receipt, and referenced evidence without mutation.
- Recomputes every claimed hash and validation result.
- Credential-scans every bounded evidence TMDL file and binds the complete evidence tree to a fresh canonical read-only MCP export in a private OS temporary directory.
- Detects output, receipt, unrelated-valid-TMDL injection, credential-comment injection, or evidence tampering even after self-hashes are resealed.

## Official report validation

`validate` supports:

- `--backend native` for deterministic offline validation;
- `--backend microsoft-report` for the exact installed official validator;
- `--backend all` to require both results.

Each backend has a versioned response envelope. Missing tools, unsupported platforms, official diagnostics, child failures, and timeouts remain distinct. Vendor output is bounded and redacted.

## Desktop Bridge

Desktop operations use exact PID, project path, and page identity:

- status is read-only;
- reload refuses unsaved state;
- clean screenshots require a saved exact project/page;
- dirty screenshots are diagnostic only;
- the Bridge subgroup never claims or terminates the externally supplied Desktop PID.

The Bridge receipt states what was directly observed. Click, drill, save, refresh, or interaction behavior is never inferred from a screenshot.

## Verification matrix

### Always-on

- `cargo fmt --check`
- `cargo check --locked --all-targets`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test --locked --all-targets --all-features`
- deterministic native/golden tests
- fake child protocol, timeout, truncation, schema-drift, and cleanup tests
- exact integration-lock/receipt identity tests

### Exact Microsoft packages

- install the committed graph;
- deep status and MCP initialize/tools-list identity;
- exact disposable offline partition replacement;
- valid and invalid official report-validation cases;
- source/stage/export hashes and child cleanup assertions.

### Windows owned-Desktop proof

- baseline Bridge status;
- open one workflow-owned project instance;
- exact path/PID claim;
- bounded DAX assertions;
- reload and page screenshots where the Bridge directly supports them;
- exact owned-PID cleanup.

### Release convergence

- fresh no-context reviewers inspect the acceptance contracts and final diff;
- simplification preserves proven behavior and removes accidental machinery;
- multi-pass bug hunt, UBS, full tests, secret/boundary scan, and `git diff --check` are clean;
- Beads graph has no open blocker or false closure;
- public and consumer repositories commit and push independently.

## Delivery graph

| Bead | Deliverable |
|---|---|
| `.1` | architecture and supply-chain baseline |
| `.2` | exact installer, cache, status, and doctor |
| `.3` | bounded local MCP session |
| `.4`, `.10`, `.11`, `.12` | disposable offline model isolation and exact TMDL correctness |
| `.5` | official report validator |
| `.6` | Desktop Bridge |
| `.7` | source-profile plan/run/verify workflow |
| `.8` | conformance, ergonomics, docs, and skill |
| `.9` | public release verification |

Close each bead only after its acceptance evidence is reproducible from the repository. Commit and push at coherent green checkpoints.
