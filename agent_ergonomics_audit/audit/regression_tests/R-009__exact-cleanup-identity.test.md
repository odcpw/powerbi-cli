# R-009 Exact Cleanup Identity

- Managed ownership rejects pre-launch matching windows.
- Cleanup roots require exact recorded creation times and `PBIDesktop*` identity.
- Baseline and PID-reused processes are never killed.
- A launched one-shot command cannot succeed unless requested cleanup reports `closed=true`.
