# Pass 1 Playbook

1. Prefer launch provenance over title alone when selecting Desktop windows.
2. Refuse ambiguous pre-existing same-title windows; never guess by PID order.
3. Treat a foreground process as owned only when it is the selected Desktop PID or a verified descendant.
4. Keep `capabilities --for` small; request the full contract only for shared catalogs.
5. Correct plausible command-family mistakes by pointing at one unique live catalog path.
6. Delete PBIR visual containers through `report visuals delete`, not raw filesystem edits.
7. Use hierarchy drill for changing grain, Series/slicers for comparison, and drillthrough for page navigation.
8. Run strict validation, DAX dependency/lint, wireframe, interaction inventory, and handoff checks in that order.
9. Keep Desktop canvas/refresh claims separate from file, DAX, window, and screenshot evidence.
10. Keep MCP process monitoring to PID-tree identity data; never poll expensive CPU, memory, disk, executable, or task fields for cleanup.
11. Keep one canonical dashboard project and one reusable QA output; use Git for rollback instead of same-title version directories.
12. Use `desktop open` only for an explicit interactive session and always pair it with idempotent `desktop close`.
13. Treat PID plus creation time as the minimum process-ownership identity; never recover ownership by title or executable sweep.
14. Use Between slicers only for numeric/date columns and preserve that invariant during rebinding.

---

# Pass 2 playbook (2026-08-18)

Scope decision: full mode; evidence source = two days of intensive live agent
use (an intensive real dashboard-authoring campaign) + a fresh probe battery. The tool's baseline
contract (capabilities catalog, JSON-only stdout, error envelopes, exit
dictionary, byte-deterministic capabilities, fuzzy --for, global-flag
did-you-mean, bare-invocation usage) scored 850-950 — the strongest baseline
this methodology has seen. Seven gaps survived, all evidence-backed:

1. **R-201 help-at-depth (P0).** `--help` below top level is an exit-2 error.
   The redirect hint is good; actual help is better. Render from the catalog
   so it can never drift.
2. **R-202 triage mega-command (P0).** The observed QA loop is
   validate → lint → external filtering, dozens of times. One call.
3. **R-203 did-you-mean parity (P1).** Global flags have it; subcommand values
   and per-command flags don't (`pages lst`, `--strick`).
4. **R-204 lint noise (P1).** 28/52 unbuffered_reuse findings on the real
   project are function/scalar steps the agent filtered by hand every round.
5. **R-205 version identity (P2).** Stale-binary phantom-results incident;
   gitSha + buildEpoch make staleness detectable.
6. **R-206 guid util (P2).** lineageTag GUIDs needed constantly; host lacks
   uuidgen.
7. **R-207 oracle flag (P2).** Env-var-only opt-in cost a launch cycle.

Applied via two parallel Grok executors (wo-help: R-201+R-203;
wo-triage: R-202+R-204..207), merged on main, re-scored, fresh-eyes,
regression-tested. Deferred (unchanged from Pass 1): transactional mutation
manifests, DAX assertion suites, automated canvas-refresh proof.
