# Handoff

## Required final checks

```powershell
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets
git diff --check
```

Then build the local binary and run `validate --strict` against both report
variants. The PostgreSQL/work variant and synthetic/home variant must both pass
with six pages, 45 bound visuals, 17 tables, 62 measures, and two relationships.

## Deliberate limitations

- The DAX lint rule is a targeted static semantic guard, not a grammar. Use the
  separate bounded `model dax execute` Desktop bridge for targeted live-engine
  proof.
- Raw role validation applies only to visual types in the generated catalog;
  unknown Desktop-authored/custom visuals are not guessed.
- Canvas/refresh validation remains a manual Desktop step.
- The Desktop IPC Bridge preview currently lacks a DAX-query method. Retest its
  manifest after Desktop updates and prefer the official transport if that
  capability appears.
- This batch may be committed and pushed because the user explicitly approved
  both repositories.
