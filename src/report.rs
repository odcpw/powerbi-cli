use crate::contract::suggested_command_path;
use crate::feature_catalog::unsupported_feature_error;
use crate::report_bookmarks::bookmarks_command;
use crate::report_build::{build_command, spec_command};
use crate::report_design::design_plan_command;
use crate::report_drilldown::drilldown_command;
use crate::report_drillthrough::drillthrough_command;
use crate::report_filters::filters_command;
use crate::report_hygiene::hygiene_command;
use crate::report_interactions::interactions_command;
use crate::report_layout::layout_command;
use crate::report_objects::objects_command;
use crate::report_pages::pages_command;
use crate::report_plan::plan_command;
use crate::report_slicers::slicers_command;
use crate::report_style::style_command;
use crate::report_themes::themes_command;
use crate::report_visuals::visuals_command;
use crate::report_wireframe::wireframe_export;
use crate::{CliError, CliResult};
use serde_json::Value;

pub(crate) fn report_command(args: &[String]) -> CliResult<Value> {
    match args {
        [family, rest @ ..] if family == "build" => build_command(rest),
        [family, rest @ ..] if family == "spec" => spec_command(rest),
        [family, rest @ ..] if family == "plan" => plan_command(rest),
        [family, action, rest @ ..] if family == "design" && action == "plan" => {
            design_plan_command(rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "design-plan" | "designplan") => {
            design_plan_command(rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "layout" | "layouts") => {
            layout_command(rest)
        }
        [family, action, rest @ ..] if family == "wireframe" && action == "export" => {
            wireframe_export(rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "tree" | "find" | "cat" | "query") => {
            objects_command(family, rest)
        }
        [family, action, rest @ ..]
            if family == "objects" && matches!(action.as_str(), "tree" | "find" | "cat" | "query") =>
        {
            objects_command(action, rest)
        }
        [family, action, rest @ ..] if family == "object" && action == "show" => {
            objects_command("cat", rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "audit" | "sanitize") => {
            hygiene_command(family, rest)
        }
        [family, rest @ ..] if family == "pages" => pages_command(rest),
        [family, rest @ ..] if matches!(family.as_str(), "bookmarks" | "bookmark") => {
            bookmarks_command(rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "filters" | "filter") => {
            filters_command(rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "slicers" | "slicer") => {
            slicers_command(rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "interactions" | "interaction") => {
            interactions_command(rest)
        }
        [family, rest @ ..]
            if matches!(family.as_str(), "themes" | "theme") =>
        {
            themes_command(rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "styles" | "style") => {
            style_command(rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "visuals" | "visual") => {
            visuals_command(rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "drillthrough" | "drill-through") => {
            drillthrough_command(rest)
        }
        [family, rest @ ..] if matches!(family.as_str(), "drilldown" | "drill-down") => {
            drilldown_command(rest)
        }
        [family, ..] if matches!(family.as_str(), "tooltip" | "tooltips") => {
            Err(unsupported_feature_error("report.tooltip-pages"))
        }
        [] => Err(CliError::invalid_args(
            "report requires a subcommand: build, spec fields, spec schema, spec explain, spec validate, spec normalize, spec upgrade, design-plan, wireframe export, pages, bookmarks, filters, slicers, interactions, themes, visuals",
        )
        .with_hint("Run `powerbi-cli report spec fields --schema <schema.json> --json`, `powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir> --json`, or inspect supported report primitives.")
        .with_suggested_command(
            "powerbi-cli report spec fields --schema <schema.json> --json",
        )
        .with_suggested_command(
            "powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir> --json",
        )),
        _ => Err(unknown_report_command(args)),
    }
}

fn unknown_report_command(args: &[String]) -> CliError {
    let mut attempted = vec!["report".to_string()];
    attempted.extend_from_slice(args);
    if let Some(candidate) = suggested_command_path(&attempted) {
        return CliError::invalid_args(format!("unknown report command: {}", args.join(" ")))
            .with_hint(format!(
                "Did you mean `powerbi-cli {candidate}`? Inspect that exact command contract before running it."
            ))
            .with_suggested_command(format!(
                "powerbi-cli --json capabilities --for \"{candidate}\""
            ));
    }
    CliError::invalid_args("unknown report command")
        .with_hint(
            "Run `powerbi-cli --json capabilities --for report` for supported report commands.",
        )
        .with_suggested_command("powerbi-cli --json capabilities --for report")
}
