# Power BI Rebind Plan

Project: `<project-dir>`

Power BI project: `<project-dir>/SalesOperations.pbip`

Status: ready for work-machine review.

## Design scorecard and proof status

This scorecard was computed from the current project before writing this runbook. Re-run triage after rebinding or editing the project. Source-template completeness does not establish report quality.

Local proof level: `unit-smoke`. Generating this runbook does not verify a schema golden, open Desktop, refresh data, or inspect a canvas. No external proof record is imported by this command.

| Proof level | Status | Evidence required |
| --- | --- | --- |
| `unit-smoke` | local-checks-passed | Native validation and local lint checks; warnings remain in the scorecard. |
| `schema-golden` | not-verified | Compare the current project with an approved schema golden. |
| `desktop-golden-pending` | not-verified | Provide a matching Desktop-authored reference; canvas and refresh proof is still pending. |
| `manual-desktop-canvas-refresh` | not-verified | Record a manual Desktop canvas and refresh review for the current project. |
| `desktop-canvas-refresh` | not-verified | Record automated Desktop canvas and refresh evidence for the current project. |

The full `scorecard.v1` below includes native validation, Microsoft-validator availability, general lint, design findings with pointers and suggested actions, offline handoff safety, and follow-up commands.

```json
<live scorecard.v1 JSON; asserted against triage>
```

## Prerequisites

- Power BI Desktop is installed on the work machine.
- The machine can reach every configured data source.

## Rebind procedure

1. Open the `.pbip` project in Power BI Desktop at work.
2. Open Transform data / Power Query and locate each table or partition listed below.
3. Replace its dummy `#table(...)` query with the corresponding M snippet.
4. If prompted, enter source credentials only in Power BI Desktop, then apply the query changes.

## Plan findings

- **warning** `rebindPlan.missing_source_template`: no source template is configured for partition:DimCustomer:DimCustomer
- **warning** `rebindPlan.missing_source_template`: no source template is configured for partition:DimDate:DimDate
- **warning** `rebindPlan.missing_source_template`: no source template is configured for partition:FactSales:FactSales

## Partition replacements

### `partition:DimCustomer:DimCustomer`

No source template configured yet. Add one with `source-template add`.

### `partition:DimDate:DimDate`

No source template configured yet. Add one with `source-template add`.

### `partition:FactSales:FactSales`

No source template configured yet. Add one with `source-template add`.

## Post-rebind verification

- [ ] Refresh completes successfully for every rebound table.
- [ ] Every report page canvas renders with the expected visuals and data.
- [ ] No Power BI Desktop issue, warning, or error banners remain.
- [ ] Optional, if `powerbi-cli` is available at work: re-run fixture normalization and verification.
  - `powerbi-cli fixture normalize <project-dir-or.pbip> --out <work-machine-summary.json> --json`
  - `powerbi-cli fixture verify <project-dir-or.pbip> --expected <approved-summary.json> --json`

Credentials must live only in Power BI Desktop on the work machine. Never put them in TMDL, source-template sidecar metadata, or this runbook.
