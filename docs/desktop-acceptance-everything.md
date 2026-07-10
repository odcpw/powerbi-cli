# Desktop Acceptance: Everything Suite

Last verified: 2026-06-25

## Scope

The `desktop_acceptance_everything` test builds `build/desktop-everything/EverythingAcceptance` and exercises every advertised `powerbi-cli` command family against one PBIP project:

- semantic model scaffolding, tables, relationships, partitions, handoff, package import/export/extract, fixtures, diff
- DAX measures and calculated columns
- generated visuals, visual mutations, formatting, conditional formatting, style/theme extract and apply
- report pages, filters, slicers, bookmarks, interactions, drilldown, drillthrough, layout, tree/query/cat surfaces

The suite generated 6 tables, 13 measures, 4 relationships, 4 pages, and 16 bound visuals.

## Desktop Oracle Evidence

Power BI Desktop Store `2.155.756.0` opened the generated PBIP, refreshed the dummy M partitions, and rendered all report pages.

Observed in Desktop after refresh:

- Overview: cards `Total Incidents = 2,040` and `Severe Incidents = 54`, line chart `Incident Rate by Branch`, company detail table, and column chart `Total Incidents by Branch`.
- Visual Catalog: area chart, stacked area chart, clustered bar chart, stacked bar chart, stacked column chart, and table all rendered with data.
- Scatter Drill: scatter/bubble chart and detail table rendered with accident-cost rows.
- Drillthrough Detail: detail table and `$7M` total-cost card rendered.

No `Failed to load the report`, `Something went wrong`, missing custom visual, incomplete-data, or refresh banners remained after Desktop refresh.

## Bugs Found By Desktop

- TMDL descriptions must be emitted as leading `///` comments. Desktop rejected calculated-column `description:` properties.
- PBIR registered themes must use schema-valid `themeCollection.customTheme` metadata and `resourcePackages`; Desktop rejected the old `customTheme.resource` shape.
- Generated stacked bar/column visual type IDs must be `barChart` and `columnChart`. Desktop treated `stackedBarChart` and `stackedColumnChart` as missing custom visuals.

These are now covered by source changes and validation/test coverage.

## Planner And Source Package Proof

Verified on 2026-06-25 with Power BI Desktop Store `2.155.756.0`.

Reproduction path:

```powershell
cargo run -- profile infer --schema examples/sales.schema.json --out build\desktop-planner-proof\sales.profile.json --json
cargo run -- report plan --schema examples/sales.schema.json --profile build\desktop-planner-proof\sales.profile.json --objective "Executive sales dashboard with revenue trend, segment comparison, and portfolio scatter" --out build\desktop-planner-proof\sales.planned.dashboard.json --json
cargo run -- report build --schema examples/sales.schema.json --profile build\desktop-planner-proof\sales.profile.json --spec build\desktop-planner-proof\sales.planned.dashboard.json --out-dir build\desktop-planner-proof\PlannedSales --json
cargo run -- package source-pack --project build\desktop-planner-proof\PlannedSales --out build\desktop-planner-proof\planned-sales-source.pbit --json
cargo run -- package import build\desktop-planner-proof\planned-sales-source.pbit --out-dir build\desktop-planner-proof\PlannedSalesImported --json
cargo run -- validate --strict build\desktop-planner-proof\PlannedSalesImported --json
cargo run -- handoff check build\desktop-planner-proof\PlannedSalesImported --json
Start-Process -FilePath build\desktop-planner-proof\PlannedSalesImported\SalesOperations.pbip
```

Observed in Desktop after clicking `Refresh now`:

- Overview page rendered two KPI cards (`$13K`, `42`), a line chart, a column chart, and a detail table populated from dummy M partitions.
- Analysis page rendered the planned scatter chart and detail table.
- Relationship and incomplete-data banners cleared after refresh.
- The Desktop window was saved and closed after verification.
