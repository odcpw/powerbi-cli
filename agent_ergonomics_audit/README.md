# Agent Ergonomics Audit — 2026-07-17

This audit records a recursive learning pass performed while building and
Desktop-testing a six-page occupational-accident Power BI report. It focuses on
friction observed through the real CLI → PBIP → Desktop workflow.

The shipped improvement batch is intentionally narrow and evidence-backed:

1. Canonicalize scatter color grouping to PBIR `Series`.
2. Preserve `legend`, `color`, and `colour` as friendly input aliases.
3. Reject stale/unsupported raw query-state roles with a canonical repair.
4. Detect scalar `IF()` variables used as tables in common DAX consumers.
5. Include that DAX rule in `validate --strict` through existing lint plumbing.
6. Diagnose empty visual directories before deep inspection.
7. Update the scatter archetype, catalog, capability contract, and proof notes.
8. Add focused positive and negative regression tests.
9. Add a reusable Desktop runtime regression reference to the powerbi-cli
   skill.
10. Feed hardened validation back into the report and add the proven optional
    Tooltips role for Category/Y charts when it exposed that catalog omission.
11. Replace manual/UI DAX query entry with `model dax execute`, a bounded,
    explicitly opted-in read-only bridge to the exact already-open Desktop
    model.
12. Freeze the bridge's refusal, privacy, timeout, row/cell cap, exact-project,
    and temporary-file cleanup contracts in capabilities, tests, docs, and the
    powerbi-cli skill.

See [EVIDENCE.md](EVIDENCE.md), [SCORECARD.md](SCORECARD.md), and
[HANDOFF.md](HANDOFF.md).

## 2026-07-22 follow-up pass

The structured in-tree workspace under [audit](audit/) records a second focused
pass covering duplicate-title Desktop selection, screenshot foreground
ownership, compact capability discovery, canonical wrong-path suggestions, the
dashboard repair skill, and MCP shutdown latency. Start with
[audit/manifest.json](audit/manifest.json) and [audit/HANDOFF.md](audit/HANDOFF.md).
