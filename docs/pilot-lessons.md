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
