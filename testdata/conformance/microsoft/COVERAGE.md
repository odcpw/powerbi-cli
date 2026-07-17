# Microsoft integration conformance coverage

The normal CI matrix installs the committed exact Microsoft npm graph, verifies deep cache readiness, and runs `tests/microsoft_report_exact.rs` against the neutral generated cases in `official-report-cases.json`.

Covered for the official report backend:

- exact package resolution and version provenance;
- valid consumed-surface validation;
- invalid diagnostic code, severity, and relative file preservation;
- explicit `native`, `microsoft-report`, and `all` provenance;
- explicit missing-dependency failure;
- ambiguous report refusal before child launch;
- malformed, oversized, timed-out, and explicit `{ "error": { ... } }` vendor responses;
- bounded hashes, field-level redaction, and `--no-schema` execution that disables schema downloads.

Desktop Bridge evidence and Modeling MCP protocol coverage are tracked by their dedicated conformance tests.
