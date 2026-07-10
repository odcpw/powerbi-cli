# Desktop-authored PBIR visual shapes — reference for wave-2 work orders

Source: `microsoft/BCApps` (MIT license), Business Central Power BI apps — real PBIR-format
reports authored by Power BI Desktop itself. Files archived in this directory verbatim.
Attribution required if any structure is adapted into the repo. Retrieved 2026-07-09 via gh api.
All four use `$schema: .../visualContainer/2.4.0/schema.json` — same version powerbi-cli emits.

## pieChart (pieChart.visual.json)
- `visualType: "pieChart"`, queryState roles: `Category` (1 column projection) + `Y` (1+ measure projections).
- Identical projection grammar to powerbi-cli's existing families (field/queryRef/nativeQueryRef, `active: true` on Category).
- Example carries `sortDefinition` (sort by Y measure, Descending, `isDefaultSort: true`).
- objects seen: `legend`, `labels` — all under `/visual/objects` (root-level objects absent, consistent with our Desktop finding).
- Binding family for catalog: like a Series-less CategoryY — 1 Category column, 1..N Y measures, NO Series role. These fixtures prove measures only; raw Y columns remain aggregation-binding gated.
- donutChart (donutChart.visual.json) is byte-identical in structure, just `visualType: "donutChart"`.

## matrix (matrix.visual.json)
- `visualType: "pivotTable"` (NOT "matrix" — important).
- queryState roles: `Rows`, `Values` (this example); `Columns` is the third role when used.
- objects seen: `columnWidth` only. Minimal generation: Rows (1+ columns, drill hierarchy semantics like Category), optional Columns (columns), Values (1+ measures). The fixture does not prove raw Values columns.

## slicer (slicer.visual.json)
- `visualType: "slicer"`, queryState role: `Values` (single column projection, `active: true`).
- `objects.data[0].properties.mode`: literal string e.g. `'Single'` (other Desktop modes: 'Basic' list, 'Dropdown', 'Between' for numeric/date ranges — this example is numeric with `numericStart`).
- CRITICAL: this example's `objects.general[0].properties.filter` contains a persisted selection
  (Version 2 From/Where categorical filter) — that is user STATE, not structure. Generated slicers
  must OMIT `general.filter` entirely (clean, no persisted values) — Desktop adds it on selection.
  This keeps generated slicers clean under powerbi-cli's `may_contain_data_values` hygiene scan.
- Note From entry uses `"Entity"` + `"Type": 0` here (inside general.filter) — differs from the
  Where/Source-alias discipline our oracle doc records for filterConfig filters; slicer visual-level
  persisted filters are a different surface. Moot if we never emit general.filter.

## visual container titles
- Every archived fixture has `/visual/visualContainerObjects/title`.
- The slicer fixture proves a literal `text` expression and a literal `show` property; the other fixtures use measure-driven title text.
- Generated literal titles therefore belong in `visualContainerObjects`, not root-level `objects`. `powerbi-cli` emits literal text with `show: true` and keeps `/visual/objects/general` for alt text.

## Implications for powerbi-cli catalog work (WO-5/WO-6)
1. Pie/donut slot into the existing visual factory with a new binding family (Category + Y, no Series);
   reuse `visual_query_json` unchanged.
2. Matrix = new family Rows/Columns/Values mapping onto the same projection builder; visualType string "pivotTable",
   catalog name/aliases "matrix"/"pivotTable".
3. Slicer authoring = Values role + `objects.data.mode` only; never emit `general.filter`;
   slicer counts as unbound-ish visual for hygiene (no persisted values at generation time).
4. All proof runs through Desktop before flipping feature_catalog status — build fixture project,
   `desktop open-check`/`desktop screenshot` with POWERBI_DESKTOP_ORACLE=1, judge render, record proof artifact.
