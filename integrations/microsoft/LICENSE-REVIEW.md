# Integration license decision

| Dependency | Pin | Declared license | Use |
|---|---:|---|---|
| `@microsoft/powerbi-modeling-mcp` and platform artifacts | `0.5.0-beta.11` | `Microsoft` | Optional exact-installed Modeling MCP backend |
| `@microsoft/powerbi-report-authoring-cli` | `0.1.4` | `MIT` | Optional exact-installed report validator |
| `@microsoft/powerbi-desktop-bridge-cli` | `0.1.2` | `MIT` | Optional exact-installed Desktop bridge |

Release decision (2026-07-17): accepted for this integration boundary.

- `powerbi-cli` does not copy, vendor, republish, or embed any Microsoft npm package. Users explicitly download the exact graph from Microsoft's npm distribution with `integrations install --allow-network` into their private user cache.
- The Modeling MCP license permits installation and internal development/testing use, prohibits redistribution and live operating-environment use, and describes the preview, telemetry, confidentiality, termination, and time-sensitive conditions. The CLI documents the package as a pinned preview black box and does not claim broader rights.
- The Modeling MCP and installed Windows platform-artifact license files were byte-identical at review time (SHA-256 `fb999bab4d2c2e91ff34140244b121a8849573812e18586003a16c6747212865`). Other platform artifacts remain upstream packages and are never distributed by this repository.
- Report Authoring CLI and Desktop Bridge declare MIT. Their upstream `LICENSE` and `NOTICE` files remain in the user-installed cache.
- The repository's own source is licensed separately under the root MIT `LICENSE`.

This records the project's release boundary, not legal advice or a relicensing of Microsoft's software. A pin or upstream-license change requires a new review and lock ID.
