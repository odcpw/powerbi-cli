# Pass 2 Regression Alerts

No product regressions remain. One pre-existing MCP descendant-reaping test failed once during a full parallel run, then passed three consecutive focused reruns and the complete rerun; no MCP code changed in this pass.

Watch these boundaries in future passes:

- A managed receipt must never own a pre-launch or PID-reused process.
- Any launched one-shot session with `cleanup.closed != true` must remain `oracle_failed`.
- Concurrent managed opens/closes must remain serialized by one state lock.
- Between slicers must reject text/unknown fields during creation, spec compilation, and rebinding.
- Linux and macOS must continue to compile the core CLI while refusing Windows-only Desktop commands with `unsupported_feature`.
