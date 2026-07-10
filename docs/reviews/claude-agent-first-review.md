I now have a thorough picture of the codebase. Let me write the review.

---

# Independent Review: Agent-First Power BI CLI

## 1. Verdict

The plan is **directionally right but mis-sequenced and incomplete on the things that matter most to agents**. The core insight — schema-first, offline-safe, PBIP/PBIR/TMDL compiler with Desktop as the compatibility oracle — is the correct niche and the plan defends it well. The sequencing problem is that it front-loads infrastructure work (modularize, vendor schemas, golden fixture harness) before agents can do any useful mutations. An agent landing in this project today can `scaffold` and `validate` but cannot add a measure, rename a page, move a visual, or query what fields a visual is bound to. The "golden fixtures required before marking stable" rule is correct in principle but will be misapplied as a gate that stalls authoring commands for months. The other gap is the agent ergonomics contract: `capabilities` doesn't describe argument schemas, there are no stable object handles, `--json` position is a trap, and there is no SKILL file telling agents how to discover the live contract.

---

## 2. Top 10 Plan Changes

**Priority 1: Ship `model measures add/update/delete` before anything else in phase 3.**
Every real dashboard has measures. Measures are simple TMDL text: a file per measure in the semantic model folder, one `measure` block with `expression`. No Desktop oracle is needed to get this right — the format is fully documented and the pattern is already used by `scaffold`. An agent that can scaffold and then add a measure can author something real. This unlocks all downstream work and gives you the first round-trip proof target without waiting for the full golden fixture harness.

**Priority 2: Add stable named handles for pages and visuals.**
Right now a page is identified by its directory name and a visual by its file name — both opaque to an agent. An agent asking "move visual to x=200" has no way to identify which visual it means without knowing PBIR internals. Assign stable, deterministic, human-readable handles at scaffold time (`page:Overview`, `visual:TotalRevenue_Card`) and preserve them through mutations. Every list/show command should return handles. Every mutation should accept `--page <handle>` and `--visual <handle>`.

**Priority 3: Make `capabilities` return the full argument schema per command.**
Current `capabilities` output: `{"path": "scaffold", "usage": "...", "summary": "...", "tags": [...]}`. An agent trying to compose `model measures add` needs to know every flag, which are required, what types they accept, and what the output contract is. Without this, agents guess and fail. Model this on the `ooxml-cli` SKILL contract: each command entry in `capabilities` should list `args[]` with `name`, `required`, `type`, `description`, and `example`. The `contractVersion` field already exists; use it to drive agent discovery.

**Priority 4: Write the SKILL.md agent robot guide immediately, not after everything stabilizes.**
`ooxml-cli` has `skills/ooxml/SKILL.md`. `powerbi-cli` has nothing equivalent. Agents arriving at this tool today will hallucinate commands (`pbicli measures add`, `pbi measure --table Sales`, etc.). The SKILL file should say: run `capabilities` first to discover the live contract; use handles not paths; check `next[]` after every mutation; always `validate` before handoff; Desktop proof is optional but listed in `capabilities` if available. Write this file in week 1 and update it with every new command.

**Priority 5: Accept `--json` anywhere in the argument list, not just before the subcommand.**
The current parser strips global flags only up to the first non-flag token. An agent will naturally write `powerbi-cli scaffold --schema s.json --out ./proj --json` and get "unknown scaffold flag: --json". This is a silent usability trap. Accept `--json` as a global flag anywhere. The fix is a two-pass parse: strip `--json` and `--format json` from the full argument list before dispatching subcommands.

**Priority 6: Add structured, machine-readable error codes on every error path.**
Current errors: `{"error": {"code": "validation_failed", "exitCode": 10, "message": "..."}}`. The `code` field is already there but not useful for branching — `validation_failed` covers everything from a missing file to a bad DAX expression. Add a typed `diagnostics[]` array with fields: `code` (e.g. `E101: missing_tmdl_table`), `severity`, `file`, `line` (where available), `object`, `message`. An agent that sees `E101` knows to run `model tables add`. An agent that sees `E200: unsafe_file_present` knows to run `validate --list-hazards`. Advertise the full code inventory in `capabilities`.

**Priority 7: Add `diff` in phase 1, not phase 8.**
An agent that mutates a project and cannot diff it has no way to verify its own work. `diff` does not need to be a full semantic diff at first — a normalized JSON summary diff between two `inspect --deep` snapshots is sufficient and trivially implementable once `inspect --deep` exists. The phased approach of "diff only after all individual commands are stable" is backward: diff is how you detect that individual commands are stable.

**Priority 8: Ship `--dry-run` on every mutation from day 1.**
Every mutation command (`model measures add`, `report visuals set-position`, etc.) must support `--dry-run` that shows what would change without writing any files. This is not optional for agents — an agent operating under uncertainty will always dry-run first. The output should be the same JSON as the real run but with `"dryRun": true` and a `"changes": [...]` array showing file-level diffs. Design the output contract once and use it everywhere.

**Priority 9: Ship a `repair` command alongside `validate`.**
Agents in a mutation loop will produce corrupt state. `validate` detects it but offers no recovery. A `repair --dry-run` command that identifies and proposes fixes for common corruption (broken PBIR references, malformed TMDL blocks, orphaned visual files) is the difference between an agent that can self-correct and an agent that gets stuck. Start with the 5 most common validation errors and a deterministic fix for each.

**Priority 10: Decouple "Desktop fixture required" from "command can ship".**
The current plan implies no mutation command is stable until a Desktop golden fixture validates it. This is too strict. The correct gate is: a command can ship as `beta` if it passes offline validation. It becomes `stable` after Desktop round-trip proof. Expose this in `capabilities` with a `stability` field on each command: `"beta"`, `"stable"`, or `"oracle-required"`. Agents can choose to use beta commands. Remove the fixture gate as a hard blocker for authoring commands.

---

## 3. Command Surface Critique

### Global Flags

The `--json` flag must be accepted anywhere in the argument list, not only as a prefix. The `--format json` alias is good; keep it. Add `--project <dir>` as a global flag that sets the default project path for all subcommands, so agents don't have to repeat the path on every call in a session.

### Missing Commands an Agent Will Try First

These don't exist yet and will produce "unknown command" errors:

```
powerbi-cli model tables list <project>
powerbi-cli model measures list <project>
powerbi-cli model measures add <project> --table Sales --name "Total" --expression "SUM(...)"
powerbi-cli report pages list <project>
powerbi-cli report visuals list <project> --page Overview
powerbi-cli report visuals set-position <project> --visual v1 --x 0 --y 0
powerbi-cli diff <before> <after>
powerbi-cli lint <project>
powerbi-cli report wireframe export <project> --out wireframe.json
```

Every one of these will be an agent's first instinct after `scaffold`. All should exist before phase 3.

### Output Contract

The `next[]` array on `scaffold` output is the single best agent ergonomics feature in the project. It must become a contract rule: **every command that mutates project state must return `next[]`** containing the exact CLI calls an agent should run next. No exceptions.

Every read command (`inspect`, `list`, `show`) must return a `handle` field on every object in the output. Handles are stable, human-readable, and accepted as arguments to mutation commands.

### Error Contract

Add to every error response:
```json
{
  "error": {
    "code": "E101",
    "symbol": "missing_tmdl_table",
    "exitCode": 10,
    "message": "...",
    "file": "SemanticModel/definition/tables/Sales.tmdl",
    "object": "Sales",
    "suggestions": ["powerbi-cli model tables add ./proj --name Sales"]
  }
}
```

### Exit Codes

Current codes are reasonable. Add:
- `11` — `lint_warnings` (lint found issues, not errors)
- `12` — `desktop_not_available` (desktop proof was requested but Desktop not found)
- `13` — `partial_success` (some files written, some failed, project may be in inconsistent state)

---

## 4. Agent Workflow Design

### Schema-to-Report (target: 5 commands)

```bash
powerbi-cli --json scaffold --schema regional-sales.schema.json --out-dir ./proj
powerbi-cli --json inspect ./proj --deep
powerbi-cli --json model measures add ./proj --table DimCustomer --name "Total Customers" \
  --expression 'COUNTROWS(DimCustomer)' --format "#,##0"
powerbi-cli --json report pages add ./proj --name "Overview" --width 1280 --height 720
powerbi-cli --json validate ./proj
```

Currently: commands 1 and 5 partially work, commands 2–4 don't exist. The `scaffold` output `next[]` should already suggest command 2.

### Style Extraction and Application (target: 3 commands)

```bash
powerbi-cli --json report themes extract ./source-proj --out corp-theme.json
powerbi-cli --json report themes apply ./target-proj --theme corp-theme.json
powerbi-cli --json validate ./target-proj
```

Not implemented. The theme is a JSON file in the report folder; extracting it is a copy with normalization. `apply` patches the report's `theme` reference and writes the file. No Desktop oracle needed.

### Measure Authoring (target: 4 commands)

```bash
powerbi-cli --json model measures list ./proj
powerbi-cli --json model measures add ./proj \
  --table Sales --name "YoY Growth" \
  --expression 'DIVIDE([This Year], [Last Year]) - 1' \
  --format "0.0%" --display-folder "KPIs"
powerbi-cli --json model measures show ./proj --table Sales --name "YoY Growth"
powerbi-cli --json validate ./proj
```

Not implemented. The TMDL format for a measure is documented and already emitted by `scaffold`; parsing and modifying it is straightforward.

### Visual Authoring (target: 5 commands)

```bash
powerbi-cli --json report visuals add ./proj --page Overview \
  --type clusteredBar --x 0 --y 0 --width 400 --height 300
powerbi-cli --json report visuals bind ./proj --page Overview --visual bar1 \
  --role Category --table Product --column Category
powerbi-cli --json report visuals bind ./proj --page Overview --visual bar1 \
  --role Values --table Sales --measure "Total Revenue"
powerbi-cli --json report visuals show ./proj --page Overview --visual bar1
powerbi-cli --json validate ./proj
```

Not implemented. The stable handle `bar1` is what makes this sequence reproducible. Without handles, an agent would need to know the internal PBIR file path.

### Validation and Proof (target: 3 commands, 1 optional)

```bash
powerbi-cli --json validate ./proj
powerbi-cli --json validate ./proj --strict
powerbi-cli --json lint ./proj
# Windows only, opt-in:
powerbi-cli --json desktop open-check ./proj
```

Command 1 works. Commands 2–4 don't. The `--strict` mode should enable schema-level JSON validation against vendored Microsoft schemas.

### Work-Machine Rebind (target: 4 commands)

```bash
powerbi-cli --json model partitions list ./proj
powerbi-cli --json model partitions set-m ./proj \
  --table Sales --partition Sales-partition \
  --m 'let Source = Sql.Database("srv", "db") ...'
powerbi-cli --json handoff rebind-plan ./proj --out rebind.json
powerbi-cli --json validate ./proj
```

Not implemented. The `handoff rebind-plan` is the core business value for the locked-down corporate environment workflow; it should be earlier in the plan.

---

## 5. Porting Recommendations

**`MinaSaad1/pbi-cli` (MIT) — Port concept, rewrite in Rust.**
The visual type catalog (which visual types exist and what roles/fields each accepts) is the most valuable artifact here. Study it to build the initial visual catalog for `report visuals add`. The semantic model command taxonomy (`tables`, `columns`, `measures`, `relationships`) maps directly to our `model *` family. Do not port Python code; rewrite from scratch. The bundled Microsoft ADOMD/TOM DLLs are separately licensed and cannot be used. Attribution in docs when borrowing the catalog concept.

**`akhilannan/pbir-utils` (MIT) — Use as test inspiration and rule reference.**
The PBIR validation rules (broken references, empty pages, duplicate visuals, orphaned filters) are the best seed for our `lint` rule pack. The wireframe concept (layout export as JSON/HTML without Desktop) is worth reimplementing independently. The sanitize operation list is a useful checklist for `repair`. Do not copy Python code; write Rust from scratch with the same behaviors as test targets.

**`microsoft/powerbi-modeling-mcp` (MIT) — Primary reference for semantic model command parameters.**
This is the most authoritative non-Desktop source for what semantic model operations exist and what their parameters are. Use it to define every flag on `model *` commands. The TOM property names map closely to TMDL keywords — validate our TMDL generation against this. This is free to study; the MCP protocol shape does not contaminate our Rust implementation.

**`microsoft/semantic-link-labs` (MIT) — Narrow reference for dependency analysis and lint rules.**
Mostly service-side; most of it is irrelevant to offline PBIP authoring. Useful: the BPA (Best Practice Analyzer) rule library to seed our `lint` rules, and the measure dependency graph concept for `model dependencies measures`. Do not port Fabric auth, service, or gateway code.

**`TabularEditor/TabularEditor` (MIT) — Best reference for semantic model concept completeness.**
The BPA rule library is the strongest public list of "what makes a semantic model bad." Use it to build the lint rule pack. The scripting model shows which operations are composable for agent workflows. The calculation group and calculation item model is the reference implementation for those complex TMDL objects. Free to study; our Rust implementation is entirely independent.

**`DaxStudio/DaxStudio` (Microsoft Reciprocal License) — Reference for DAX diagnostics only, no code.**
The reciprocal license applies at the file level, making any code copy into this MIT project a contamination risk. Use only as behavioral reference for: DAX query result formats, DAX formatting style (what "formatted DAX" looks like), and query plan diagnostic concepts. The DAX formatter is the only thing worth wanting from here, but reimplementing it is a large project; use `dax-formatter` API (external service, HTTP) or a permissive open-source DAX formatter if one exists.

**`pbi-tools/pbi-tools` (AGPL) — Feature checklist only.**
The extract/compile/convert/deploy workflow is the closest competitor. Use its command list to audit our coverage gaps. Do not read source. Do not copy JSON templates, schemas, fixture files, or examples. The AGPL means even test fixtures derived from the code are contaminated. Clean-room only: write a requirements note from reading the documentation, then implement from that note plus Microsoft's own specifications.

**`maxanatsko/pbir.tools` (custom non-commercial/no-derivatives) — Zero porting.**
The license prohibits derivatives and commercial use. Do not copy code, JSON structures, examples, UI concepts, or documentation prose. Use only to confirm that the feature domain is real (users want shell-like browsing, backup/restore, validation). Any specific behavior must be independently derived from Microsoft's PBIR documentation.

**`data-goblin/power-bi-agentic-development` (GPL) — Zero porting, behavioral signal only.**
GPL contamination risk for any incorporation. The agent workflow expectations (what prompts an agent would use, what commands it would call) confirm that the agent-first design is on the right track. Do not copy skills, JSON patterns, or workflow definitions. Derive agent workflows independently from our own SKILL.md design.

---

## 6. Missing Power BI Features

### Missing Entirely and Critical

**Stable visual and page handles.** Without them, no agent can reliably reference a specific object across two consecutive commands. This is an architectural gap, not a missing command.

**`model measures add/update/delete`.** The most fundamental semantic model operation. Missing from the current command surface.

**`inspect --deep`.** The current `inspect` counts objects. Agents need full structured output: tables[], measures[], pages[], visuals[{handle, type, position, bindings[]}], relationships[], hazards[].

**`report pages list/show/add/delete/reorder`.** Scaffolded pages exist on disk but cannot be inspected or mutated without knowing PBIR internals.

**`--dry-run` on all mutations.** Not present anywhere.

**`lint`.** The plan recognizes this as essential but places it too late. Basic lint (empty pages, broken references, oversized visuals, missing data type, unused measures) should exist in phase 1.

**Machine-readable diagnostic codes.** Currently absent from the error contract.

**SKILL.md / agent robot guide.** The most critical missing deliverable for agent ergonomics.

### Under-Prioritized

**Measure authoring** is listed as phase 3 step 5–6, but it should be the first mutation command shipped. No other mutation matters more.

**`handoff rebind-plan`** is in phase 5 but is the core business case for the corporate-locked workflow. It should be phase 2.

**`report wireframe export`** is in phase 4 but agents need spatial layout understanding from day 1. A simple JSON with `{page, visuals: [{handle, type, x, y, w, h, title}]}` is trivially derived from existing `inspect` data and should ship with `inspect --deep`.

**Schema manifest versioning.** The schema format will change. There is no `schemaVersion` field today. When it changes, existing agent schemas will silently produce wrong projects.

### Over-Prioritized

**Calculation groups** appear in the roadmap at phase 3 equivalence. They are an advanced semantic model feature used by 5% of reports. Move to phase 7+.

**Cultures and translations** at phase 3 equivalence. Move to phase 7+.

**Perspectives** at phase 3 equivalence. Move to phase 6+.

**The Desktop oracle as a gate on authoring commands.** It is correctly sequenced for the proof layer but should not block `beta` command shipping.

### Mis-Framed

**DAX static lint** is treated as "defer to optional bridge." DAX expressions in measures can be statically linted offline (undefined column references, unbalanced parentheses, type mismatches in some cases) without a live engine. A tree-sitter grammar for DAX exists. Static DAX lint should be in phase 3, separate from execution.

---

## 7. Testing Strategy

### Required Now (Before New Commands)

**Snapshot tests for all scaffold output.** Add `insta` or a manual golden-file comparison. Every generated file — `.pbip`, `definition.pbir`, every TMDL file, every PBIR visual file — should have a golden snapshot. Before modularizing, add these snapshots. This catches regressions during the refactor.

**Exit code tests.** Every error path must be tested for the correct exit code, not just the error message. Test that `--schema missing.json` exits with code 3, that a schema with empty `tables` exits with code 2, that a project with a `cache.abf` exits with code 10.

**TMDL escaping unit tests.** Column names with spaces, quotes, special characters, and Unicode must produce valid TMDL. `"Größenklasse"` is already in the regional-sales schema and must be tested explicitly for correct TMDL quoting.

**M literal unit tests.** Date literals, decimal values, null handling, and string escaping in `#table(...)` expressions. These are the most fragile part of the generator.

### After Each New Command

**Metamorphic tests.** For every mutation command: `scaffold(spec_with_X)` must produce the same project state as `scaffold(spec_without_X)` followed by `command_add_X`. This catches generator/mutator inconsistency that would otherwise require Desktop to detect.

**Round-trip inspect tests.** `scaffold → inspect → serialize → compare` must be idempotent. Scaffold, inspect, serialize the summary to JSON, inspect again, compare. If inspect is stateful, it's broken.

**Idempotency tests.** `scaffold --force` on the same schema must produce byte-identical files (excluding any volatile fields). This requires removing all sources of non-determinism from the generator.

### Oracle Tests

**One fixture per visual type, not one fixture total.** The plan mentions "one page with a card" but the fixture strategy should produce a Desktop-authored golden for: card, table, matrix, clustered bar, clustered column, line, combo, slicer, text box, page filter, report filter. Each fixture goes into `testdata/desktop-golden/<type>/` with a normalized `inspect --deep` summary alongside it.

**Corruption recovery tests.** Deliberately corrupt a project (delete a PBIR file, write invalid JSON, break a TMDL reference) and verify that `validate` produces the correct diagnostic code and `repair --dry-run` proposes the right fix.

**Offline safety regression tests.** Run `validate` on every generated project and assert that no `cache.abf`, `localSettings.json`, `credentials`, or real data footprint is present. This should be a mandatory CI step.

### Fuzz

**Fuzz the schema manifest parser.** Random valid-looking JSON with unexpected fields, missing required fields, extremely long names, Unicode in unexpected places, deeply nested structures. The parser must not panic. It must return clean `invalid_args` errors with useful messages.

---

## 8. Risk Register

**Risk 1: PBIR visual binding schema is underdocumented and changes silently.**
The `queryState` structure in PBIR visual files is not formally published. Desktop can change it in any update. Generated `queryState` that worked with Desktop 2.136 may silently fail with 2.137. **Mitigation:** pin the Desktop version in CI; normalize and hash the volatile portions of golden fixtures; fail loudly when a fixture hash changes rather than silently accepting drift.

**Risk 2: Single `main.rs` becomes unmaintainable before modularization.**
The file is already several hundred lines and has all struct definitions, all command implementations, and all output generation mixed together. Adding 10 new `model *` subcommands to this file will make it 3000+ lines and cause merge conflicts on every PR. **Mitigation:** modularize in the very next commit, before any new features. Use snapshot tests to lock current behavior first.

**Risk 3: `--json` flag position breaks agent integration.**
Agents generating CLI calls will put `--json` after the subcommand: `powerbi-cli scaffold --schema s.json --out ./proj --json`. This currently fails. An agent that discovers this empirically will produce a workaround that may not be documented, creating fragility. **Mitigation:** fix the parser to accept `--json` anywhere; this is a two-line change in `parse_global_flags`.

**Risk 4: No stable IDs means mutations invalidate agent state.**
If a visual's file name changes (e.g., during a scaffold with `--force`), all agent references to that visual by file path are broken. **Mitigation:** assign stable UUIDs or deterministic handles at scaffold time and preserve them as a first-class field in the project metadata. Store a `powerbi-cli.meta.json` in the project root that maps handles to current paths.

**Risk 5: Desktop oracle is Windows-only and hard to run in CI.**
If Desktop golden fixtures are required before any command is marked stable, and Desktop CI is hard to set up and maintain, the entire command surface stays in `beta` indefinitely. **Mitigation:** separate the `beta`/`stable` distinction clearly in `capabilities`; let agents use `beta` commands; run Desktop CI only on a self-hosted Windows runner on a weekly schedule, not on every PR.

**Risk 6: License contamination from research repos.**
A developer who reads quarantined repos and then writes implementation from memory risks unconscious copying. The quarantine rules are documented but not enforced. **Mitigation:** add a pre-commit hook that checks for patterns from quarantined repos; require the two-person clean-room protocol (researcher vs. implementer) for any feature inspired by a quarantined tool; add a CI job that verifies no quarantined file paths appear in the commit history.

**Risk 7: Schema manifest format will break without versioning.**
Schema manifests have no `schemaVersion` field. When the format adds support for filters, themes, or conditional formatting in the manifest, agents writing schemas today will silently produce incomplete projects. **Mitigation:** add `"schemaVersion": "1"` immediately; write a migration path in `scaffold` that detects the version and applies defaults for missing fields.

---

## 9. First 5 Implementation Slices

These are ordered by agent value delivered, not by architectural cleanliness.

**Slice 1: Fix `--json` flag position and add `inspect --deep`.**
Fix the two-line parser change so `--json` is accepted anywhere. Then extend `inspect` to return a complete structured summary: `tables[]`, `columns[]`, `measures[]`, `relationships[]`, `pages[]`, `visuals[{handle, type, position, bindings[]}]`, `hazards[]`, `next[]`. This is the foundation every subsequent agent workflow depends on. No new file needed; extend the current `inspect_command` function. Add snapshot tests before touching anything.

**Slice 2: Modularize `main.rs`.**
After adding snapshot tests that lock current behavior, split into: `src/cli.rs` (arg parsing, routing), `src/output.rs` (CliOutput, CliError, exit codes), `src/manifest.rs` (DashboardSpec and friends), `src/project.rs` (resolve, path normalization), `src/tmdl.rs` (TMDL generation and parsing), `src/pbir.rs` (PBIR generation and parsing), `src/validate.rs`, `src/inspect.rs`, `src/scaffold.rs`. No behavior changes — just move code. Run snapshot tests before and after to confirm nothing regressed.

**Slice 3: `model measures list/show/add/update/delete`.**
TMDL measure files live in `SemanticModel/definition/tables/<TableName>.tmdl`. Parse the measure blocks from TMDL text (the format is simple enough for a hand-written parser at this stage). `list` returns `[{handle, table, name, expression, formatString, displayFolder}]`. `add` writes a new measure block. `update` replaces the expression. `delete` removes the block. Every mutation returns `next[]` with `inspect --deep` and `validate`. Add metamorphic tests: `scaffold(spec_with_measure)` must match `scaffold(spec_without) + measures add`.

**Slice 4: `report pages list/show/add/delete/reorder` with stable handles.**
PBIR page files are JSON in `report/definition/pages/`. `list` returns `[{handle, name, displayName, order, width, height, isActive}]`. `add` writes a new page JSON file and updates `pages.json`. `delete` removes the file and updates `pages.json`. `reorder` updates the `ordinal` field. The handle is the page display name normalized to snake_case. Add it to `pages.json` as a stable identifier field. Return `next[]` with `report visuals list --page <handle>`.

**Slice 5: SKILL.md and structured error codes.**
Write `skills/powerbi/SKILL.md` (mirroring `ooxml-cli`'s SKILL structure): discovery protocol (always run `capabilities` first), mutation protocol (dry-run → apply → validate → inspect), handle protocol (use handles not paths), handoff protocol (validate → handoff check before moving). Simultaneously, change `CliError` to carry a typed error code enum, add `diagnostics[]` to the JSON error output, and advertise the code inventory in `capabilities`. This slice has no new features but dramatically improves agent reliability with existing commands.

---

## 10. Uncomfortable Truths

**The plan's phasing reflects what's architecturally satisfying, not what unblocks agents.** Modularizing before adding commands is the right instinct, but the actual first useful agent operation after scaffold — adding a measure — is not gated on modularization. It's gated only on someone implementing it. Reorder the backlog to prioritize agent-useful commands and do the modularization in parallel.

**The Desktop oracle gate will kill velocity if applied universally.** "No command is stable until a Desktop fixture validates it" sounds rigorous but in practice means 0–1 new stable commands per month if Desktop CI setup is non-trivial. Measures don't need a Desktop oracle to be correct — TMDL is a text format with a documented grammar. Relax the gate: Desktop proof for visual bindings and formatting, offline validation for semantic model operations.

**`MinaSaad1/pbi-cli` already covers most of the planned command surface in Python+.NET.** It targets agents, covers semantic models and PBIR report files, handles filters, formatting, bookmarks, and themes. The differentiation — single binary, cross-platform, schema-first, offline-safe, no .NET runtime — is real and valuable, but it must be protected by staying in that lane. The moment `powerbi-cli` adds a Windows-only .NET dependency for Desktop automation, the differentiation collapses.

**The schema manifest format will get too complex for a single JSON file.** A real-world manifest with hundreds of columns/measures across many tables can easily exceed 300KB. Agents writing schemas will split them. The CLI should support schema composition (`$include`, `$ref`, or a directory-based manifest) before agents hit this wall. This is not in the roadmap at all.

**The `next[]` array is the best ergonomics feature in the project and it's only on `scaffold`.** It is not yet a contract. Make it mandatory: every command — read or write — that changes project state or discovers something actionable must return `next[]`. This single rule, enforced by a code review checklist, will make the difference between an agent that gets stuck and an agent that chains operations correctly.

**There is no error recovery path.** If an agent's mutation produces a malformed TMDL file, `validate` reports it but offers no path forward. An agent in a loop will call `validate`, see the error, attempt another mutation (likely making it worse), and spiral. The `repair` command is not a nice-to-have; it is the mechanism by which agents exit bad states. Without it, agents using this tool autonomously will require human intervention to recover.

**The project has no published artifact.** There is no `cargo install powerbi-cli`, no GitHub release, no pre-built binary. An agent that discovers this tool by searching can't install it without a Rust toolchain. This is fine for now but should be planned: GitHub Actions release workflow producing static binaries for Windows, Linux, macOS by the time the command surface reaches phase 3.
