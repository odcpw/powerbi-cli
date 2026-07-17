# Native and Microsoft validator discrepancies

## Legacy visual alt-text placement

The neutral invalid case adds `altText` under `visual.objects.general`. The official validator reports `PBIR_FORMATTING_PROP_UNKNOWN`; native non-strict validation can still accept the project because native validation focuses on deterministic PBIP structure and offline safety. Native strict lint separately identifies the legacy location.

This difference is intentional and visible. The `all` backend retains independent `validators.native` and `validators.microsoftReport` outcomes and never merges, suppresses, or downgrades the official diagnostic.

No unresolved discrepancy can be reported as official validity.
