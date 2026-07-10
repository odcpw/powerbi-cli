# Data-Agnostic Dashboard Build Synthesis

Date: 2026-06-23

## Goal

Make `powerbi-cli` an agent-first compiler/workbench for arbitrary Power BI
dashboards from schema, profile, and explicit report intent. Any one fixture
and sales are examples, not product boundaries.

## Review Inputs

This pass used four independent review lenses:

- Systems/FMEA: isolate schema, profile, spec, build, validation, and proof so
  report generation does not become another monolith.
- Product/spec design: ship a build-first declarative spec before attempting a
  free-form planner.
- CLI ergonomics: make the first obvious commands be `schema validate`,
  `profile infer`, `report spec validate`, and `report build`.
- Golden/conformance testing: prove the agent contract with snapshot-like
  fixture summaries and structured refusal for unimplemented inference.

## Decisions

- Add focused modules: `schema.rs`, `profile.rs`, and `report_build.rs`.
- Compile `powerbi-cli.dashboard.v1` specs into the existing scaffold/project
  primitives rather than creating a parallel PBIR writer.
- Validate visual roles through the existing visual catalog.
- Treat profile data as advisory metadata; it must not silently invent report
  intent.
- Keep `report plan` as `unsupported_feature` until multiple unrelated
  dashboard archetype goldens prove it.
- Emit exact next commands after build so agents can validate, inspect, handoff
  check, fixture-freeze, and Desktop-proof explicitly.

## Implemented Slice

- `schema validate`
- `schema normalize`
- `profile infer`
- `profile validate`
- `profile summarize`
- `report spec fields`
- `report spec validate`
- `report build`
- `report plan` structured refusal
- `examples/sales.dashboard.json`
- `testdata/golden/generic-sales.summary.json`
- `examples/archetypes/flat-ops.*` plus golden summary
- `examples/archetypes/scatter-bubble.*` plus golden summary
- Manual Desktop canvas/refresh proof records for the two archetypes in
  `testdata/desktop-proof/`
- Contract tests for capabilities and robot docs
- Golden/conformance test for a generic schema/profile/spec build

## Remaining Proof

The repeatable CI proof is still local PBIP/PBIR/TMDL validation plus golden
summary verification. Two generated archetypes have also been manually opened,
refreshed, and inspected in Power BI Desktop:

- `flat-ops`: card, clustered bar, table;
- `scatter-bubble`: scatter/bubble chart and table.

Those proof records distinguish `desktop open-check` launch proof from manual
canvas/refresh proof. The next proof boundary is an automated
Desktop canvas/refresh oracle that opens Desktop, refreshes dummy partitions,
detects blank canvases or issue banners, captures evidence, and closes Power BI
Desktop plus `msmdsrv` after each run.
