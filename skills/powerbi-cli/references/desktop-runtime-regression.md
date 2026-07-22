# Desktop Runtime Regression Loop

Use this reference after report repair, new DAX, a new visual binding, or a
home/work source change. Static validation is a gate; Power BI Desktop refresh
is the runtime oracle.

## Preserve the source boundary

- Keep the home project on synthetic/static or credential-free local data.
- Keep the work project on its PostgreSQL/ODBC/enterprise partitions.
- Mirror semantic and report changes deliberately; never overwrite the work
  partition blocks with synthetic M.
- Before handoff, run `handoff check` and inspect every changed partition.

For business keys, test uniqueness at the grain the report actually uses. A
business number may repeat across plant parts. Prefer a documented composite
key such as normalized business number plus plant-part code, and test both
blank components and duplicate composites.

## Author defensively

- Scatter color grouping: bind PBIR `Series`. Desktop calls the field well
  Legend, and the CLI accepts `legend` as an input alias, but raw
  `queryState.Legend` is not a working scatter binding.
- DAX table choices: never use `VAR T = IF(condition, TableA, TableB)`. Branch
  around the scalar calculation that consumes each table.
- Disconnected selector labels used with `TREATAS` must exactly match their
  destination values, including whitespace, dash type, and separators.
- A log-scale measure must not replace the raw business value everywhere. Keep
  raw cards, tooltips, and tables; label the transformed axis explicitly.

## Validate the isolated QA copy

1. Copy the PBIP project without `.pbi`, cache files, or Desktop local state.
2. Run `validate --strict` on the copy.
3. Open that exact copy in Desktop and refresh.
4. When a targeted model assertion can be expressed as a table-valued DAX
   query, run `model dax execute` with both opt-ins and default/tighter bounds;
   keep its result separate from canvas proof.
5. When the live PBIX semantic model itself must be inspected as source, run
   `model live export-tmdl` with `--allow-model-read` into one fresh temporary or
   reviewed output. Require the output hash/counts and complete MCP child/pump
   cleanup. Treat its M expressions and static values as sensitive metadata;
   it is TMDL-only and does not prove report-page extraction.
6. Visit every changed page and confirm that no visual has an error banner.
7. Exercise every new selector/toggle, including its non-default option.
8. Click the matrix/table rows that drive drill or cross-filter behavior.
9. Confirm the time axis remains stable after branch/company/body-part
   selections.
10. For scatter charts, confirm bubbles render, the Legend well is populated,
   and a branch selection changes the point grain to companies when intended.
11. Compare transformed charts with raw cards/tables to prevent misleading
   labels or accidental aggregation changes.

Use these exact local QA commands before Desktop:

```bash
pbi --json validate --strict <project>
pbi --json model dax dependencies --project <project>
pbi --json model dax lint --project <project>
pbi --json report wireframe export <project>
pbi --json report interactions list --project <project>
```

Use hierarchy drill for one chart changing from branch to company. Use a
company Series/Legend binding or multi-select slicer when several companies
must remain visible on the same year axis. Use drillthrough only for navigation
to a target page with carried filter context.

If several Desktop instances share the same PBIP title, keep the newly launched
instance open or close the duplicates before proof. The CLI refuses ambiguous
pre-existing title matches instead of capturing an arbitrary report.

If Desktop reveals a failure that strict validation missed, preserve a minimal
fixture, add a validator/lint rule when it can be made precise, add a regression
test, update `docs/pbir-desktop-oracle.md`, and repeat this loop.
