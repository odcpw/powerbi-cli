# Workflow fixture provenance

The neutral workflow fixtures are generated inside `src/workflow.rs` tests.
The semantic-model-only tests copy the committed exact-MCP fixture at
`testdata/conformance/microsoft/modeling-mcp/Synthetic.SemanticModel`. The exact
end-to-end test scaffolds `examples/sales.schema.json` with the current
`powerbi-cli` binary, creates a credential-free synthetic resource and complete
M template, then exercises the installed versions pinned by
`integrations/microsoft/integration-lock.json`.

Run the default contract suite with:

```text
cargo test --locked workflow::tests
```

Run the exact-package proof after `integrations install` with:

```text
cargo test --locked workflow::tests::workflow_plan_run_verify_with_exact_installed_sidecars -- --ignored --nocapture
```
