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
