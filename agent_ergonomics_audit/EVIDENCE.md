# Evidence

## Observed failures

| Boundary | Static result | Desktop/runtime result | Root cause |
|---|---|---|---|
| Scatter color grouping | Strict validation passed | Legend well remained empty; no useful bubbles | Catalog wrote raw `queryState.Legend`; Desktop consumes `Series` |
| Body-part selector DAX | References resolved; strict validation passed | Visual displayed a DAX error | `VAR T = IF(...table..., ...table...)` attempted to use scalar IF as a table selector |
| Deleted visual cleanup | Structural validation ignored empty folder | Deep inspection failed with generic `file_not_found` | Visual directory existed without `visual.json` |
| Line-chart raw tooltip | Hardened role validation initially rejected it | Desktop had already rendered it correctly | Category/Y catalog omitted the proven optional `Tooltips` role |
| Disconnected selector | Model and DAX were structurally valid | Selection did not filter | Selector label separator did not exactly match the destination value used by `TREATAS` |

The scatter diagnosis was isolated by evaluating the model query separately:
the measure returned nonblank points, while Desktop's field well showed Legend
empty. Changing only the PBIR role key from `Legend` to `Series` rendered the
bubbles and legend. This supports a precise validator rule rather than a broad
heuristic.

## Baseline workflow friction

- Raw PBIR JSON required manual inspection because the visual catalog exposed
  the wrong canonical role.
- Static DAX analysis reported references and cycles but missed a repeatable
  scalar/table semantic trap.
- Confirming a DAX fix required entering or running a query through UI control;
  the CLI had no live semantic-engine execution surface.
- An empty OneDrive-backed visual directory was surfaced one command later than
  the structural defect and without a repair.
- The generic agent-ergonomics preflight could not run in WSL because `node` and
  `jq` were absent there, although the Windows Rust/PowerShell toolchain was
  healthy. The audit therefore used the native Windows commands documented by
  the project skill.

## Acceptance evidence

- Catalog tests assert scatter exposes `Series` and never `Legend`.
- Visual-add tests assert `role=legend` input produces a `Series` binding and
  raw `queryState.Series`.
- Strict-validation tests assert stale scatter `Legend` is rejected with a
  `use Series` repair.
- DAX tests assert the table-variable pattern fails lint/strict validation and
  a normal scalar IF variable remains unflagged.
- Structural tests assert an empty visual directory produces a remove-or-restore
  diagnostic.
- Live synthetic Desktop 2.155.756.0 checks returned a literal row and the
  report's `[Anzahl Unfälle]`/`[Gesamtkosten]` measures through `model dax
  execute`; an intentionally invalid measure name returned a bounded exit-10
  engine diagnostic.
- CLI tests assert two independent opt-ins, accepted query forms, mutation-form
  refusal, query non-echo, bounds, exact-project discovery, and temporary-file
  cleanup metadata.
- The final handoff requires the full Rust test suite plus strict validation of
  both PostgreSQL and synthetic report variants.

## Why the implementation uses the local model engine

Microsoft documents that Desktop hosts an Analysis Services model on a dynamic
local port and that Analysis Services client libraries can execute DAX queries.
Microsoft's newer Desktop IPC Bridge preview is useful for state, reload, and
page screenshots, but its 2026-07-15 manifest documents no DAX-query method.
The implemented slice therefore uses ADOMD for read-only query execution and
keeps process/port discovery behind exact-project matching and explicit opt-in.
The discovery mechanism is a compatibility boundary to retest after Desktop
updates; if the official IPC manifest adds query execution, it should become
the preferred transport.
