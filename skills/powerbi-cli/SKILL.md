---
name: powerbi-cli
description: Build, inspect, validate, and edit Power BI source projects with the Rust CLI. Use for synthetic examples, PBIP reports, PBIR visuals, TMDL models, and safe handoffs.
---

# Power BI CLI

Treat the freshly built CLI as the command reference. Begin with
`powerbi-cli capabilities --json` or `powerbi-cli features list --json`
and use `powerbi-cli doctor --json` when the executable or integrations
are in doubt. The repository README contains the generated catalog.

## Create or inspect a project

1. Use a synthetic schema or an authorized, schema-only manifest. Do not
   copy credentials, real rows, caches, or local settings into a portable
   project. Use an example from the repository when learning the workflow.
2. For a new report, preview `powerbi-cli report compose --schema examples/sales.schema.json --objective "Sales overview" --dry-run --json`.
   Inspect the stages, scorecard, proof plan, and wireframes. For output,
   replace `--dry-run` with a new `--out-dir`. If a declarative spec is
   already available, inspect the build contract instead of planning twice.
3. For an existing report, inspect the exact PBIP and select page, visual,
   table, or measure handles from the output. Run a dry run before editing.
   Prefer a new output directory. Use an in-place confirmation token only
   when requested and bind it to the dry run for that exact edit.
4. Keep an explain preview separate from an operations envelope. Only send a
   document with the replayable operations schema to `powerbi-cli ops apply`.
   For unknown fields, follow the returned pointer or query
   `powerbi-cli lint --explain ops.dangling_handle --json`.

## Verify the result

- Run `powerbi-cli validate --strict build/composed-sales --json` and
  `powerbi-cli handoff check build/composed-sales --json` for the generated
  project. Treat errors as a refusal, not a cue to bypass validation.
- Use Power BI Desktop on Windows to verify rendering and interaction. Report
  file validation, model evaluation, and live Desktop behavior separately.
  Do not claim a refresh, render, or feature interaction you did not observe.
- See `references/desktop-runtime-regression.md` for the Desktop evidence
  loop. The repository's PBIR Desktop notes document schema constraints and
  compatibility evidence.
- Inspect any source-rebind plan in the authorized destination environment
  before binding it to real data. Keep authentication outside portable files.

## Contribute to the CLI

Keep dispatch, artifact readers, model writers, and validation separate.
For a catalog change, regenerate the repository README with
`powerbi-cli robot-docs render` and run `powerbi-cli robot-docs verify`.
Check changes to emitted files with a synthetic end-to-end fixture and
byte-parity tests where applicable.
