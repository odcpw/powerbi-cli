# Pass 2 Uplift Diff

- Interactive testing now has a first-class `desktop open` / idempotent `desktop close` lifecycle instead of unbounded `--leave-open` or raw launches.
- Cleanup is serialized and requires exact PID, creation time, post-baseline provenance, and `PBIDesktop*` identity; unresolved ownership is an error, never a success.
- Numeric/date Between slicers are first-class in direct visual creation and dashboard specs, and text rebinding cannot bypass the type contract.
- The skill now directs agents to one canonical project, one reusable QA output, and a `finally`-style Desktop close step instead of proliferating same-title versions.
- Windows tests, Linux `-D warnings` compilation, skill validation, a live managed open/close smoke, and independent fresh-eyes review all pass.
