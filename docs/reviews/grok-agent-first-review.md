I'll read the project docs, reference CLI, and current codebase to ground a concrete independent review.
# Independent Review: Agent-First `powerbi-cli` Plan

## 1. Verdict

**Fundamentally right, but mis-sequenced and not yet agent-first in practice.**

The strategic bet is correct: PBIP/PBIR/TMDL compiler + inspector + validator + Desktop oracle, not PBIX synthesis or live Desktop automation as the core path. The offline-safe corporate handoff niche is real and defensible.

The gap is execution order and contract completeness. The plan describes an `ooxml-cli`-class agent workbench, but the shipped binary is still a 2,200-line monolith with five commands, shallow `inspect`/`validate`, no schema enforcement despite schema URL constants, no stable handles, no `lint`/`wireframe`/`handoff check`, and no Desktop oracle harness. Worse, `scaffold` already emits PBIR `queryState` bindings without Desktop round-trip proof — which directly contradicts the plan’s own “never invent bindings by memory” rule.

The roadmap is also **too broad on future commands** and **too timid on near-term agent ergonomics** (wireframe, lint, structured mutation responses, canonical skill). Phase ordering should be: **agent contract → oracle harness → golden corpus → typed mutations**, not “modularize, then eventually Desktop fixtures.”

---

## 2. Top 10 Plan Changes (priority order)

1. **Ship the agent contract before new features.** Mirror `ooxml-cli`: every mutation returns `validateCommand`, `inspectCommand`, `readbackCommand`, optional `desktopOpenCheckCommand`; capabilities must document flags, `requiresOutput`, `opCompatible`, and example JSON. Agents cannot discover `--page` / `--visual` syntax from today’s capabilities.

2. **Move Desktop oracle to Phase 1, not Phase 2+.** `desktop open-check` is the only proof that matters for PBIR. Without it, `validate` will pass projects Desktop rejects (you already have `build/desktop-proof-oracle/` artifacts — formalize them). Agents need a boolean `desktopProof: { available, passed, level }` field, not prose in `next`.

3. **Freeze binding expansion until golden corpus exists.** Current card/table/chart `queryState` generation is a liability until each visual family has a Desktop-authored fixture + normalized expected summary. Agents benefit from **refusal** (`binding_catalog_missing`) more than speculative bindings.

4. **Promote `inspect --deep`, `lint`, and `report wireframe export` ahead of full CRUD.** Agents reason on summaries and layout previews, not raw `visual.json`. `pbir-utils` wireframe + lint are high-leverage, MIT, and offline-safe — build Rust equivalents before `model roles` or `calculation-groups`.

5. **Introduce stable handles now.** Emit `project`, `page`, `visual`, `table`, `measure` selectors in JSON (e.g. `page:ReportSectionOverview`, `visual:VisualContainerTotalRevenueCard685a891a2d`). Agents should never parse `pages.json` folder names by guesswork.

6. **Unify mutation output discipline: `--out-dir` for projects, `--in-place` explicit.** `scaffold --out-dir` is good; future `model measures add` must follow the same rule. Add `--dry-run` on every mutator. Agents need predictable filesystem semantics.

7. **Collapse the planned command surface for v0.x.** Drop or defer: `model cultures/translations/calculation-groups`, `report annotations`, `dax query`, `desktop reload`, `from-template` (merge into `clone-template`). Ship a **thin stable core** agents can memorize via `capabilities --for model` rather than a 80-command wishlist.

8. **Elevate `handoff check` / `handoff rebind-plan` to Phase 1.** `POWERBI_HANDOFF.md` is already generated; expose machine-checkable handoff JSON. This is the unique corporate workflow — more valuable than bookmarks.

9. **Vendor Microsoft JSON schemas and actually validate.** Constants like `VISUAL_CONTAINER_SCHEMA` exist in `main.rs` but validation is parse-only. Agents trust `validate --strict`; without schema gates it is theater.

10. **Rewrite `skills/powerbi-cli/SKILL.md` to ooxml depth.** Add discovery loop, proof matrix, handle rules, cold-start wrapper, and “trust capabilities over docs.” Today’s skill is 70 lines vs ooxml’s 350+; agents will fall back to stale README examples.

---

## 3. Command Surface Critique

### What works
- Top-level verbs: `scaffold`, `inspect`, `validate`, `capabilities`, `doctor` — agents will guess these.
- Global `--json` / `--format json`.
- Exit codes: `0`, `2`, `3`, `10`, `70` — good skeleton.
- `scaffold` returns `next` follow-ups (partial ooxml pattern).

### What’s wrong

| Issue | Agent impact |
|---|---|
| No `help <command>` or per-command capability entries with flags | Agents hallucinate `--strict`, `--deep`, `--page` |
| Errors on stderr, success on stdout — good — but mutations don’t use structured `error.code` taxonomy beyond 4 codes | Can’t branch retry logic |
| No `--project` alias; path is positional | Ambiguous when multiple `.pbip` exist |
| `capabilities --for` does substring match on tags | `capabilities --for measure` won’t find future `model measures` |
| No `list` top-level command | Agents will try `powerbi-cli list pages` and fail |

### Proposed contract (decide now)

**Global flags**
```text
--json | --format json          # stdout JSON
--quiet                         # suppress stderr diagnostics except errors
--project <dir|.pbip>           # canonical project selector (also positional)
```

**Mutation guards (all mutators)**
```text
--out-dir <dir>                 # default for project-scoped writes
--in-place                      # explicit; requires --confirm-project <name>
--dry-run                       # print plan JSON, no writes
--force                         # overwrite policy (already on scaffold)
```

**Exit codes (extend)**
```text
0   success
2   invalid_args
3   file_not_found
10  validation_failed
11  lint_failed
20  mutation_refused          # unsupported visual type, missing catalog entry
30  oracle_unavailable        # Desktop not installed / not Windows
40  oracle_failed             # Desktop rejected project
70  unexpected
```

**Commands agents will guess first (must exist or redirect)**

| Guess | Should map to |
|---|---|
| `generate` / `create` | `scaffold` (alias or clear error with suggestion) |
| `list tables` / `tables list` | `model tables list` |
| `add measure` | `model measures add` |
| `add visual` / `chart` | `report visuals add` |
| `bind` | `report visuals bind` |
| `theme` | `report themes show` |
| `lint` | `lint` |
| `diff` | `diff` |
| `export` | `report wireframe export` or `inspect --deep` |
| `open` / `compile` | **Refuse** with message: “Use `desktop open-check`; PBIX compile not supported.” |
| `connect` | **Refuse** in core CLI; optional `desktop-bridge` plugin only |

**Structured mutation response (required)**
```json
{
  "ok": true,
  "project": { "dir": "...", "pbip": "...", "handle": "project:RegionalSales" },
  "changed": [{ "kind": "measure", "handle": "measure:FactSales[Umsatz Übersicht]" }],
  "validateCommand": "powerbi-cli --json validate --strict ...",
  "inspectCommand": "powerbi-cli --json inspect --deep ...",
  "desktopOpenCheckCommand": "powerbi-cli --json desktop open-check ...",
  "readbackCommand": "powerbi-cli --json model measures show --measure \"Umsatz Übersicht\" ..."
}
```

**Rename for consistency**
- `clone-template` → keep; drop `from-template` (duplicate).
- `sanitize --apply` → `sanitize --in-place --confirm-project` (explicit danger).

---

## 4. Agent Workflow Design

### Schema → report (shortest happy path)
```text
powerbi-cli --json capabilities --for scaffold
powerbi-cli --json scaffold --schema examples/sales.schema.json --out-dir ./build/sales --force
# run validateCommand + inspectCommand from response
powerbi-cli --json inspect --deep ./build/sales
powerbi-cli --json report wireframe export ./build/sales --out ./build/sales/wireframe.json
powerbi-cli --json handoff check ./build/sales
# Windows opt-in:
powerbi-cli --json desktop open-check ./build/sales
```

### Style extraction / application
```text
powerbi-cli --json clone-style --from ./corp-template.pbip --out ./style/theme.json
powerbi-cli --json scaffold --schema ./schema.json --out-dir ./build/new --force
powerbi-cli --json apply-style --style ./style/theme.json --project ./build/new --out-dir ./build/styled
powerbi-cli --json lint ./build/styled --rules builtin/report-style
powerbi-cli --json validate --strict ./build/styled
```

### Measure authoring
```text
powerbi-cli --json model measures list --project ./build/sales
powerbi-cli --json model measures add --project ./build/sales --table FactSales \
  --name "Revenue YTD" --expression "TOTALYTD(SUM(...), ...)" --format-string "$#,0" \
  --out-dir ./build/sales-edited
powerbi-cli --json dax lint --expression "..." --table FactSales
# run readbackCommand from response
powerbi-cli --json model dependencies measures --project ./build/sales-edited
```

### Visual authoring
```text
powerbi-cli --json capabilities --for visual
powerbi-cli --json report pages list --project ./build/sales
powerbi-cli --json report visuals add --project ./build/sales --page page:ReportSectionOverview \
  --type lineChart --title "Revenue Trend" --x 32 --y 120 --width 480 --height 320 \
  --out-dir ./build/sales-v2
powerbi-cli --json report visuals bind --project ./build/sales-v2 \
  --visual visual:... --binding category:DimDate[Month] --binding values:FactSales[Revenue] \
  --out-dir ./build/sales-v3
powerbi-cli --json report wireframe export ./build/sales-v3
```

### Validation / proof
```text
powerbi-cli --json validate --strict ./project
powerbi-cli --json lint ./project
powerbi-cli --json handoff check ./project
# Claims "Desktop compatible" ONLY after:
powerbi-cli --json desktop open-check ./project
powerbi-cli --json desktop save-check ./project --out-dir ./proof/roundtrip
powerbi-cli --json diff ./project ./proof/roundtrip
```

### Work-machine rebind
```text
powerbi-cli --json handoff rebind-plan ./project --templates ./templates/sqlserver.json
powerbi-cli --json source-template list --project ./project
powerbi-cli --json model partitions set-sql-template --project ./project \
  --table DimCustomer --template sqlserver:DimCustomer --out-dir ./project-rebind-plan
# Human at work executes plan in Desktop; then:
powerbi-cli --json handoff check ./project   # must fail if still dummy M
```

---

## 5. Porting Recommendations

| Repo | Port directly | Reimplement in Rust | Test inspiration only | Ignore |
|---|---|---|---|---|
| **MinaSaad1/pbi-cli** (MIT) | Nothing wholesale — Python + .NET TOM coupling | Command taxonomy (`visual bind` flags, bulk ops shape), JSON output patterns, agent skill structure, visual type aliases | Live Desktop `pbi connect` workflows | Bundled Microsoft DLLs; Windows-only connect as core dependency |
| **akhilannan/pbir-utils** (MIT) | Nothing — Python | **Wireframe visualizer**, **rule-based lint**, sanitize actions (unused measures, folder standardization), filter sort/update, interaction disable, metadata extract summaries | YAML rule pack *ideas* (rewrite rules in Rust JSON) | Web UI (`serve`); update-check banner noise |
| **microsoft/powerbi-modeling-mcp** (MIT) | MCP tool schema as capabilities reference | Semantic model operation scope checklist (tables, measures, relationships, partitions, roles, calc groups) | — | MCP server itself; Fabric auth paths |
| **microsoft/semantic-link-labs** (MIT) | — | BPA/dependency analysis concepts for `model dependencies` | Service refresh/gateway ops | Fabric-only deployment |
| **TabularEditor** (MIT) | — | BPA rule categories, DAX dependency graph semantics, TMDL object model mental map | `.bim` test data patterns (not files) | Desktop app UI; TOM scripting host |
| **DaxStudio** (MS-RL) | **No code** | DAX format/lint UX goals | Query performance patterns | Any source file |
| **pbi-tools** (AGPL) | **No code** | PBIP path resolution behavior, extract/compile *requirements* | Interop test scenarios (clean-room) | Extract/compile implementation; deploy |
| **maxanatsko/pbir.tools** (NC/ND) | **Nothing** | Shell UX ideas (backup before mutate) as `--backup` flag behavior | Validation categories | All code/JSON/templates |
| **data-goblin/power-bi-agentic-development** (GPL) | **Nothing** | Agent workflow expectations (inspect→mutate→validate loop) | Skill section headings | Skills, examples, JSON |

**High-value reimplementation targets from permissive repos**
1. `pbir-utils` wireframe + lint (offline, agent-visible layout).
2. `pbi-cli` visual bind CLI flag shape (not PBIR internals).
3. Tabular Editor BPA rule *names* and severities for `lint --rules builtin/bpa-lite`.

**License consequences stated plainly**
- Porting `pbir-utils` Python into the repo as a subprocess: allowed (MIT), but defeats single-binary Rust goal; use only as temporary oracle for differential tests, not production path.
- AGPL `pbi-tools` behavior tests: fine if tests are written from spec + your fixtures, not from their sources.
- GPL agent skills: reimplement workflow prose from scratch (you already have a thin skill).

---

## 6. Missing / Mis-Prioritized Power BI Features

| Feature | Status | Priority call |
|---|---|---|
| **Measures** | Scaffold only | **P0** — `model measures add/update` + DAX reference lint |
| **Relationships** | Scaffold only | **P0** — inactive/cross-filter validation |
| **Calculated columns** | Not planned early enough | **P1** — after measures; needed for real models |
| **Visual types** | Card/table/chart scaffolded without oracle | **P0 freeze** until golden catalog |
| **Visual binding catalog** | Planned Phase 5 — too late | **P0** — blocks all visual authoring claims |
| **Conditional formatting** | Phase 6+ | **P3** — defer |
| **Themes** | Phase 6 in porting-analysis | **P1** — agents need `clone-style`/`apply-style` for corporate look |
| **Filters/slicers** | Phase 7 | **P2** — after page/visual CRUD |
| **Bookmarks** | Phase 7 | **P3** — defer |
| **Interactions** | Phase 7 | **P2** — `pbir-utils` has simple bulk disable; quick win |
| **Source templates / rebind** | Phase 5 | **P1** — core differentiator vs `pbi-cli` |
| **Desktop oracle** | Planned, not implemented | **P0** — non-negotiable |
| **Matrix, combo, map, custom visuals** | Mentioned in catalog | **P4+** — refuse until native catalog proof |
| **RLS roles** | Later | **P3** for dashboard authoring; **P1** if target is enterprise semantic models |
| **Calculation groups** | Later | **P4** |
| **DAX query execution** | Optional bridge | **Defer** — static lint is enough for agents offline |

**Over-prioritized in plan:** cultures, translations, perspectives, annotations, `dax query`, Fabric bridges, batch `apply` before individual mutators work.

**Under-prioritized:** wireframe, lint, handoff machine checks, theme/style, partition templates, measure dependency graph.

---

## 7. Testing Strategy

### Always-on CI (cross-platform)
1. **Scaffold golden summaries** — `sales.schema.json` and `regional-sales.schema.json` → normalized `inspect --deep` JSON snapshots (scrub volatile hashes/IDs).
2. **TMDL text goldens** — per-table `.tmdl` for quoting, `measure` blocks, relationship syntax.
3. **M literal fuzz** — strings with quotes, umlauts, nulls, dates, datetimes, scientific notation, wide decimals (property tests).
4. **Path normalization** — `clean_relative_path` adversarial inputs (`..`, `C:\`, mixed slashes).
5. **Offline hazard scanner** — synthetic tree with `cache.abf`, `.pbix`, `localSettings.json` must fail.
6. **Schema validation gate** — once vendored, every generated JSON file must pass `jsonschema` against pinned Microsoft schemas.
7. **Binding catalog conformance** — for each golden visual fixture, `scaffold`/`bind` output must match normalized `queryState` hash.

### Desktop oracle (Windows, `POWERBI_DESKTOP_ORACLE=1`)
1. **open-check** — no “Issues were found” dialog; title bar contains project name + “Power BI Project”.
2. **save-check** — round-trip save to temp dir; semantic `diff` empty for the normalized summary.
3. **Per-visual-family matrix** — card, table, line, bar, slicer, page filter, themed report (one fixture each).
4. **Failure capture test** — intentionally broken `visual.json` must record Desktop error text into `testdata/oracle-failures/`.

### Metamorphic tests
1. `inspect → scaffold from exported schema manifest copy → inspect` counts stable.
2. `model measures add → delete → add` restores identical normalized measure summary.
3. `report visuals set-position` then negate delta returns original wireframe bounds.
4. `clone-style → apply-style` twice is idempotent on theme hash.

### Differential / clean-room tests
1. Run `pbir-utils validate` and `pbir-utils visualize` on golden fixtures (subprocess, dev-only) vs `powerbi-cli lint`/`wireframe` — compare high-level counts, not code.
2. Never commit quarantined repo files; only your Desktop-generated fixtures.

### Agent contract tests
1. `capabilities` JSON schema snapshot.
2. Every mutation integration test asserts presence of `validateCommand` and `readbackCommand`.
3. `cargo test --test cli_smoke` already good — extend to `--strict` failures and exit code `10`.

---

## 8. Risk Register

| Risk | Severity | Mitigation |
|---|---|---|
| **PBIR `queryState` fragility** | Critical | Golden catalog + refuse unknown visual types; no more invented bindings |
| **Validators pass, Desktop fails** | Critical | `desktop open-check` before stability claims; capture failures as tests |
| **Monolithic `main.rs` regression rate** | High | Modularize with snapshot tests *before* any feature; enforce module boundaries |
| **Competitive overlap with `pbi-cli`** | High | Own offline/cross-platform/handoff niche; don’t replicate `pbi connect` |
| **Schema manifest drift** | Medium | Check in `powerbi-cli.manifest.copy.json`; `handoff check` verifies manifest ↔ project |
| **AGPL/GPL contamination** | Medium | Keep research outside repo; behavior-only notes; legal review before importing any snippet |
| **Agent skill staleness** | Medium | Capabilities-driven skill; contract version in `version` output |
| **7k-line example schemas** | Medium | Capabilities should point to `sales.schema.json`; keep large samples as integration-only |
| **Windows-only proof gaps on Linux CI** | Medium | Two-tier CI: fast everywhere + nightly Windows oracle job |
| **TMDL writer incomplete** | Medium | Parse-first for mutations; round-trip read TMDL → edit → write |

---

## 9. First 5 Implementation Slices

1. **Agent contract hardening (no new features)**  
   Extend `capabilities` with full flag docs; standardize mutation `validateCommand`/`inspectCommand`/`readbackCommand`; add `help <cmd>`; expand `skills/powerbi-cli/SKILL.md` proof matrix. *Leverage: every future command is usable day one.*

2. **Modularize + snapshot lock**  
   Split `main.rs` into `cli`, `manifest`, `pbir`, `tmdl`, `validate`, `inspect`, `output`; zero behavior change; golden tests for `sales` + `regional-sales` scaffold output. *Leverage: unblocks parallel agent work.*

3. **`inspect --deep` + stable handles + `report wireframe export`**  
   Structured pages/visuals/measures/relationships/bindings/hazards; wireframe JSON for layout reasoning. *Leverage: agents can author without reading PBIR raw files.*

4. **`desktop open-check` + first golden corpus (3 fixtures)**  
   Blank+dummy table, one card, one bound line chart; normalized summary JSON; opt-in CI job. *Leverage: ends false compatibility claims; validates existing scaffold bindings.*

5. **`model measures list/show/add/update` + `dax references` + `lint` (builtin/offline-safety)**  
   First real mutator family with `--out-dir`, `--dry-run`, readback proofs. *Leverage: completes schema→scaffold→iterate loop for real dashboards.*

---

## 10. Uncomfortable Truths

1. **`pbi-cli` already wins the “agent Power BI CLI” mindshare** on Windows with a broader command surface and shipped skills. `powerbi-cli` must not try to out-feature it; it must out-discipline it on offline safety, cross-platform CI, deterministic compile, and honest proof.

2. **You are already violating your own golden-fixture rule.** `scaffold` emits `queryState` for multiple visuals in the wider archetype fixtures without documented Desktop round-trip proof. That is exactly how ooxml’s `definedNames` class of bugs appears in Power BI form.

3. **`validate` is weaker than the docs imply.** JSON parse + file presence ≠ schema validation. Agents will over-trust it unless you rename current behavior `validate --basic` and reserve `--strict` for real gates.

4. **The planned 80-command surface is a liability.** Agents do not need `model cultures delete` in year one; they need five commands that always return replayable proof commands.

5. **PBIR is a hostile format for programmatic authoring.** The viable pattern is: Desktop creates skeleton visuals → CLI clones/normalizes/binds within a catalog → oracle proves. “Compiler from schema manifest” works for TMDL; it only partially works for PBIR.

6. **Batch `apply --ops` is seductive and should stay late.** Agents need atomic, inspectable commands first; operation plans are for human/agent review of *known-good* primitives, not a substitute for designing them.

7. **A huge schema example is agent-hostile.** It’s a great stress test but a terrible discovery entry point. Capabilities should never default agents to an oversized example schema.

8. **Rust single-binary is a real advantage only if you ship it.** A modularized 2,200-line file that grows to 15,000 lines without oracle tests is worse than `pbir-utils` as a sidecar.

9. **Licensing caution is correct but can become an excuse.** MIT `pbir-utils` wireframe/lint is the obvious next port target; quarantine should not delay that.

10. **Without Windows oracle CI within weeks, the project remains a file generator, not a Power BI tool.** Desktop is the oracle — treat oracle harness as product infrastructure, not a nice-to-have test appendix.

---

**Bottom line:** Keep the compiler/oracle vision. Re-sequence to **contract → proof → catalog → mutate**. Narrow the command surface, deepen `inspect`/`lint`/`wireframe`/`handoff`, and stop expanding visual binding until Desktop goldens exist. That is the fastest path to an agent-first CLI that is actually trustworthy, not merely ambitious.
