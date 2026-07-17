# Scorecard

Scores use a 1–10 scale and cover the observed workflow, not every planned CLI
feature.

| Dimension | Before | After | Evidence |
|---|---:|---:|---|
| Visual-role correctness | 3 | 9 | Scatter catalog/build/mutation all emit Desktop's `Series` role |
| Input ergonomics | 7 | 9 | UI terms `legend`/`color` remain accepted aliases |
| Failure actionability | 4 | 9 | Stale role and empty visual folder include exact repairs |
| DAX preflight value | 5 | 9 | Proven scalar-IF/table misuse is caught statically; targeted EVALUATE queries can run against the exact open Desktop model |
| Contract consistency | 6 | 9 | Catalog, capabilities, archetype, golden, tests, README, oracle notes, and skill agree |
| Runtime honesty | 8 | 9 | Live query proof is separated from canvas/refresh proof and result data is explicitly classified as sensitive |
| Regression confidence | 6 | 9 | Positive/negative CLI tests plus full-suite and report revalidation gates |

Remaining gap: the CLI can execute bounded DAX queries but does not
automatically verify that every Desktop canvas rendered after refresh. Query
success and canvas/refresh proof remain explicitly separate.
