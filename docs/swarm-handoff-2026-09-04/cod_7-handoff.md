# Codex 7 wind-down handoff

- Bead in progress: `pbi-t5-design-system-mlf.1`
- Branch: `ntm/powerbi-cli/cod_7`
- Last commit: `9886be7` (`wip: pbi-t5-design-system-mlf.1 design defaults implementation in progress`)

## Complete

- Added the embedded `src/design/defaults.json` design-defaults.v1 catalog and its strict loader/resolver.
- Validated active defaults against the formatting catalog and routed PBIR writes through the shared SetObject encoder/application path.
- Wired defaults into report build, scaffold, spec explain, and `report design defaults show` for spec/project inspection.
- Added schema, contract, feature-catalog, acceptance-harness, and generated command-documentation changes.
- Preserved omitted visual `format` fields in manifest copies so defaults-disabled output does not gain `null` fields.
- `cargo check --all-targets` passed before the final schema refusal adjustment.
- `cargo test --bin powerbi-cli design::defaults` passed (3 tests) before wind-down.

## Incomplete / untested

- Final targeted test matrix was not run after the last patch.
- `docs/roadmap.md` still needs its localized design-defaults documentation entry.
- `robot-docs render --json` and its check still need to be run after the final contract changes.
- Desktop acceptance, CLI contract, and design-defaults integration coverage still need final verification.
- The normal bead commit and final bead report were not produced; this is an intentional WIP handoff commit.

## Known failing tests

- The last targeted run (`cargo test --test report_spec --test report_spec_schema_explain --test report_build_response --test dashboard_build`) had one failure in `report_spec::every_uncompiled_v2_section_names_its_owning_bead`: the visual-format case returned `invalid_args` instead of the expected `unsupported_feature`. A follow-up patch restored format refusal when defaults are disabled and leaves that fix untested.

## Exact next step

Run the targeted report/spec/CLI/desktop acceptance tests, fix any failures, update `docs/roadmap.md`, regenerate and check the marker-delimited README/SKILL regions with `cargo run --quiet -- robot-docs render --json`, then commit the completed bead and write `cod_7-e.md`.
