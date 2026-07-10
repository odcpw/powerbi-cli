# Agent-First Review Synthesis

Date: 2026-06-22

Inputs:

- `docs/reviews/agent-first-plan-review-prompt.md`
- `docs/reviews/claude-agent-first-review.md`
- `docs/reviews/grok-agent-first-review.md`

## Consensus

Both reviews agree on the same core diagnosis:

- The strategic niche is correct: cross-platform, offline-safe PBIP/PBIR/TMDL
  authoring with Power BI Desktop as the compatibility oracle.
- The current plan is too feature-taxonomy-heavy and not agent-first enough in
  its sequencing.
- The next work should prioritize agent contract, deep inspection, stable
  handles, validation/lint/proof, and a small set of high-value mutators.
- A broad future command catalog is less useful than five commands that return
  replayable proof commands and can survive agent retry loops.

## Decisions Accepted

1. Agent contract comes before feature breadth.

   `capabilities` must grow from a command list into a machine-readable command
   schema: flags, arguments, required outputs, stability, proof level, exit
   codes, diagnostics, examples, and next commands.

2. `--json` must be accepted anywhere.

   Agents will write both `powerbi-cli --json inspect project` and
   `powerbi-cli inspect project --json`. Both should work. The parser should
   strip global flags in a pre-pass before command dispatch.

3. Stable handles are mandatory.

   Pages, visuals, tables, columns, measures, relationships, partitions, and
   source templates need stable selectors returned by list/show/inspect and
   accepted by mutators. Agents should not infer PBIR/TMDL paths.

4. Every mutation returns follow-up commands.

   Mutation JSON should include `inspectCommand`, `validateCommand`,
   `readbackCommand`, and, when relevant, `desktopOpenCheckCommand` or
   `handoffCheckCommand`.

5. `inspect --deep`, `lint`, `wireframe`, and `handoff check` move earlier.

   Agents need structured state and spatial layout summaries before they can
   safely author pages and visuals. A raw PBIR folder is not an agent interface.

6. Desktop oracle moves into the first infrastructure wave.

   Desktop proof should not block every beta command, but visual binding and
   formatting claims must be backed by Desktop-authored golden fixtures and an
   opt-in Windows oracle harness.

7. Measures are the first real semantic mutator.

   After the agent contract and deep inspect surface exist, implement
   `model measures list/show/add/update/delete` before advanced model objects.

8. Visual binding expansion is frozen until a golden catalog exists.

   Current first-slice bindings are useful, but new visual families should not
   be added by memory. Add Desktop-authored fixtures and normalize summaries.

9. Source rebind and handoff are core, not appendix.

   `handoff check`, `handoff rebind-plan`, and source-template commands are the
   differentiator for the locked-down corporate workflow and should land before
   bookmarks, translations, or calculation groups.

10. The repo needs a canonical agent skill now.

    `ooxml-cli` works partly because agents have a local skill that tells them
    how to discover the live contract and run proof loops. `powerbi-cli` needs
    the same class of guide.

## Product Shape After Review

The revised product sequence is:

```text
agent contract
-> deep inspect and stable handles
-> strict validation, lint, handoff check, wireframe
-> Desktop oracle and golden visual catalog
-> measures, relationships, partitions/source templates
-> page/visual CRUD
-> themes/style and formatting
-> filters/slicers/interactions
-> batch ops
-> optional Desktop/Fabric bridges
```

## First Implementation Slices

1. **Agent contract hardening**

   Add `--json` anywhere, richer `capabilities`, structured diagnostic codes,
   output envelopes, generated follow-up commands, and the repo-local
   `skills/powerbi-cli/SKILL.md`.

2. **Snapshot lock and modularization**

   Add golden/snapshot coverage for current scaffold/inspect/validate output,
   then split the current monolith into CLI, output, manifest, project, TMDL,
   PBIR, inspect, validate, and scaffold modules.

3. **Deep inspect, handles, and wireframe**

   `inspect --deep` should return handles, model objects, page/visual layout,
   bindings, hazards, and proof status. `report wireframe export` should provide
   layout JSON/HTML without Desktop.

4. **Strict validation, lint, and handoff**

   Add Microsoft-schema-backed JSON validation where available, PBIR/TMDL
   reference checks, lint rule packs, `handoff check`, and `handoff rebind-plan`.

5. **Desktop oracle and first golden catalog**

   Add `desktop open-check`, `desktop save-check`, normalized Desktop fixture
   summaries, and a first visual catalog for card, table, line/bar chart, slicer,
   theme, and filter fixtures.

6. **Measures and semantic model mutations**

   Add `model measures list/show/add/update/delete`, then relationships and
   partition source-template operations. Every mutation supports `--dry-run` and
   returns proof commands.

## License Posture

The review confirms a pragmatic split:

- MIT/permissive repos can be studied and ported/reimplemented with attribution
  when that helps the Rust design.
- Python sidecars from MIT repos are technically allowed, but should not become
  the production path because they weaken the single-binary goal.
- AGPL/GPL/custom-license repos remain behavior-signal and test-inspiration
  sources unless the project explicitly accepts the license consequences.
- For quarantined projects, implement from Microsoft docs, our own Desktop
  fixtures, and behavior-level requirements, not copied source, JSON, examples,
  or prose.

This is not timidity. It is keeping the public Rust CLI usable and shippable
without accidental license surprises.

## Open Tensions

- Claude prioritized early measure mutation even before Desktop oracle. Grok
  prioritized oracle harness before typed mutation. The synthesis is: semantic
  model text mutations can ship as beta with strict offline validation, while
  visual binding/formatting requires Desktop oracle proof before stability.
- `repair` is valuable but should follow stable diagnostics. Add
  `repair --dry-run` once validation emits precise machine-readable diagnostic
  codes.
- The command surface should stay broad in design but narrow in implementation.
  Do not ship 80 commands; ship the smallest agent-trustworthy path first.
