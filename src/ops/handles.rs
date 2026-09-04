use crate::pbir_filters::{FilterArrayOrigin, FilterScope, named_filter_handle};
use crate::report_build::{page_name, visual_name};
use crate::report_filter_shapes::{FilterSpec, ResolvedFilterColumn, generated_filter_name};
use crate::tmdl::measure_handle as semantic_measure_handle;
use crate::{CliError, CliResult};

/// Derive a stable public handle using the same naming functions used by the
/// dashboard compiler and report inspection. `page` is required for visual
/// and filter handles and ignored for page handles.
pub(crate) fn derive_handle(kind: &str, page: Option<&str>, spec_id: &str) -> CliResult<String> {
    match kind {
        "page" => Ok(page_handle(page.unwrap_or(spec_id))),
        "measure" => {
            let table = page.ok_or_else(|| {
                CliError::invalid_args("measure handle derivation requires a table name")
            })?;
            Ok(measure_handle(table, spec_id))
        }
        "visual" | "card" | "slicer" | "textbox" => {
            let page = page.ok_or_else(|| {
                CliError::invalid_args("visual handle derivation requires a page name")
            })?;
            Ok(visual_handle(page, spec_id))
        }
        "filter" => {
            let owner = page.ok_or_else(|| {
                CliError::invalid_args("filter handle derivation requires an owner handle")
            })?;
            filter_handle(owner, spec_id)
        }
        other => Err(CliError::invalid_args(format!(
            "unsupported operation handle kind: {other}"
        ))),
    }
}

pub(crate) fn page_handle(name_or_id: &str) -> String {
    format!("page:{}", page_name(strip_page_prefix(name_or_id)))
}

/// Derive the semantic-model measure identity using TMDL's percent/colon
/// escaping rather than duplicating the component encoder here.
pub(crate) fn measure_handle(table: &str, name: &str) -> String {
    semantic_measure_handle(table, name)
}

pub(crate) fn visual_handle(page: &str, spec_id: &str) -> String {
    format!(
        "visual:{}:{}",
        page_name(strip_page_prefix(page)),
        visual_name(spec_id)
    )
}

fn strip_page_prefix(value: &str) -> &str {
    value.strip_prefix("page:").unwrap_or(value)
}

/// Derive an identity-based filter handle from an existing owner handle. The
/// name is passed through the kernel's `named_filter_handle` implementation so
/// percent/colon escaping cannot diverge from report filter readback.
pub(crate) fn filter_handle(owner: &str, name: &str) -> CliResult<String> {
    if owner == "report" || owner == "report:main" {
        return Ok(named_filter_handle(
            FilterScope::Report,
            None,
            None,
            name,
            FilterArrayOrigin::FilterConfig,
        ));
    }
    if let Some(page) = owner.strip_prefix("page:") {
        return Ok(named_filter_handle(
            FilterScope::Page,
            Some(page),
            None,
            name,
            FilterArrayOrigin::FilterConfig,
        ));
    }
    if let Some(visual) = owner.strip_prefix("visual:") {
        let mut pieces = visual.splitn(2, ':');
        let page = pieces.next().filter(|value| !value.is_empty());
        let visual = pieces.next().filter(|value| !value.is_empty());
        if let (Some(page), Some(visual)) = (page, visual) {
            return Ok(named_filter_handle(
                FilterScope::Visual,
                Some(page),
                Some(visual),
                name,
                FilterArrayOrigin::FilterConfig,
            ));
        }
    }
    Err(CliError::invalid_args(format!(
        "filter owner must be report:main, page:<Name>, or visual:<Page>:<Container>: {owner}"
    )))
}

/// Derive the generated filter identity used by the filter authoring kernel.
/// Generated names intentionally include target/type/condition hashes; this
/// wrapper keeps operation declarations identical to the eventual PBIR name.
pub(crate) fn generated_filter_handle(
    scope: FilterScope,
    page_name: Option<&str>,
    visual_name: Option<&str>,
    column: &ResolvedFilterColumn,
    spec: &FilterSpec,
) -> String {
    let name = generated_filter_name(scope, column, spec);
    named_filter_handle(
        scope,
        page_name,
        visual_name,
        &name,
        FilterArrayOrigin::FilterConfig,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report_filter_shapes::FilterSpec;
    use serde_json::json;

    #[test]
    fn page_and_visual_handles_reuse_compiler_slug_rules() {
        assert_eq!(
            page_handle("overview page"),
            "page:ReportSectionOverviewPage"
        );
        assert_eq!(
            visual_handle("overview page", "revenue card"),
            "visual:ReportSectionOverviewPage:VisualContainerRevenueCard"
        );
        assert_eq!(
            visual_handle(
                "page:ReportSectionOverviewPage",
                "VisualContainerRevenueCard"
            ),
            "visual:ReportSectionOverviewPage:VisualContainerRevenueCard"
        );
        assert_eq!(
            page_handle("page:ReportSectionOverviewPage"),
            "page:ReportSectionOverviewPage"
        );
        assert_eq!(
            derive_handle("card", Some("overview page"), "revenue card").expect("handle"),
            "visual:ReportSectionOverviewPage:VisualContainerRevenueCard"
        );
        assert_eq!(
            derive_handle("page", Some("overview page"), "ignored").expect("page handle"),
            "page:ReportSectionOverviewPage"
        );
    }

    #[test]
    fn measure_handles_reuse_tmdl_component_encoding() {
        assert_eq!(
            measure_handle("Sales:%Facts", "Gross:%Margin"),
            "measure:Sales%3A%25Facts:Gross%3A%25Margin"
        );
        assert_eq!(
            derive_handle("measure", Some("Sales:%Facts"), "Gross:%Margin")
                .expect("measure handle"),
            "measure:Sales%3A%25Facts:Gross%3A%25Margin"
        );
    }

    #[test]
    fn filter_handles_use_the_existing_owner_identity_encoding() {
        assert_eq!(
            filter_handle("page:ReportSectionOverview", "Segment:Filter").expect("handle"),
            "filter:page:ReportSectionOverview:Segment%3AFilter"
        );
        assert_eq!(
            filter_handle(
                "visual:ReportSectionOverview:VisualContainerRevenue",
                "SegmentFilter"
            )
            .expect("handle"),
            "filter:visual:ReportSectionOverview:VisualContainerRevenue:SegmentFilter"
        );
    }

    #[test]
    fn generated_filter_handle_delegates_target_and_condition_hashes() {
        let column = ResolvedFilterColumn {
            table: "Customers".into(),
            column: "Segment".into(),
            data_type: Some("string".into()),
        };
        let spec = FilterSpec::Categorical {
            values: vec![json!("Enterprise")],
        };
        let handle = generated_filter_handle(
            FilterScope::Page,
            Some("ReportSectionOverview"),
            None,
            &column,
            &spec,
        );
        assert!(handle.starts_with("filter:page:ReportSectionOverview:PowerBICliPage"));
        assert!(handle.ends_with("Filter"));
    }
}
