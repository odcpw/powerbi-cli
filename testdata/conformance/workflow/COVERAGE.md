# Source-workflow conformance coverage

| Contract requirement | Test | Status |
|---|---|---|
| Deterministic plan fingerprint | `source_profile_plan_is_deterministic_and_selects_only_the_pbip_closure` | Pass |
| Selected PBIP closure excludes sibling projects, `.git`, and unregistered data | same | Pass |
| Drift, overwrite, path escape, and credential rejection | `plan_rejects_drift_overwrite_path_escape_and_credentials` | Pass |
| Credential-like resource override paths are rejected before plan/output persistence | `credential_like_override_path_is_rejected_before_plan_or_output_persistence` | Pass |
| Unsafe cache/data/credential files and non-UTF-8 or credential-bearing SVG inside selected artifacts | `plan_rejects_unsafe_files_inside_selected_artifacts` | Pass |
| Plan and output containment survives a recomputed fingerprint | `plan_and_recomputed_fingerprint_cannot_write_inside_source_project` | Pass |
| Resealed plans cannot widen profile-derived template, connector, or resource semantics | `resealed_plan_cannot_widen_profile_derived_semantics` | Pass |
| Root connector identity cannot be spoofed; hard-coded paths and unknown/dynamic/computed-postfix calls are refused | `connector_identity_ignores_comments_and_strings_and_rejects_other_connectors` | Pass |
| Complete transformed M and payload-free plan | `complete_transformed_m_is_materialized_without_template_payload_in_plan` | Pass |
| Recomputed receipt checksum cannot bypass semantics; failure preserves source identity | `recomputed_receipt_checksum_cannot_bypass_semantics_and_copy_failure_preserves_source` | Pass |
| Derived staged TMDL, copied report/resources, complete canonical MCP evidence, and credential-scanned evidence reject self-resealed artifact swaps | `reconstructed_stage_and_copy_evidence_reject_resealed_artifact_swaps` | Pass |
| Bounded hashing rejects oversized metadata before open, enforces the cap while streaming, and rejects links/reparse points | `tree_hash_is_bounded_and_rejects_links` | Pass |
| Capability-relative output writes remain bound to the opened root across pathname replacement and outside alias swaps; publication identity then fails | `workflow_output_identity_swap_keeps_copy_bound_to_opened_root`, `workflow_output_capability_cannot_be_redirected_by_root_alias_swap` | Pass |
| Exact pinned MCP, official validator, and verify | `workflow_plan_run_verify_with_exact_installed_sidecars` | Pass in the exact integration CI lane after installation |

All testable mandatory clauses in `powerbi-cli.source-profile.v1`,
`powerbi-cli.workflow-plan.v1`, and `powerbi-cli.workflow-receipt.v1` are covered.
There are no accepted divergences.
