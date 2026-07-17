# Microsoft conformance provenance

- Integration lock: `integrations/microsoft/integration-lock.json`
- Complete npm graph: `integrations/microsoft/package-lock.json`
- Report validator: `@microsoft/powerbi-report-authoring-cli@0.1.4`
- Report package integrity: `sha512-SibT9RCS7dQdqYGhU0/r1yixYZgxRsjlpKSF+a/wGczN0YRG6M9nRoFBe2d4hptwwUeqvnkjNxpo7NTUxXNIDQ==`
- Node floor: 20; CI runtime: Node 22
- Fixture source: neutral generated `examples/sales.schema.json`
- Normalization schema: `powerbi-cli.validation.microsoft-report.v1` payload nested under `validators.microsoftReport`
- Generation date: 2026-07-17 UTC

CI records the exact OS, Node version, cache lock fingerprint, package version, child hashes, command outcome, and repository commit. Volatile absolute cache and project paths are not conformance identities.
