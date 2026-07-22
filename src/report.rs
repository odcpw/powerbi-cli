use crate::contract::suggested_command_path;
use crate::feature_catalog::unsupported_feature_error;
use crate::inspect::deep_inspect;
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
use crate::{
    CliError, CliResult, canonical_display, command_arg, resolve_project, validate_project,
};
use serde_json::Value;
use serde_json::json;
use std::path::PathBuf;

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
            "report requires a subcommand: build, spec fields, spec validate, design-plan, wireframe export, pages, bookmarks, filters, slicers, interactions, themes, visuals",
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

fn wireframe_export(args: &[String]) -> CliResult<Value> {
    let path = parse_wireframe_args(args)?;
    let resolved = resolve_project(&path)?;
    let validation = validate_project(&resolved)?;
    let deep = deep_inspect(&resolved, &validation)?;
    let report = deep["report"].clone();
    let handles = deep["handles"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| matches!(item["kind"].as_str(), Some("project" | "page" | "visual")))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(json!({
        "schema": "powerbi-cli.report.wireframe.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "valid": validation.errors.is_empty(),
        "counts": {
            "pages": validation.pages,
            "visuals": validation.visuals,
            "boundVisuals": validation.bound_visuals
        },
        "handles": handles,
        "pages": report["pages"].clone(),
        "next": [
            format!("powerbi-cli inspect --deep {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": validation.warnings,
        "errors": validation.errors
    }))
}

fn parse_wireframe_args(args: &[String]) -> CliResult<PathBuf> {
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown report wireframe export flag: {other}"
                ))
                .with_hint(
                    "Only JSON wireframe export is supported now. Use global `--json` or `--format json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report wireframe export <project-dir-or.pbip> --json",
                ));
            }
            other => {
                if path.is_some() {
                    return Err(CliError::invalid_args(
                        "report wireframe export accepts exactly one path",
                    )
                    .with_hint(
                        "Run `powerbi-cli report wireframe export <project-dir-or.pbip> --json`.",
                    )
                    .with_suggested_command(
                        "powerbi-cli report wireframe export <project-dir-or.pbip> --json",
                    ));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    path.ok_or_else(|| {
        CliError::invalid_args("report wireframe export requires a path")
            .with_hint("Run `powerbi-cli report wireframe export <project-dir-or.pbip> --json`.")
            .with_suggested_command(
                "powerbi-cli report wireframe export <project-dir-or.pbip> --json",
            )
    })
}
