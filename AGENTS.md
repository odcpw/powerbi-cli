# AGENTS.md — law for every agent working in this repository

Read this file completely before touching anything. README.md, goal.md,
skills/powerbi-cli/SKILL.md and docs/roadmap.md are the product context;
docs/bridge-plan-2026-09.md is the current plan; `.beads/issues.jsonl` is the
backlog (beads).

## 1. What this project is

`powerbi-cli` is a Rust CLI that lets AI agents author offline-safe Power BI
projects (PBIP folder, PBIR report JSON, TMDL semantic model) from a schema
manifest, without real data or credentials, so the project can be carried to a
locked-down machine, rebound to corporate sources and refreshed in Power BI
Desktop. The product thesis (goal.md) is a compiler/workbench that builds ANY
dashboard from schema + profile + intent, data-domain agnostic, agent-first.

## 2. Product non-negotiables (never traded away)

- Supported features emit real PBIR/TMDL. Unproven features return
  `error.code = unsupported_feature` and never write guessed JSON. No fake
  fallbacks, no stubs, no `todo!()`, no "TODO implement".
- Power BI Desktop is the compatibility oracle. Proof levels are honest:
  `unit-smoke < schema-golden < desktop-golden-pending <
  manual-desktop-canvas-refresh < desktop-canvas-refresh`. Work done on Linux
  cannot claim a Desktop-proven level.
- Every mutation supports `--dry-run | --out-dir | --in-place` (guarded) and
  returns readback / validate / next commands.
- JSON on stdout, diagnostics on stderr. Stable handles: `page:<Name>`,
  `visual:<Page>:<Container>`, `filter:...`; measure handles percent-encode
  `%` and `:`.
- Offline safety: never introduce credentials, caches, `.pbix`, localSettings
  or real data rows, not even in tests or fixtures.
- No monolith: focused modules with tests per module. Follow an existing
  command end to end (clap definition, dispatcher, module, contract catalog in
  `src/contract/*.rs`, `src/feature_catalog.rs`, tests) before adding one.
  Reuse `src/cli_support.rs`.
- Every new or renamed command path MUST be invoked in
  `tests/desktop_acceptance_everything.rs` (the everything-acceptance harness
  asserts that every advertised capability is exercised); run
  `cargo test --test desktop_acceptance_everything` before committing a
  command change.
- Docs change in the same commit as behavior: README.md,
  skills/powerbi-cli/SKILL.md, the capabilities catalog, the feature catalog,
  docs/roadmap.md as applicable. Keep doc edits localized (insert small blocks
  next to related content, no reflow or restructuring) because many branches
  merge in parallel.

## 3. Quality gates

The merged tree must pass all four, in this order:

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
git diff --check
```

Never weaken a test, validator, lint or gate to make something pass. Gate or
test changes must be their own commit with an explanation. If an unrelated
pre-existing test fails, report it, do not "fix" it.

## 4. Swarm operating mode (code-first, batch-verify)

Many agents work in parallel on an eight-core machine with one expensive
build. Builds are the bottleneck, so agents write code and the orchestrator
verifies centrally.

- You work ONLY in the worktree/branch you were launched in. Never edit the
  main checkout at /home/oliver/Projects/odcpw/powerbi-cli, never check out
  or merge another branch, never push, never open PRs.
- Toolchain: prepend `~/.cargo/bin` to PATH in every shell that runs cargo.
  Warm your build cache once with
  `[ -d target ] || cp -r /home/oliver/Projects/odcpw/powerbi-cli/target ./target`.
- Allowed build work in an agent pane: `cargo fmt`, `cargo check --all-targets`,
  and TARGETED tests (`cargo test --test <file> <filter>` or
  `cargo test --lib <module>::`). Do not run the full test suite or clippy over
  the whole workspace in an agent pane; the orchestrator runs the four gates
  over the merged tree and returns failures to the same agent for rework.
- Bead tracker: agents never run `br` or `bv` and never edit `.beads/`. The
  orchestrator assigns beads, verifies, and closes them with cited evidence.
  Your bead text is delivered as a file in your orders.
- Real code and real tests in the same bead. Tests cover the happy path, every
  refusal/diagnostic path, `--dry-run`/`--out-dir`/`--in-place`, JSON output
  shape, and determinism (same input, byte-identical output). Test names read
  as specifications.
- Do not split acceptance criteria into follow-up items to declare a bead
  done. If a criterion cannot be met, say so explicitly in your report.
- Commit early and often on your branch, one commit per bead when the bead is
  substantively complete, in this format:

```
<type>: <summary>  (bead <bead-id>)

<what and why, 2-6 lines>

Co-Authored-By: Codex <noreply@openai.com>
```

- When your orders are complete, write the final report file named in your
  orders, then stop. Do not pick new work on your own.

## 5. Honest credit

Process artifacts are not progress. Refusal-only work never completes a
positive-capability bead. A close without cited evidence (commit hash, test
names, gate run) is a defect. Commit count is not a metric. Report failures
plainly with their output.
