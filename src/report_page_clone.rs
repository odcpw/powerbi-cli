use crate::cli_support::{
    MutationMode, mode_name, require_report_page_mode, set_report_page_mode, shell_arg,
    take_report_value as take_value, target_project,
};
use crate::pbir::{PageRecord, PageSelector, find_page, load_report_snapshot, page_detail};
use crate::pbir_filters::FilterScope;
use crate::project_io::{write_json_atomic, write_json_new_atomic};
use crate::report_filter_shapes::{regenerated_filter_name, validate_filter_name};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    read_json_value, resolve_project, validate_project,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const VISUAL_CONTAINER_PREFIX: &str = "VisualContainer";

#[derive(Debug, Default)]
struct CloneOptions {
    project: Option<PathBuf>,
    source: Option<String>,
    new_name: Option<String>,
    display_name: Option<String>,
    visual_prefix: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

#[derive(Debug)]
struct SourceVisual {
    name: String,
}

#[derive(Debug)]
enum PlannedFile {
    Json { relative: PathBuf, value: Value },
    Copy { source: PathBuf, relative: PathBuf },
}

#[derive(Debug)]
struct PageClonePlan {
    directories: Vec<PathBuf>,
    files: Vec<PlannedFile>,
    filter_renames: Vec<Value>,
    interaction_drops: Vec<Value>,
    changes: Vec<Value>,
    warnings: Vec<Value>,
}

pub(super) fn clone_page(args: &[String]) -> CliResult<Value> {
    let options = parse_clone_args(args)?;
    let source_project = options.project.clone().ok_or_else(|| {
        CliError::invalid_args("report pages clone requires --project <project-dir-or.pbip>")
            .with_hint("Pass the PBIP project directory or .pbip path explicitly.")
            .with_suggested_command(clone_dry_run_command())
    })?;
    let source_selector = options.source.as_deref().ok_or_else(|| {
        CliError::invalid_args("report pages clone requires --from <page-name-or-handle>")
            .with_hint("Use `report pages list` to get a stable page handle.")
            .with_suggested_command(clone_dry_run_command())
    })?;
    let new_page_name = options.new_name.as_deref().ok_or_else(|| {
        CliError::invalid_args("report pages clone requires --new-name <ReportSectionX>")
            .with_hint("Choose a new internal PBIR page name that does not already exist.")
            .with_suggested_command(clone_dry_run_command())
    })?;
    validate_page_name(new_page_name)?;
    if let Some(prefix) = options.visual_prefix.as_deref() {
        validate_visual_prefix(prefix)?;
    }
    if let Some(display_name) = options.display_name.as_deref() {
        validate_nonempty_text(display_name, "--display-name")?;
    }
    let mode = require_report_page_mode(options.mode, "report pages clone")?;

    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, clone_page)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    refuse_duplicate_page_name(new_page_name, &snapshot.pages)?;
    let source_page = find_page(
        &snapshot.pages,
        &page_selector(source_selector),
        "report pages clone",
    )?
    .clone();
    let display_name = options
        .display_name
        .clone()
        .unwrap_or_else(|| format!("{} (Kopie)", source_page.display_name));
    validate_nonempty_text(&display_name, "--display-name")?;

    let source_page_json = page_json_path(&source_page)?;
    let source_page_dir = source_page_json.parent().ok_or_else(|| {
        CliError::validation_failed(format!(
            "source page path has no parent: {}",
            source_page_json.display()
        ))
    })?;
    let pages_dir = target_resolved.report_dir.join("definition").join("pages");
    let target_page_dir = pages_dir.join(new_page_name);
    ensure_direct_child(&target_page_dir, &pages_dir, "target page")?;
    if target_page_dir.exists() {
        return Err(page_already_exists(new_page_name));
    }

    let source_visuals = source_visuals(source_page_dir)?;
    let common_source_prefix = common_source_visual_prefix(&source_visuals);
    let visual_renames = visual_rename_map(
        &source_visuals,
        new_page_name,
        options.visual_prefix.as_deref(),
        &common_source_prefix,
    )?;
    let clone_plan = build_page_clone_plan(
        source_page_dir,
        &target_page_dir,
        new_page_name,
        &display_name,
        &visual_renames,
    )?;

    let pages_json_path = pages_dir.join("pages.json");
    let mut pages_json = read_json_value(&pages_json_path)?;
    let before_order = page_order(&pages_json, &pages_json_path)?;
    let mut after_order = before_order.clone();
    after_order.push(new_page_name.to_string());
    set_page_order(&mut pages_json, &after_order, &pages_json_path)?;

    let target_page_json = target_page_dir.join("page.json");
    let target = target_page_summary(
        &source_page,
        new_page_name,
        &display_name,
        &target_page_json,
        &visual_renames,
        before_order.len(),
    );
    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        materialize_page_clone(&clone_plan, &target_page_dir)?;
        if let Err(error) = write_json_atomic(&pages_json_path, &pages_json) {
            let _ = fs::remove_dir_all(&target_page_dir);
            return Err(error);
        }
    }

    let validation = if dry_run {
        None
    } else {
        Some(validate_project(&target_resolved)?)
    };
    let validation_ok = validation
        .as_ref()
        .map(|report| report.errors.is_empty())
        .unwrap_or(true);
    let exit_code = if validation_ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let project_arg = command_arg(&target_resolved.project_dir);
    let target_handle = format!("page:{new_page_name}");
    let readback = format!(
        "powerbi-cli report pages show --project {project_arg} --handle {} --json",
        shell_arg(&target_handle)
    );
    let wireframe = format!("powerbi-cli report wireframe export {project_arg} --json");
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");

    let mut changes = vec![
        json!({
            "kind": "pbir.pages.pageOrder",
            "action": "append",
            "path": canonical_display(&pages_json_path),
            "before": before_order,
            "after": after_order
        }),
        json!({
            "kind": "pbir.page",
            "action": "clone",
            "path": canonical_display(&target_page_json),
            "before": Value::Null,
            "after": target
        }),
    ];
    changes.extend(visual_renames.iter().map(|(before, after)| {
        json!({
            "kind": "pbir.visual",
            "action": "clone-rename",
            "path": canonical_display(&target_page_dir.join("visuals").join(after).join("visual.json")),
            "before": { "name": before },
            "after": { "name": after }
        })
    }));
    changes.extend(clone_plan.changes.clone());

    Ok(json!({
        "schema": "powerbi-cli.report.pages.cloneMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "clone",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "source": page_detail(&source_page),
        "target": target,
        "clonePlan": {
            "strategy": "copy-page-subtree-and-rewrite-identities",
            "sourcePath": canonical_display(source_page_dir),
            "targetPath": canonical_display(&target_page_dir),
            "displayNameGenerated": options.display_name.is_none(),
            "visualNameStrategy": if options.visual_prefix.is_some() { "replace-common-prefix" } else { "append-deterministic-suffix" },
            "visualPrefix": options.visual_prefix,
            "commonSourceVisualPrefix": common_source_prefix,
            "visualRenames": visual_renames.iter().map(|(before, after)| json!({
                "before": before,
                "after": after
            })).collect::<Vec<_>>(),
            "filterRenames": clone_plan.filter_renames,
            "interactionDrops": clone_plan.interaction_drops
        },
        "counts": {
            "visualsCloned": visual_renames.len(),
            "filtersRenamed": clone_plan.filter_renames.len(),
            "interactionsDropped": clone_plan.interaction_drops.len(),
            "warnings": clone_plan.warnings.len()
        },
        "changes": changes,
        "warnings": clone_plan.warnings,
        "validation": validation.map(|report| json!({
            "ok": report.errors.is_empty(),
            "warnings": report.warnings,
            "errors": report.errors,
            "counts": {
                "tables": report.tables,
                "relationships": report.relationships,
                "measures": report.measures,
                "pages": report.pages,
                "visuals": report.visuals,
                "boundVisuals": report.bound_visuals
            }
        })),
        "readbackCommand": readback,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, wireframe, inspect, validate]
    }))
}

fn parse_clone_args(args: &[String]) -> CliResult<CloneOptions> {
    let mut options = CloneOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" | "-p" => {
                set_once(
                    &mut options.project,
                    PathBuf::from(take_value(args, &mut index, "--project")?),
                    "--project",
                )?;
            }
            "--from" => {
                set_once(
                    &mut options.source,
                    take_value(args, &mut index, "--from")?,
                    "--from",
                )?;
            }
            "--new-name" | "--newName" => {
                set_once(
                    &mut options.new_name,
                    take_value(args, &mut index, "--new-name")?,
                    "--new-name",
                )?;
            }
            "--display-name" | "--displayName" => {
                set_once(
                    &mut options.display_name,
                    take_value(args, &mut index, "--display-name")?,
                    "--display-name",
                )?;
            }
            "--visual-prefix" | "--visualPrefix" => {
                set_once(
                    &mut options.visual_prefix,
                    take_value(args, &mut index, "--visual-prefix")?,
                    "--visual-prefix",
                )?;
            }
            "--dry-run" => {
                set_report_page_mode(&mut options.mode, MutationMode::DryRun)?;
                index += 1;
            }
            "--in-place" => {
                set_report_page_mode(&mut options.mode, MutationMode::InPlace)?;
                index += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut index, "--out-dir")?);
                set_report_page_mode(&mut options.mode, MutationMode::OutDir)?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report pages clone flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"report pages clone\"` for exact flags.",
                )
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report pages clone\"",
                ));
            }
        }
    }
    Ok(options)
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> CliResult<()> {
    if slot.is_some() {
        return Err(CliError::invalid_args(format!(
            "{flag} may be specified only once"
        )));
    }
    *slot = Some(value);
    Ok(())
}

fn page_selector(value: &str) -> PageSelector {
    if value.starts_with("page:") {
        PageSelector {
            handle: Some(value.to_string()),
            name: None,
        }
    } else {
        PageSelector {
            handle: None,
            name: Some(value.to_string()),
        }
    }
}

fn validate_page_name(name: &str) -> CliResult<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains("..")
        || name
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch == ':' || ch.is_control())
    {
        return Err(CliError::invalid_args(format!("unsafe page name: {name}"))
            .with_hint("Use a simple internal page name without path separators.")
            .with_suggested_command(clone_dry_run_command()));
    }
    Ok(())
}

fn validate_visual_prefix(prefix: &str) -> CliResult<()> {
    if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "--visual-prefix must contain only ASCII letters and numbers",
    )
    .with_hint("Pass the stem that should follow VisualContainer, for example `--visual-prefix KPI`.")
    .with_suggested_command(clone_dry_run_command()))
}

fn validate_nonempty_text(value: &str, flag: &str) -> CliResult<()> {
    if !value.trim().is_empty() && !value.chars().any(char::is_control) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{flag} must be nonempty text"
    )))
}

fn refuse_duplicate_page_name(name: &str, pages: &[PageRecord]) -> CliResult<()> {
    if pages
        .iter()
        .any(|page| page.name.eq_ignore_ascii_case(name))
    {
        return Err(page_already_exists(name));
    }
    Ok(())
}

fn page_already_exists(name: &str) -> CliError {
    CliError::invalid_args(format!("page already exists: {name}"))
        .with_hint("Choose a unique --new-name; page cloning never overwrites an existing page.")
        .with_suggested_command(clone_dry_run_command())
}

fn page_json_path(page: &PageRecord) -> CliResult<&Path> {
    page.path.as_deref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "page has no page.json path in inspect output: {}",
            page.handle
        ))
    })
}

fn source_visuals(page_dir: &Path) -> CliResult<Vec<SourceVisual>> {
    let visuals_dir = page_dir.join("visuals");
    if !visuals_dir.exists() {
        return Ok(Vec::new());
    }
    if !visuals_dir.is_dir() {
        return Err(CliError::validation_failed(format!(
            "page visuals path is not a directory: {}",
            visuals_dir.display()
        )));
    }
    let mut visuals = Vec::new();
    for entry in fs::read_dir(&visuals_dir).map_err(|error| {
        CliError::unexpected(format!(
            "read visuals dir {}: {error}",
            visuals_dir.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            CliError::unexpected(format!(
                "read visual entry {}: {error}",
                visuals_dir.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            CliError::unexpected(format!(
                "read visual entry type {}: {error}",
                entry.path().display()
            ))
        })?;
        if file_type.is_symlink() {
            return Err(CliError::invalid_args(format!(
                "report pages clone refuses linked visual entries: {}",
                entry.path().display()
            )));
        }
        if !file_type.is_dir() {
            continue;
        }
        let folder_name = entry.file_name().into_string().map_err(|_| {
            CliError::validation_failed(format!(
                "visual directory name is not valid UTF-8: {}",
                entry.path().display()
            ))
        })?;
        let visual_json_path = entry.path().join("visual.json");
        if !visual_json_path.is_file() {
            return Err(CliError::validation_failed(format!(
                "visual directory is missing visual.json: {}",
                entry.path().display()
            )));
        }
        let visual_json = read_json_value(&visual_json_path)?;
        let json_name = visual_json["name"].as_str().ok_or_else(|| {
            CliError::validation_failed(format!(
                "{} is missing required visual name",
                visual_json_path.display()
            ))
        })?;
        if json_name != folder_name {
            return Err(CliError::validation_failed(format!(
                "{} name `{json_name}` does not match visual folder `{folder_name}`",
                visual_json_path.display()
            ))
            .with_hint("Repair the source visual identity before cloning the page."));
        }
        visuals.push(SourceVisual { name: folder_name });
    }
    visuals.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(visuals)
}

fn common_source_visual_prefix(visuals: &[SourceVisual]) -> String {
    if visuals.len() < 2
        || visuals
            .iter()
            .any(|visual| !visual.name.starts_with(VISUAL_CONTAINER_PREFIX))
    {
        return VISUAL_CONTAINER_PREFIX.to_string();
    }
    let token_sets = visuals
        .iter()
        .map(|visual| split_pascal_tokens(&visual.name[VISUAL_CONTAINER_PREFIX.len()..]))
        .collect::<Vec<_>>();
    let common_count = token_sets
        .first()
        .map(|first| {
            (0..first.len())
                .take_while(|index| {
                    token_sets
                        .iter()
                        .all(|tokens| tokens.get(*index) == first.get(*index))
                })
                .count()
        })
        .unwrap_or_default();
    let shared_stem = token_sets
        .first()
        .into_iter()
        .flat_map(|tokens| tokens.iter().take(common_count))
        .cloned()
        .collect::<String>();
    format!("{VISUAL_CONTAINER_PREFIX}{shared_stem}")
}

fn split_pascal_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    for character in value.chars() {
        if character.is_ascii_uppercase() && !token.is_empty() {
            tokens.push(std::mem::take(&mut token));
        }
        token.push(character);
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn visual_rename_map(
    visuals: &[SourceVisual],
    new_page_name: &str,
    requested_prefix: Option<&str>,
    common_source_prefix: &str,
) -> CliResult<BTreeMap<String, String>> {
    let mut renames = BTreeMap::new();
    let mut targets = BTreeSet::new();
    for visual in visuals {
        let target = if let Some(prefix) = requested_prefix {
            let suffix = visual
                .name
                .strip_prefix(common_source_prefix)
                .or_else(|| visual.name.strip_prefix(VISUAL_CONTAINER_PREFIX))
                .unwrap_or(&visual.name);
            format!("{VISUAL_CONTAINER_PREFIX}{prefix}{suffix}")
        } else {
            let suffix = short_hash_hex(&format!("{new_page_name}\u{0}{}", visual.name), 8);
            format!("{}{suffix}", visual.name)
        };
        validate_visual_name(&target)?;
        if !targets.insert(target.clone()) {
            return Err(CliError::invalid_args(format!(
                "visual renaming would create duplicate target name: {target}"
            ))
            .with_hint("Choose a different --visual-prefix."));
        }
        renames.insert(visual.name.clone(), target);
    }
    Ok(renames)
}

fn validate_visual_name(name: &str) -> CliResult<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains("..")
        || name
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch == ':' || ch.is_control())
    {
        return Err(CliError::invalid_args(format!(
            "unsafe generated visual name: {name}"
        )));
    }
    Ok(())
}

fn build_page_clone_plan(
    source_page_dir: &Path,
    target_page_dir: &Path,
    new_page_name: &str,
    display_name: &str,
    visual_renames: &BTreeMap<String, String>,
) -> CliResult<PageClonePlan> {
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut filter_renames = Vec::new();
    let mut interaction_drops = Vec::new();
    let mut changes = Vec::new();
    let mut warnings = Vec::new();
    let mut saw_page_json = false;
    let live_visual_names = visual_renames.values().cloned().collect::<BTreeSet<_>>();

    for entry in WalkDir::new(source_page_dir)
        .min_depth(1)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|error| {
            CliError::unexpected(format!(
                "walk source page {}: {error}",
                source_page_dir.display()
            ))
        })?;
        if entry.file_type().is_symlink() {
            return Err(CliError::invalid_args(format!(
                "report pages clone refuses linked page entries: {}",
                entry.path().display()
            )));
        }
        let relative = entry
            .path()
            .strip_prefix(source_page_dir)
            .map_err(|error| {
                CliError::unexpected(format!("resolve source page relative path: {error}"))
            })?;
        let target_relative = rewrite_visual_path(relative, visual_renames);
        if entry.file_type().is_dir() {
            directories.push(target_relative);
            continue;
        }
        if !entry.file_type().is_file() {
            return Err(CliError::invalid_args(format!(
                "report pages clone supports only regular files and directories: {}",
                entry.path().display()
            )));
        }
        if entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            let mut value = read_json_value(entry.path())?;
            rewrite_json_visual_names(&mut value, visual_renames)?;
            let target_path = target_page_dir.join(&target_relative);
            if relative == Path::new("page.json") {
                let object = value.as_object_mut().ok_or_else(|| {
                    CliError::validation_failed(format!(
                        "{} is not a JSON object",
                        entry.path().display()
                    ))
                })?;
                object.insert("name".to_string(), Value::String(new_page_name.to_string()));
                object.insert(
                    "displayName".to_string(),
                    Value::String(display_name.to_string()),
                );
                regenerate_filter_names(
                    &mut value,
                    FilterScope::Page,
                    &target_path,
                    &mut filter_renames,
                    &mut changes,
                )?;
                prune_stale_interactions(
                    &mut value,
                    &live_visual_names,
                    &target_path,
                    &mut interaction_drops,
                    &mut changes,
                    &mut warnings,
                )?;
                saw_page_json = true;
            } else if is_visual_json(relative) {
                let source_visual_name = visual_name_from_path(relative).ok_or_else(|| {
                    CliError::validation_failed(format!(
                        "visual.json is outside a named visual folder: {}",
                        entry.path().display()
                    ))
                })?;
                let target_visual_name =
                    visual_renames.get(source_visual_name).ok_or_else(|| {
                        CliError::validation_failed(format!(
                            "no rename was planned for visual {source_visual_name}"
                        ))
                    })?;
                let object = value.as_object_mut().ok_or_else(|| {
                    CliError::validation_failed(format!(
                        "{} is not a JSON object",
                        entry.path().display()
                    ))
                })?;
                object.insert(
                    "name".to_string(),
                    Value::String(target_visual_name.clone()),
                );
                regenerate_filter_names(
                    &mut value,
                    FilterScope::Visual,
                    &target_path,
                    &mut filter_renames,
                    &mut changes,
                )?;
            }
            files.push(PlannedFile::Json {
                relative: target_relative,
                value,
            });
        } else {
            files.push(PlannedFile::Copy {
                source: entry.path().to_path_buf(),
                relative: target_relative,
            });
        }
    }
    if !saw_page_json {
        return Err(CliError::validation_failed(format!(
            "source page is missing page.json: {}",
            source_page_dir.display()
        )));
    }
    directories.sort_by_key(|path| path.components().count());
    directories.dedup();
    Ok(PageClonePlan {
        directories,
        files,
        filter_renames,
        interaction_drops,
        changes,
        warnings,
    })
}

fn rewrite_visual_path(relative: &Path, visual_renames: &BTreeMap<String, String>) -> PathBuf {
    for (before, after) in visual_renames {
        let visual_root = Path::new("visuals").join(before);
        if let Ok(remainder) = relative.strip_prefix(&visual_root) {
            return Path::new("visuals").join(after).join(remainder);
        }
    }
    relative.to_path_buf()
}

fn is_visual_json(relative: &Path) -> bool {
    relative.components().count() == 3
        && visual_name_from_path(relative).is_some()
        && relative.file_name().and_then(|value| value.to_str()) == Some("visual.json")
}

fn visual_name_from_path(relative: &Path) -> Option<&str> {
    let relative = relative.strip_prefix("visuals").ok()?;
    relative.components().next()?.as_os_str().to_str()
}

fn rewrite_json_visual_names(
    value: &mut Value,
    visual_renames: &BTreeMap<String, String>,
) -> CliResult<()> {
    match value {
        Value::String(text) => {
            *text = rewrite_string(text, visual_renames);
        }
        Value::Array(items) => {
            for item in items {
                rewrite_json_visual_names(item, visual_renames)?;
            }
        }
        Value::Object(object) => {
            let source = std::mem::take(object);
            let mut rewritten = Map::new();
            for (key, mut child) in source {
                rewrite_json_visual_names(&mut child, visual_renames)?;
                let key = rewrite_string(&key, visual_renames);
                if rewritten.insert(key.clone(), child).is_some() {
                    return Err(CliError::validation_failed(format!(
                        "visual name rewriting creates duplicate JSON object key: {key}"
                    )));
                }
            }
            *object = rewritten;
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn rewrite_string(text: &str, visual_renames: &BTreeMap<String, String>) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while cursor < text.len() {
        let remaining = &text[cursor..];
        let next = visual_renames
            .iter()
            .filter_map(|(before, after)| {
                remaining
                    .find(before)
                    .map(|offset| (offset, before.len(), before, after))
            })
            .min_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)));
        let Some((offset, _, before, after)) = next else {
            output.push_str(remaining);
            break;
        };
        output.push_str(&remaining[..offset]);
        output.push_str(after);
        cursor += offset + before.len();
    }
    output
}

fn regenerate_filter_names(
    owner: &mut Value,
    scope: FilterScope,
    path: &Path,
    renames: &mut Vec<Value>,
    changes: &mut Vec<Value>,
) -> CliResult<()> {
    let Some(filter_config) = owner.get_mut("filterConfig") else {
        return Ok(());
    };
    let filter_config = filter_config.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{} filterConfig is not an object", path.display()))
    })?;
    let Some(filters) = filter_config.get_mut("filters") else {
        return Ok(());
    };
    let filters = filters.as_array_mut().ok_or_else(|| {
        CliError::validation_failed(format!(
            "{} filterConfig.filters is not an array",
            path.display()
        ))
    })?;
    let mut used_names = BTreeSet::new();
    for (ordinal, filter) in filters.iter_mut().enumerate() {
        let old_name = filter.get("name").cloned().unwrap_or(Value::Null);
        let base_name = regenerated_filter_name(scope, filter);
        let new_name = unique_filter_name(&base_name, ordinal, &used_names);
        validate_filter_name(&new_name)?;
        let object = filter.as_object_mut().ok_or_else(|| {
            CliError::validation_failed(format!(
                "{} filterConfig.filters[{ordinal}] is not an object",
                path.display()
            ))
        })?;
        object.insert("name".to_string(), Value::String(new_name.clone()));
        used_names.insert(new_name.clone());
        let rename = json!({
            "scope": scope.as_str(),
            "path": canonical_display(path),
            "jsonPointer": format!("/filterConfig/filters/{ordinal}/name"),
            "before": old_name,
            "after": new_name
        });
        renames.push(rename.clone());
        changes.push(json!({
            "kind": "pbir.filter.name",
            "action": "regenerate",
            "path": canonical_display(path),
            "jsonPointer": format!("/filterConfig/filters/{ordinal}/name"),
            "before": old_name,
            "after": new_name
        }));
    }
    Ok(())
}

fn unique_filter_name(base: &str, ordinal: usize, used_names: &BTreeSet<String>) -> String {
    if !used_names.contains(base) {
        return base.to_string();
    }
    for attempt in 0..1000 {
        let suffix = short_hash_hex(&format!("{base}\u{0}{ordinal}\u{0}{attempt}"), 8);
        let keep = 50usize.saturating_sub(1 + suffix.len());
        let candidate = format!("{}D{suffix}", base.chars().take(keep).collect::<String>());
        if !used_names.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("a 32-bit deterministic suffix space cannot be exhausted by one filter owner")
}

fn prune_stale_interactions(
    page: &mut Value,
    live_visual_names: &BTreeSet<String>,
    path: &Path,
    drops: &mut Vec<Value>,
    changes: &mut Vec<Value>,
    warnings: &mut Vec<Value>,
) -> CliResult<()> {
    let Some(interactions) = page.get_mut("visualInteractions") else {
        return Ok(());
    };
    let interactions = interactions.as_array_mut().ok_or_else(|| {
        CliError::validation_failed(format!(
            "{} visualInteractions is not an array",
            path.display()
        ))
    })?;
    let before = std::mem::take(interactions);
    for (ordinal, interaction) in before.into_iter().enumerate() {
        let source = interaction["source"].as_str().unwrap_or_default();
        let target = interaction["target"].as_str().unwrap_or_default();
        let source_exists = live_visual_names.contains(source);
        let target_exists = live_visual_names.contains(target);
        if source_exists && target_exists {
            interactions.push(interaction);
            continue;
        }
        let reason = match (source_exists, target_exists) {
            (false, false) => "source and target visuals do not exist on the cloned page",
            (false, true) => "source visual does not exist on the cloned page",
            (true, false) => "target visual does not exist on the cloned page",
            (true, true) => unreachable!(),
        };
        let drop = json!({
            "ordinal": ordinal,
            "source": source,
            "target": target,
            "reason": reason,
            "before": interaction,
            "after": Value::Null
        });
        drops.push(drop.clone());
        changes.push(json!({
            "kind": "pbir.page.visualInteraction",
            "action": "drop-stale",
            "path": canonical_display(path),
            "jsonPointer": format!("/visualInteractions/{ordinal}"),
            "before": interaction,
            "after": Value::Null,
            "reason": reason
        }));
        warnings.push(json!({
            "code": "page_clone.stale_visual_interaction_dropped",
            "severity": "warning",
            "message": format!("Dropped visual interaction {source} -> {target}: {reason}."),
            "path": canonical_display(path),
            "jsonPointer": format!("/visualInteractions/{ordinal}"),
            "source": source,
            "target": target
        }));
    }
    Ok(())
}

fn materialize_page_clone(plan: &PageClonePlan, target_page_dir: &Path) -> CliResult<()> {
    if target_page_dir.exists() {
        return Err(page_already_exists(
            target_page_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("<target>"),
        ));
    }
    fs::create_dir(target_page_dir).map_err(|error| {
        CliError::unexpected(format!(
            "create target page dir {}: {error}",
            target_page_dir.display()
        ))
    })?;
    let result = (|| {
        for relative in &plan.directories {
            let path = target_page_dir.join(relative);
            fs::create_dir_all(&path).map_err(|error| {
                CliError::unexpected(format!(
                    "create clone directory {}: {error}",
                    path.display()
                ))
            })?;
        }
        for file in &plan.files {
            match file {
                PlannedFile::Json { relative, value } => {
                    let path = target_page_dir.join(relative);
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).map_err(|error| {
                            CliError::unexpected(format!(
                                "create clone directory {}: {error}",
                                parent.display()
                            ))
                        })?;
                    }
                    write_json_new_atomic(&path, value)?;
                }
                PlannedFile::Copy { source, relative } => {
                    let path = target_page_dir.join(relative);
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).map_err(|error| {
                            CliError::unexpected(format!(
                                "create clone directory {}: {error}",
                                parent.display()
                            ))
                        })?;
                    }
                    if path.exists() {
                        return Err(CliError::invalid_args(format!(
                            "clone target file already exists: {}",
                            path.display()
                        )));
                    }
                    fs::copy(source, &path).map_err(|error| {
                        CliError::unexpected(format!(
                            "copy {} to {}: {error}",
                            source.display(),
                            path.display()
                        ))
                    })?;
                }
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(target_page_dir);
    }
    result
}

fn page_order(value: &Value, path: &Path) -> CliResult<Vec<String>> {
    let Some(items) = value["pageOrder"].as_array() else {
        return Err(CliError::validation_failed(format!(
            "{} has no pageOrder array",
            path.display()
        )));
    };
    items
        .iter()
        .map(|item| {
            item.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                CliError::validation_failed(format!(
                    "{} pageOrder contains a non-string entry",
                    path.display()
                ))
            })
        })
        .collect()
}

fn set_page_order(value: &mut Value, order: &[String], path: &Path) -> CliResult<()> {
    let object = value.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{} is not a JSON object", path.display()))
    })?;
    object.insert(
        "pageOrder".to_string(),
        Value::Array(order.iter().cloned().map(Value::String).collect()),
    );
    Ok(())
}

fn target_page_summary(
    source: &PageRecord,
    name: &str,
    display_name: &str,
    page_json_path: &Path,
    visual_renames: &BTreeMap<String, String>,
    ordinal: usize,
) -> Value {
    json!({
        "handle": format!("page:{name}"),
        "name": name,
        "displayName": display_name,
        "ordinal": ordinal,
        "width": source.width,
        "height": source.height,
        "displayOption": source.display_option,
        "type": source.page_type,
        "visibility": source.visibility,
        "pageBinding": source.page_binding,
        "isActive": false,
        "path": canonical_display(page_json_path),
        "visualCount": visual_renames.len(),
        "visualHandles": visual_renames.values().map(|visual| format!("visual:{name}:{visual}")).collect::<Vec<_>>()
    })
}

fn ensure_direct_child(path: &Path, parent: &Path, kind: &str) -> CliResult<()> {
    if path.parent() == Some(parent) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "unsafe {kind} path: {}",
        path.display()
    )))
}

fn clone_dry_run_command() -> &'static str {
    "powerbi-cli report pages clone --project <project-dir-or.pbip> --from <page-name-or-handle> --new-name <ReportSectionX> --dry-run --json"
}

fn short_hash_hex(text: &str, length: usize) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}").chars().take(length).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_visual_stem_is_replaced_by_requested_prefix() {
        let visuals = vec![
            SourceVisual {
                name: "VisualContainerRateCard".to_string(),
            },
            SourceVisual {
                name: "VisualContainerRateTrend".to_string(),
            },
        ];
        let common = common_source_visual_prefix(&visuals);
        assert_eq!(common, "VisualContainerRate");
        let renames =
            visual_rename_map(&visuals, "ReportSectionCopy", Some("P"), &common).expect("renames");
        assert_eq!(renames["VisualContainerRateCard"], "VisualContainerPCard");
        assert_eq!(renames["VisualContainerRateTrend"], "VisualContainerPTrend");
    }

    #[test]
    fn simultaneous_rewrite_handles_names_that_prefix_other_names() {
        let renames = BTreeMap::from([
            ("VisualA".to_string(), "VisualA1111".to_string()),
            ("VisualAB".to_string(), "VisualAB2222".to_string()),
        ]);
        assert_eq!(
            rewrite_string("VisualAB -> VisualA", &renames),
            "VisualAB2222 -> VisualA1111"
        );
    }
}
