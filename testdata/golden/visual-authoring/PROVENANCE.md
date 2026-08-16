# Visual authoring golden provenance

These exact `visual.json` shapes replicate Power BI Desktop-rendered fixtures
captured during the 2026-08 production pilot. They are intentionally genericized
to the `FactSales`, `DimCustomer`, and `DimDate` model used by `scaffold_sales`
in the integration tests.

The fixtures cover `card`, `tableEx`, `lineChart`, `scatterChart`, and
`hundredPercentStackedColumnChart`. Their checked generation is the
`schema-golden` precondition required by the roadmap freeze note; it does not
claim an automated Desktop canvas/refresh proof level.
