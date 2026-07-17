# Improvements Implemented

## Contract and generation

- Scatter's optional color-grouping role is now canonically `Series`.
- Catalog examples include a Series binding.
- Dashboard specs, report mutations, and report builds share that contract.
- Friendly aliases normalize to the canonical role before PBIR generation.

## Validation and diagnostics

- Known generated visual types have raw `queryState` role keys checked against
  the catalog. An alias in stored PBIR is rejected with its canonical spelling;
  an unknown role lists supported roles.
- Empty visual containers are rejected during page validation with a direct
  remove-or-restore instruction.
- DAX lint masks strings/comments, identifies variables assigned directly from
  `IF()`, and reports when they are passed as the first argument to known table
  consumers. The rule is narrow enough to leave scalar IF usage untouched.
- `model dax execute` accepts only `EVALUATE` or `DEFINE ... EVALUATE`, requires
  both Desktop-oracle and model-data opt-ins, matches the exact already-open
  PBIP, and refuses auto-launch or model writes. Query size, rows, text cells,
  and runtime are bounded; output returns only query length/fingerprint, and
  temporary query/assembly files are removed after execution.

## Learning assets

- Desktop oracle notes now record both newly proven failure modes.
- The powerbi-cli skill links a progressive-disclosure runtime regression
  reference covering source variants, composite business keys, exact TREATAS
  labels, log-scale honesty, and live page interaction checks.
- Regression tests freeze both accepted aliases and rejected serialized roles.
- The capability/feature catalogs and skill now direct agents to bounded live
  DAX execution before UI automation and explicitly keep query proof separate
  from canvas/refresh proof.
