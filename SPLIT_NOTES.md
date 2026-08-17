# `tests/report.rs` isomorphic split notes

## Baseline

- Source binary: `tests/report.rs` (12,149 lines; B9 test monolith).
- Cold release run: `cargo test --release --test report` passed 86, failed 0; test execution 8.69s; total cold wall time 109.689s.
- Warm release run: `cargo test --release --test report` passed 86, failed 0; test execution 6.93s; total warm wall time 7.146s.
- Name inventory: `cargo test --release --test report -- --list` reported 86 tests and 0 benchmarks.
- Combined-family wall-time ceiling: 17.146s (`7.146s + max(10%, 10s)`).

### Exact baseline test names

1. `capabilities_advertise_report_layout_commands`
2. `known_unimplemented_report_features_return_structured_refusals`
3. `report_audit_and_sanitize_clear_filter_and_slicer_state_through_out_dir`
4. `report_bookmarks_list_and_show_raw_bookmarks_by_handle`
5. `report_bookmarks_list_empty_scaffold_returns_zero_bookmarks`
6. `report_bookmarks_list_reports_metadata_and_file_diagnostics`
7. `report_bookmarks_show_rejects_missing_or_unknown_handle_with_suggested_list_command`
8. `report_drillthrough_rejects_unproven_variants`
9. `report_drillthrough_set_show_clear_round_trips_through_out_dirs`
10. `report_filter_duplicate_identities_are_unique_but_mutation_ambiguous`
11. `report_filter_name_handles_survive_earlier_deletion_without_retargeting`
12. `report_filter_nameless_entries_use_fingerprint_handles`
13. `report_filters_add_rejects_unsafe_or_invalid_requests`
14. `report_filters_add_report_round_trips_through_out_dir`
15. `report_filters_add_supports_page_and_visual_selectors`
16. `report_filters_clear_groups_filter_config_and_legacy_arrays`
17. `report_filters_clear_page_round_trips_through_out_dir`
18. `report_filters_clear_rejects_unsafe_requests`
19. `report_filters_clear_visual_supports_full_handle_and_page_visual_selector`
20. `report_filters_delete_rejects_unsafe_requests`
21. `report_filters_delete_round_trips_through_out_dir`
22. `report_filters_list_and_show_report_page_visual_filters_by_handle`
23. `report_filters_list_empty_scaffold_returns_zero_filters`
24. `report_filters_numeric_range_full_lifecycle_all_scopes`
25. `report_filters_numeric_range_rejects_unsafe_requests`
26. `report_filters_relative_date_full_lifecycle_all_scopes`
27. `report_filters_relative_date_rejects_unsafe_requests`
28. `report_filters_show_rejects_missing_or_unknown_handle_with_suggested_list_command`
29. `report_filters_topn_full_lifecycle_visual_scope`
30. `report_filters_topn_rejects_unsafe_requests`
31. `report_filters_update_categorical_values_full_lifecycle`
32. `report_filters_update_rejects_unsafe_requests`
33. `report_interactions_disable_dry_run_and_out_dir_upsert_no_filter`
34. `report_interactions_list_and_show_page_visual_interactions_by_handle`
35. `report_interactions_list_empty_scaffold_returns_zero_interactions`
36. `report_interactions_mutations_reject_unsafe_or_unproven_requests`
37. `report_interactions_set_updates_existing_row_without_duplicates_and_supports_in_place`
38. `report_interactions_show_accepts_endpoint_selector_and_rejects_bad_selectors`
39. `report_object_tree_find_cat_and_query_expose_stable_handles`
40. `report_pages_and_visuals_are_readable_by_handle`
41. `report_pages_mutations_reject_unsafe_requests`
42. `report_pages_mutations_round_trip_through_out_dirs`
43. `report_sanitize_in_place_requires_exact_confirm_token`
44. `report_slicers_clear_accepts_visual_selectors_and_rejects_non_slicer`
45. `report_slicers_clear_handles_legacy_array_and_preserves_unmatched_filters`
46. `report_slicers_clear_rejects_unsafe_requests`
47. `report_slicers_clear_round_trips_through_out_dir`
48. `report_slicers_list_and_show_raw_slicer_by_handle`
49. `report_slicers_list_empty_scaffold_returns_zero_slicers`
50. `report_slicers_show_accepts_visual_handle_and_rejects_missing_or_unknown_handle`
51. `report_theme_preset_uses_schema_three_version_object`
52. `report_themes_apply_rejects_unsafe_or_wrong_bundle`
53. `report_themes_extract_and_apply_raw_bundle`
54. `report_visual_add_defaults_require_a_binding_and_create_alias_is_readable`
55. `report_visual_add_rejects_unsafe_requests`
56. `report_visual_add_round_trips_through_out_dir`
57. `report_visual_add_supports_catalog_chart_aliases`
58. `report_visual_add_supports_series_and_scatter_bubble_roles`
59. `report_visual_clone_preserves_desktop_authored_slicer_template_state`
60. `report_visual_clone_rejects_unsafe_requests`
61. `report_visual_clone_round_trips_through_out_dir`
62. `report_visual_delete_handles_read_only_visual_directories_on_windows`
63. `report_visual_delete_rejects_unsafe_requests`
64. `report_visual_delete_round_trips_through_out_dir`
65. `report_visual_explicit_sort_refuses_unproven_shapes`
66. `report_visual_new_families_reject_invalid_bindings_and_slicer_modes`
67. `report_visual_new_families_round_trip_add_format_bind_clone_and_delete`
68. `report_visual_set_bindings_preserves_between_slicer_type_safety`
69. `report_visual_set_bindings_rejects_bad_specs`
70. `report_visual_set_bindings_round_trips_through_out_dir`
71. `report_visual_set_position_rejects_unsafe_geometry`
72. `report_visual_set_position_round_trips_through_out_dir`
73. `report_visuals_catalog_advertises_generated_types_roles_and_limits`
74. `report_visuals_formatting_extract_and_apply_round_trip_through_out_dir`
75. `report_visuals_formatting_list_and_show_summarize_objects_without_raw`
76. `report_visuals_formatting_set_color_creates_missing_title_card_with_page_visual_selector`
77. `report_visuals_formatting_set_color_creates_numeric_data_view_wildcard`
78. `report_visuals_formatting_set_color_rejects_unsafe_requests`
79. `report_visuals_formatting_set_color_round_trips_through_out_dir`
80. `report_visuals_formatting_set_text_creates_missing_cards_with_page_visual_selector`
81. `report_visuals_formatting_set_text_rejects_unsafe_requests`
82. `report_visuals_formatting_set_text_round_trips_through_out_dir`
83. `report_visuals_reject_unproven_value_columns_and_duplicate_fields`
84. `validate_accepts_desktop_field_well_filter_placeholders`
85. `validate_rejects_stale_scatter_legend_role_with_series_repair`
86. `validate_reports_empty_visual_directory_with_repair_hint`

## Family map

The map is updated as each family is extracted. Test names remain byte-for-byte unchanged.

### `tests/report_filters.rs`

- `report_filters_list_empty_scaffold_returns_zero_filters`
- `report_filters_list_and_show_report_page_visual_filters_by_handle`
- `report_filters_show_rejects_missing_or_unknown_handle_with_suggested_list_command`
- `report_filters_add_report_round_trips_through_out_dir`
- `report_filters_add_supports_page_and_visual_selectors`
- `report_filters_add_rejects_unsafe_or_invalid_requests`
- `report_filters_numeric_range_full_lifecycle_all_scopes`
- `report_filters_topn_full_lifecycle_visual_scope`
- `report_filters_relative_date_full_lifecycle_all_scopes`
- `report_filters_update_categorical_values_full_lifecycle`
- `report_filters_numeric_range_rejects_unsafe_requests`
- `report_filters_topn_rejects_unsafe_requests`
- `report_filters_relative_date_rejects_unsafe_requests`
- `report_filters_update_rejects_unsafe_requests`
- `report_filters_delete_round_trips_through_out_dir`
- `report_filters_delete_rejects_unsafe_requests`
- `report_filter_name_handles_survive_earlier_deletion_without_retargeting`
- `report_filter_duplicate_identities_are_unique_but_mutation_ambiguous`
- `report_filter_nameless_entries_use_fingerprint_handles`
- `report_filters_clear_page_round_trips_through_out_dir`
- `report_filters_clear_visual_supports_full_handle_and_page_visual_selector`
- `report_filters_clear_rejects_unsafe_requests`
- `report_filters_clear_groups_filter_config_and_legacy_arrays`

### `tests/report_formatting.rs`

- `report_visuals_formatting_list_and_show_summarize_objects_without_raw`
- `report_visuals_formatting_extract_and_apply_round_trip_through_out_dir`
- `report_visuals_formatting_set_text_round_trips_through_out_dir`
- `report_visuals_formatting_set_text_creates_missing_cards_with_page_visual_selector`
- `report_visuals_formatting_set_text_rejects_unsafe_requests`
- `report_visuals_formatting_set_color_round_trips_through_out_dir`
- `report_visuals_formatting_set_color_creates_missing_title_card_with_page_visual_selector`
- `report_visuals_formatting_set_color_creates_numeric_data_view_wildcard`
- `report_visuals_formatting_set_color_rejects_unsafe_requests`

### `tests/report_slicers.rs`

- `report_slicers_list_empty_scaffold_returns_zero_slicers`
- `report_slicers_list_and_show_raw_slicer_by_handle`
- `report_slicers_show_accepts_visual_handle_and_rejects_missing_or_unknown_handle`
- `report_slicers_clear_round_trips_through_out_dir`
- `report_slicers_clear_accepts_visual_selectors_and_rejects_non_slicer`
- `report_slicers_clear_rejects_unsafe_requests`
- `report_slicers_clear_handles_legacy_array_and_preserves_unmatched_filters`

## Shared-helper decisions

- The local `RunOutput`, `run_powerbi`, `stdout_json`, and `stderr_json` variants were byte-identical in behavior to the existing exports in `tests/common/mod.rs`; the split binaries use the common versions and no renamed shadow variants were introduced.
- Helpers used by two or more report-family binaries move to `tests/common/mod.rs` with only the `pub` visibility adjustment required by separate integration crates.
- Family-only helpers remain private in their owning `report_*` file.
- No helper required a distinct-name move into common.

## Gate evidence per commit

Each entry records the post-commit gate: exact-name inventory, combined-family timing, full release suite, clippy with warnings denied, and formatting check. Known MCP timing flakes are rerun in isolation if observed.

### `900c487` — filters

- Exact-name inventory: 86 expected, 86 actual, 86 unique; 0 missing, added, or duplicated.
- Combined report-family run: 86 passed, 0 failed in 7.853s (ceiling 17.146s).
- `cargo test --release --no-fail-fast`: passed; no MCP timing flake observed.
- `cargo clippy --release --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.

### `a345d41` — formatting

- Exact-name inventory: 86 expected, 86 actual, 86 unique; 0 missing, added, or duplicated.
- Combined report-family run: 86 passed, 0 failed in 11.544s (ceiling 17.146s).
- `cargo test --release --no-fail-fast`: all integration targets passed; the unit target encountered host-timing failures in `mcp::tests::child_guard_drop_terminates_the_owned_process_tree`, `mcp::tests::fake_server_timeout_cancels_and_reaps_without_deadlock`, and `mcp::tests::graceful_root_exit_also_reaps_captured_descendants`. Each passed when rerun alone with `--exact`, so they were classified as the documented MCP child-process host flakes.
- `cargo clippy --release --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
