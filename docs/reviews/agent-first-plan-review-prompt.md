# Independent Review Prompt: Agent-First Power BI CLI

You are independently reviewing the plan for `powerbi-cli`, a Rust CLI intended
primarily for AI coding agents authoring Power BI dashboard projects. Do not
rubber-stamp the plan. Be critical, concrete, and pragmatic.

## Project Context

Repository under review:

```text
<workspace>\powerbi-cli
```

Reference project for the desired CLI feel:

```text
<workspace>\ooxml-cli
```

External Power BI tooling research clones:

```text
<workspace>\powerbi-cli-research
```

Read these local files if you have filesystem access:

- `README.md`
- `docs/roadmap.md`
- `docs/porting-analysis.md`
- `docs/clean-room-research.md`
- `examples/archetypes/regional-sales.schema.json`
- `<workspace>\ooxml-cli\README.md`
- `<workspace>\ooxml-cli\GOAL.md`
- `<workspace>\ooxml-cli\skills\ooxml\SKILL.md`

If you do not have filesystem access, use the summary below.

## What `powerbi-cli` Currently Does

`powerbi-cli` is a cross-platform Rust CLI. It can scaffold an offline-safe PBIP
Power BI Project from a JSON schema manifest. It emits:

- PBIP project files;
- PBIR report/page/visual files;
- TMDL semantic model files;
- inline dummy Power Query M partitions;
- tables, columns, measures, relationships, pages, visuals, and early visual
  field bindings.

Current commands:

- `version`
- `capabilities`
- `doctor`
- `scaffold --schema <json> --out-dir <dir> [--force]`
- `inspect <project|pbip>`
- `validate <project|pbip>`

Important workflow:

```text
bring schema/dummy rows home
-> scaffold PBIP/PBIR/TMDL project
-> author report/model metadata with agents
-> validate no cache/credentials/real data are present
-> open at work in Power BI Desktop
-> replace dummy M partitions with corporate sources and refresh
```

This is explicitly not direct PBIX binary generation. Power BI Desktop is the
oracle for compatibility.

## Desired `ooxml-cli`-Like Agent-First Feel

`ooxml-cli` is the model for agent ergonomics:

- live `capabilities` command;
- JSON output for reads and mutations;
- stdout is data, stderr is diagnostics;
- mutating commands require `--out`, `--in-place`, or `--dry-run`;
- mutations return follow-up commands for inspect/validate/proof;
- stable handles from the CLI instead of guessed internal paths;
- validator plus desktop-oracle proof before compatibility claims;
- canonical agent skill/robot guide that tells agents how to discover the live
  contract and avoid stale examples;
- pure cross-platform implementation path, with desktop Office/Power BI as an
  optional proof oracle, not an implementation dependency.

For `powerbi-cli`, that means the primary user is not a human manually clicking
around. The primary user is an agent trying to create, inspect, mutate, validate,
and prove a dashboard project under uncertainty.

## Existing Plan Summary

The current plan says `powerbi-cli` should become:

- a PBIP/PBIR/TMDL compiler from schema manifests;
- an inspector and validator;
- a linter/sanitizer;
- a semantic model mutator;
- a report/page/visual mutator;
- a theme/style extractor and applier;
- a Desktop oracle harness;
- eventually optional live Desktop/Fabric bridges.

Planned command families include:

- foundation: `version`, `capabilities`, `doctor`, `inspect`, `validate`,
  `lint`, `sanitize`, `diff`, `apply`;
- project: `scaffold`, `from-template`, `clone-style`, `apply-style`,
  `handoff`, `source-template`;
- semantic model: tables, columns, calculated columns, calculated tables,
  measures, relationships, partitions, roles, perspectives, calculation groups,
  cultures, translations, named expressions, dependencies;
- DAX/M: format, lint, references, validate/query through optional Desktop
  bridge;
- report: pages, visuals, visual binding, filters, slicers, bookmarks,
  interactions, themes, formatting, conditional formatting, annotations,
  wireframe export;
- desktop oracle: open-check, save-check, export-snapshot, capture-golden,
  reload.

Planned development phases:

1. modularize the Rust file;
2. add deep inspect/validation/lint;
3. create Desktop-authored golden fixtures;
4. implement semantic model commands;
5. implement report CRUD;
6. build a visual binding catalog from Desktop fixtures;
7. add formatting/themes/conditional formatting;
8. add filters/slicers/bookmarks/interactions;
9. add batch operation plans and agent review;
10. add optional live Desktop/Fabric bridges.

## External Tooling Signals

Permissive or open references:

- `MinaSaad1/pbi-cli`: broad agent-oriented command surface covering semantic
  model operations, DAX, tables, columns, relationships, report/pages/visuals,
  filters, formatting, bookmarks, custom visuals, Desktop sync. Uses Python,
  .NET/TOM/ADOMD, and PBIR JSON editing. MIT plus Microsoft AS client library
  terms.
- `akhilannan/pbir-utils`: PBIR lint/sanitize/metadata/wireframe/themes/pages/
  filters/interactions/measure dependency workflows. MIT.
- `microsoft/powerbi-modeling-mcp`: official MCP shape for semantic model
  operations: tables, columns, measures, relationships, DAX query, partitions,
  hierarchies, calc groups, roles, perspectives, expressions, cultures,
  translations, calendars.
- `microsoft/semantic-link-labs`: Fabric/service-side semantic model metadata,
  dependencies, BPA, translations, refresh, gateway/service operations. MIT.
- `TabularEditor/TabularEditor`: mature semantic model concepts, scripting,
  dependency analysis, BPA, TMDL/TOM world. MIT.
- `DaxStudio/DaxStudio`: DAX/query/performance diagnostics. Reciprocal license.

Quarantined/restricted references:

- `pbi-tools/pbi-tools`: extract/compile/convert/deploy/export-data/source-control
  workflows. AGPL.
- `maxanatsko/pbir.tools`: PBIR shell-like browsing/mutation UX, backup/restore,
  validation, report ops. Custom non-commercial/no-derivatives.
- `data-goblin/power-bi-agentic-development`: agent skills and PBIR workflow
  expectations. GPL.

Do not be timid about identifying code/features worth porting. Separate
technical usefulness from license consequences. If code is technically worth
using, say so. Also say what the consequence would be: direct port with
attribution, reimplementation, optional plugin, relicensing requirement, or
behavior-test-only.

## Review Task

Your job is to determine the most optimal agent-first CLI plan possible.

Please answer in this exact structure:

1. **Verdict**: Is the current plan fundamentally right, too timid, too broad,
   missing the agent-first point, or mis-sequenced?
2. **Top 10 Plan Changes**: Concrete changes to the roadmap, in priority order.
   Each item should say why an agent benefits.
3. **Command Surface Critique**: Proposed command names, global flags, output
   contract, and error/exit-code contract. Include commands an agent would
   guess first.
4. **Agent Workflow Design**: Describe the shortest happy paths for:
   schema-to-report, style extraction/application, measure authoring, visual
   authoring, validation/proof, and work-machine rebind.
5. **Porting Recommendations**: For each external repo, say what should be
   directly ported, reimplemented, treated as test inspiration only, or ignored.
   Be specific.
6. **Missing Power BI Features**: Measures, calculated columns, visual types,
   conditional formatting, themes, bookmarks, filters, relationships, source
   templates, Desktop oracle, etc. What is missing or over/under-prioritized?
7. **Testing Strategy**: What exact golden/oracle/fuzz/metamorphic tests should
   exist so agents can trust the CLI?
8. **Risk Register**: The biggest technical/product risks and how to reduce
   them.
9. **First 5 Implementation Slices**: Small, high-leverage implementation slices
   we should do next.
10. **Uncomfortable Truths**: Anything the maintainer may not want to hear.

Rules:

- Do not give generic Power BI advice.
- Do not say "it depends" without deciding.
- Do not avoid licensing realities, but do not let legal caution hide good
  engineering ideas.
- Do not ask for clarification.
- Assume the CLI is for agents first and humans second.
- Prefer deterministic, inspectable, replayable CLI operations over broad
  interactive automation.
