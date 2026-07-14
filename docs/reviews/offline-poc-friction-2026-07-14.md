# Offline POC agent friction — 2026-07-14

End-to-end exercise: inspect spreadsheet source metadata, model a PostgreSQL dimension/fact relationship, generate a two-page PBIP, add work-machine rebind templates, package it, open it in Power BI Desktop, refresh it, and inspect both canvases.

## Fixed during the exercise

### Source pack rejected generated source templates

- Symptom: `package source-pack` rejected `.powerbi-cli/source-templates.json` as an unapproved dot-directory file.
- Impact: a safe transfer archive could not retain the CLI-generated PostgreSQL rebind map.
- Fix: allow exactly `.powerbi-cli/source-templates.json` as metadata while continuing to reject every other unknown dot-directory file.
- Proof: the source-pack round-trip test now authors a PostgreSQL source template and verifies that the imported project retains the sidecar. All five source-pack safety tests pass.

### Date-only values failed in datetime dummy columns

- Symptom: a schema row containing `"2015-01-23"` for a `datetime` column was emitted as quoted M text inside a typed `#table`.
- Impact: Power BI loaded the fact table with one conversion error per row.
- Fix: emit date-only ISO values in datetime columns as `#datetime(year, month, day, 0, 0, 0)`.
- Proof: unit coverage for date-only and timestamp inputs, plus a clean Power BI Desktop refresh of the regenerated proof-of-concept report.

## Remaining friction / useful follow-ups

1. `capabilities --for <exact command>` still returns the full heavyweight agent contract around the matching command. A compact mode that returns only `path`, `usage`, `flags`, `examples`, proof level, and output fields would reduce parsing and output noise.
2. `report wireframe export` returns useful structured JSON but cannot write an SVG/HTML/PNG preview. A deterministic visual artifact would make layout QA possible without launching Desktop.
3. First-open Desktop verification required two separate `Refresh now` actions: calculated objects, then table data. A CLI oracle option that performs refresh, captures table/row errors, and returns the exact failing table/column would tighten the repair loop.
4. `desktop open-check` is intentionally honest about its proof boundary, but an explicit `--refresh --capture <dir>` workflow would turn the currently manual canvas/refresh review into repeatable evidence.

The repo skill did not cause repeated command-usage loss in this run, so no skill workflow change was necessary. The friction was in command output volume and missing proof artifacts rather than forgotten syntax.
