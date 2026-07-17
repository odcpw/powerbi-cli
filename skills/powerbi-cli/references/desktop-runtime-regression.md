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
5. Visit every changed page and confirm that no visual has an error banner.
6. Exercise every new selector/toggle, including its non-default option.
7. Click the matrix/table rows that drive drill or cross-filter behavior.
8. Confirm the time axis remains stable after branch/company/body-part
   selections.
9. For scatter charts, confirm bubbles render, the Legend well is populated,
   and a branch selection changes the point grain to companies when intended.
10. Compare transformed charts with raw cards/tables to prevent misleading
   labels or accidental aggregation changes.

If Desktop reveals a failure that strict validation missed, preserve a minimal
fixture, add a validator/lint rule when it can be made precise, add a regression
test, update `docs/pbir-desktop-oracle.md`, and repeat this loop.
