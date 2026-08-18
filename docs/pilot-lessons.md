# Lessons from the first production pilot (2026-08)

A nine-page analytical PBIP was authored and hardened end to end with this
CLI plus direct file editing: five KPI/trend pages on a star schema, a
concentration (Pareto) page with helper axes, two statistical watchlist
pages backed by refresh-time M analytics, and an aggregate-only diagnostics
page. Every change was gated through `validate --strict --backend all` and
proven in Power BI Desktop against a scaled synthetic fixture. This document
records what the CLI did well, what it lacked, and which patterns deserve
first-class support. It is the authoritative backlog input from real usage.

## What the CLI carried

- **Validation as the gate.** Native + official Microsoft validator caught
  real authoring mistakes before Desktop did: the 50-character filter-name
  limit, stale visual-interaction references after a page clone, role-kind
  mismatches on a scatter chart, a missing page `name` property. This loop
  (edit → `validate --strict --backend all` → fix) was the backbone of the
  whole pilot.
- **Visual-level Top-N filters.** `report filters add --scope visual --top N
  --by <measure>` was used eleven times as a *performance guard* pattern:
  cap the axis/series population before expensive measures evaluate. This is
  now the standard scaling pattern for the pilot's charts.
- **Inspection.** `report visuals list`, `report filters show`, `model dax
  lint`, and `capabilities --for` were the discovery tools of choice.

## Fixed in the CLI during the pilot

- DAX reference lint now accepts extension-column aliases declared in
  `GROUPBY` and `SUMMARIZE` tails (previously false-positive
  `dax.reference_missing_measure`, which blocked the strict preflight of
  `desktop open`). Regression test added.
- The official report validator's `succeededWithWarnings` result is now
  treated as success with surfaced warnings instead of `protocol_failed`.

## Feature gaps found (prioritized backlog)

1. **Visual authoring coverage.** Hand-written JSON was required for
   `lineChart` (Category/Series/Y), `scatterChart` (Category/X/Y/Size),
   `tableEx` (multi-column Values), `hundredPercentStackedColumnChart`, and
   measure-bound `card`. Acceptance: `report visuals add` supports these
   shapes and emits role-correct projections (see item 3).
2. **Page clone command.** Cloning a page is the dominant authoring move for
   consistent multi-page reports. A `report pages clone` should: copy the
   folder, rewrite visual container names with a prefix, rewrite the page
   `name`/`displayName`, regenerate or length-guard filter names (Desktop
   hard limit: 50 chars), and retarget or drop `visualInteractions` whose
   endpoints do not exist on the new page. Both of the pilot's two
   post-clone validation failures would have been prevented by owning this.
3. **Runtime-parity role rules.** Desktop rejects bare (grouped) columns in
   scatter `X`/`Y` when a `Category` is present and rejects the `Details`
   role name entirely; the validator caught `Size`/`Details` but let bare
   `X`/`Y` pass. Tighten scatter rules to match Desktop runtime behavior:
   require `Aggregation`-wrapped columns or measures in X/Y/Size when
   Category is bound.
4. **Offline synthetic source swap as a command.** The pilot's QA recipe is
   manual but mechanical: inject shared M expressions that generate
   deterministic synthetic tables, then replace each partition's shared
   `Database = <connector>(...)` step with a
   `#table({"Schema","Item","Data"}, ...)` navigation shim so every
   downstream M step runs unchanged. Formalize as e.g. `workflow synthesize
   --project <p> --out-dir <qa>` with row-scale options. This turned out to
   be the single highest-value QA capability: it made live Desktop proofs
   (including performance reproductions) possible without any real data.
5. **M hygiene lint.** An unbuffered M analytics query hung Desktop's load
   for over eight minutes on only thousands of rows because lazily evaluated
   intermediates were referenced repeatedly (joins, multiply-sorted ranking
   tables) and re-computed multiplicatively. Wrapping reused intermediates in
   `Table.Buffer` restored normal load times. Add at minimum a documented
   guidance note; ideally a lint that flags `let` steps referenced more than
   once downstream of a non-foldable source without a buffer.
6. **Sort authoring.** `sortByColumn` on columns had to be hand-written in
   TMDL (works, Desktop-proven), and visual-level sort order is not
   authorable. Both are natural `model`/`report` commands.
7. **Measure authoring ergonomics.** Long multi-line measures were edited
   directly in TMDL; `model measures add --expression` is awkward for
   500-character DAX with quoting. Consider `--expression-file`.
8. **Desktop preflight control.** `desktop open` runs the strict lint with
   no bypass; when the lint itself has a defect (see fixes above) the whole
   proof loop is blocked. Add `--preflight <strict|normal|skip>` with strict
   as default.

## Round-3 gaps (first real-data feedback loop, 2026-08-17)

A production-data session fed photographed evidence back into the authoring
loop and forced a rework round (exposure floors, refresh-time group ranks,
visual polish, one cloned-and-rewired page). New gaps, prioritized:

1. **M type-safety lint (highest value).** Columns produced by
   `Table.ExpandTableColumn` are untyped and can be *loaded as text* even
   when the TMDL column says `dataType: double`; a downstream DAX comparison
   then errors and the visual renders blank. Cost a full Desktop
   close→rebuild→reopen→refresh round-trip to diagnose. Lint rule: numeric
   TMDL column whose partition's final step chain contains an expansion that
   feeds it without a later `Table.TransformColumnTypes` → warning.
2. **Visual formatting/objects commands.** Axis titles, category-axis
   visibility, card fonts, word wrap, and projection display names all had
   to be patched with hand-written JSON scripts across ~20 visuals. A
   `report visuals set-object`/`set-display-name` family would remove the
   riskiest hand-authoring left in the loop.
3. **Visual scaffolding for small parts.** Cards, slicers, and textboxes
   (reading guides) were created by copying JSON idioms from sibling
   visuals. `report visuals add-card --measure`, `add-slicer --field`,
   `add-textbox --paragraphs-file` would cover it.
4. **Top-N guard filter management.** The guard subquery (order-by measure,
   Top value) was mutated by hand, including rewiring guards on a cloned
   page. `report visuals set-topn-guard --order-by <measure> --top <n>`.
5. **Grouped-rank M snippet.** "Rank rows inside their group at refresh
   time" (sort + index per group, zero for ineligible rows, final explicit
   retype) was hand-written twice; it is the standard escape from
   query-time RANKX at scale and deserves a generator or documented recipe.
6. **Desktop refresh automation.** `desktop open/screenshot/dax execute`
   are solid; the refresh click remains manual (this round: driven via OS
   automation). The planned `desktop refresh-check` is the missing link for
   a fully unattended canvas-refresh proof.

Positive field results worth recording: `report pages clone` carried a real
page-derivation job (identity regeneration, filter renames, zero warnings)
and `model dax execute` against the live Desktop session turned a
blank-visual mystery into a one-query diagnosis (the text-typed column).

## Rounds 4–6 field results (2026-08-17/18)

The six commands built in the round-3 campaign all saw real use within a
day: `add-textbox` authored a live reading-guide, `set-topn-guard` retuned
guards, `pages clone` derived three more pages, `add-static` built two
lookup tables, and the `m.untyped_expansion` lint caught a real latent bug
in the shipped report on its FIRST run (the size-filter column). New
Desktop-proven PBIR idioms that the CLI does not yet author and should
learn (candidate `set-object` catalog extensions or new commands):

1. `visual.query.sortDefinition` (`{sort:[{field,direction}],isDefaultSort}`)
   — default descending sorts on tables/matrices; hand-patched on 8 visuals.
2. `objects.dataPoint` with `selector.data[].scopeId.Comparison` on a series
   column value — pinning legend colors per category (quadrant semantics);
   Power BI otherwise deals colors by legend order.
3. `objects.categoryAxis/valueAxis` `start`/`end` fixed ranges plus
   `objects.plotArea.image` with a `ResourcePackageItem` URL, and the
   report.json `resourcePackages` item `type: "Image"` registration — the
   full offline background-map recipe (axis bounds must equal the image's
   drawn bounds for alignment by construction).
4. `objects.bubbles.bubbleSize` (`-50L`-style percent literal) on scatter.
5. M anti-lesson for a future lint: duplicate let-step names across edit
   rounds surface in Desktop as a spurious-looking "cyclic reference"
   refresh error — a `m.duplicate_step_name` ERROR rule would catch what
   took a full Desktop round-trip to diagnose.

## Patterns to promote into documentation

- **Guard-filter pattern** for large-cardinality visuals: a cheap ranking
  measure (plain column MAX, or a count through a deliberately inactive
  relationship activated via USERELATIONSHIP) drives a visual Top-N filter,
  with sentinel ranking values keeping structural rows (totals, remainders)
  alive. Heavy display measures then evaluate over a bounded axis only.
- **Disconnected analytic tables** computed at refresh time in M (signals,
  backtest metrics, diagnostics) keep interaction latency flat and run
  identically against synthetic and live sources; they carry their own
  dimension columns so they cannot disturb the star schema.
- **Aggregate-first rate measures** (`SUM(numerator)/SUM(denominator)`),
  never averages of ratios.
- **Aggregate-only diagnostics export**: a long-format table (category,
  metric, dimension, value, note) with small-cell suppression built in M, so
  a locked-down environment can share model-health evidence without sharing
  data.
