use crate::safety_scan::{contains_credential_like_text_str, generated_m_table_safety};
use crate::{CliError, CliResult, ResolvedProject};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct MeasureRecord {
    pub(crate) table: String,
    pub(crate) name: String,
    pub(crate) expression: String,
    pub(crate) lineage_tag: Option<String>,
    pub(crate) format_string: Option<String>,
    pub(crate) display_folder: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) path: PathBuf,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) block: String,
}

impl MeasureRecord {
    pub(crate) fn handle(&self) -> String {
        measure_handle(&self.table, &self.name)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ColumnRecord {
    pub(crate) table: String,
    pub(crate) name: String,
    pub(crate) expression: Option<String>,
    pub(crate) data_type: Option<String>,
    pub(crate) lineage_tag: Option<String>,
    pub(crate) format_string: Option<String>,
    pub(crate) summarize_by: Option<String>,
    pub(crate) source_column: Option<String>,
    pub(crate) display_folder: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) is_hidden: bool,
    pub(crate) is_key: bool,
    pub(crate) path: PathBuf,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) block: String,
}

impl ColumnRecord {
    pub(crate) fn handle(&self) -> String {
        column_handle(&self.table, &self.name)
    }

    pub(crate) fn is_calculated(&self) -> bool {
        self.expression.is_some()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PartitionRecord {
    pub(crate) table: String,
    pub(crate) name: String,
    pub(crate) expression_kind: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) source_kind: String,
    pub(crate) safety: PartitionSafety,
    pub(crate) path: PathBuf,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) source_start_line: Option<usize>,
    pub(crate) source_end_line: Option<usize>,
    pub(crate) block: String,
}

impl PartitionRecord {
    pub(crate) fn handle(&self) -> String {
        partition_handle(&self.table, &self.name)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PartitionSafety {
    pub(crate) status: String,
    pub(crate) findings: Vec<PartitionSafetyFinding>,
}

#[derive(Debug, Clone)]
pub(crate) struct PartitionSafetyFinding {
    pub(crate) code: String,
    pub(crate) severity: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone)]
pub(crate) struct TableDocument {
    pub(crate) table: String,
    pub(crate) path: PathBuf,
    newline: String,
    had_final_newline: bool,
    lines: Vec<String>,
    pub(crate) columns: Vec<ColumnRecord>,
    pub(crate) measures: Vec<MeasureRecord>,
    pub(crate) partitions: Vec<PartitionRecord>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct MeasureSelector {
    pub(crate) handle: Option<String>,
    pub(crate) table: Option<String>,
    pub(crate) name: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ColumnSelector {
    pub(crate) handle: Option<String>,
    pub(crate) table: Option<String>,
    pub(crate) name: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct PartitionSelector {
    pub(crate) handle: Option<String>,
    pub(crate) table: Option<String>,
    pub(crate) name: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct MeasureDefinition {
    pub(crate) name: String,
    pub(crate) expression: String,
    pub(crate) lineage_tag: Option<String>,
    pub(crate) format_string: Option<String>,
    pub(crate) display_folder: Option<String>,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CalculatedColumnDefinition {
    pub(crate) name: String,
    pub(crate) expression: String,
    pub(crate) data_type: String,
    pub(crate) lineage_tag: Option<String>,
    pub(crate) format_string: Option<String>,
    pub(crate) summarize_by: Option<String>,
    pub(crate) display_folder: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) is_hidden: bool,
}

#[derive(Debug)]
pub(crate) struct MutationPlan {
    pub(crate) table: String,
    pub(crate) name: String,
    pub(crate) handle: String,
    pub(crate) path: PathBuf,
    pub(crate) before_block: Option<String>,
    pub(crate) after_block: Option<String>,
    pub(crate) new_text: String,
}

pub(crate) fn load_table_documents(resolved: &ResolvedProject) -> CliResult<Vec<TableDocument>> {
    let tables_dir = resolved
        .semantic_model_dir
        .join("definition")
        .join("tables");
    if !tables_dir.is_dir() {
        return Err(CliError::file_not_found(format!(
            "semantic model tables directory not found: {}",
            tables_dir.display()
        )));
    }

    let mut paths = fs::read_dir(&tables_dir)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", tables_dir.display())))?
        .map(|entry| crate::read_dir_entry(&tables_dir, entry, "load TMDL table documents"))
        .collect::<CliResult<Vec<_>>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("tmdl"))
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.to_string_lossy().cmp(&right.to_string_lossy()));
    paths.into_iter().map(parse_table_document).collect()
}

pub(crate) fn find_measure<'a>(
    docs: &'a [TableDocument],
    selector: &MeasureSelector,
) -> CliResult<&'a MeasureRecord> {
    let (table, name) = selector_parts(selector)?;
    let matches = docs
        .iter()
        .flat_map(|doc| doc.measures.iter())
        .filter(|measure| same_name(&measure.table, &table) && same_name(&measure.name, &name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [record] => Ok(record),
        [] => Err(CliError::validation_failed(format!(
            "measure not found: {}",
            measure_handle(&table, &name)
        ))
        .with_hint("Run `powerbi-cli model measures list --project <project> --json` to get valid handles.")
        .with_suggested_command(
            "powerbi-cli model measures list --project <project-dir-or.pbip> --json",
        )),
        _ => Err(CliError::validation_failed(format!(
            "measure selector is ambiguous: {}",
            measure_handle(&table, &name)
        ))
        .with_hint("Use the exact handle returned by `model measures list`.")),
    }
}

pub(crate) fn find_calculated_column<'a>(
    docs: &'a [TableDocument],
    selector: &ColumnSelector,
) -> CliResult<&'a ColumnRecord> {
    let (table, name) = column_selector_parts(selector)?;
    let matches = docs
        .iter()
        .flat_map(|doc| doc.columns.iter())
        .filter(|column| {
            column.is_calculated()
                && same_name(&column.table, &table)
                && same_name(&column.name, &name)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [record] => Ok(record),
        [] => Err(CliError::validation_failed(format!(
            "calculated column not found: {}",
            column_handle(&table, &name)
        ))
        .with_hint("Run `powerbi-cli model calculated-columns list --project <project> --json` to get valid handles.")
        .with_suggested_command(
            "powerbi-cli model calculated-columns list --project <project-dir-or.pbip> --json",
        )),
        _ => Err(CliError::validation_failed(format!(
            "calculated column selector is ambiguous: {}",
            column_handle(&table, &name)
        ))
        .with_hint("Use the exact handle returned by `model calculated-columns list`.")),
    }
}

pub(crate) fn find_partition<'a>(
    docs: &'a [TableDocument],
    selector: &PartitionSelector,
) -> CliResult<&'a PartitionRecord> {
    let (table, name) = partition_selector_parts(selector)?;
    let matches = docs
        .iter()
        .flat_map(|doc| doc.partitions.iter())
        .filter(|partition| {
            same_name(&partition.table, &table) && same_name(&partition.name, &name)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [record] => Ok(record),
        [] => Err(CliError::validation_failed(format!(
            "partition not found: {}",
            partition_handle(&table, &name)
        ))
        .with_hint("Run `powerbi-cli model partitions list --project <project> --json` to get valid handles.")
        .with_suggested_command(
            "powerbi-cli model partitions list --project <project-dir-or.pbip> --json",
        )),
        _ => Err(CliError::validation_failed(format!(
            "partition selector is ambiguous: {}",
            partition_handle(&table, &name)
        ))
        .with_hint("Use the exact handle returned by `model partitions list`.")),
    }
}

pub(crate) fn find_table<'a>(
    docs: &'a [TableDocument],
    table: &str,
) -> CliResult<&'a TableDocument> {
    let matches = docs
        .iter()
        .filter(|doc| same_name(&doc.table, table))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [doc] => Ok(doc),
        [] => Err(
            CliError::validation_failed(format!("table not found: {table}"))
                .with_hint(
                    "Run `powerbi-cli inspect --deep <project> --json` to get table handles.",
                )
                .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json"),
        ),
        _ => Err(
            CliError::validation_failed(format!("table selector is ambiguous: {table}"))
                .with_hint("Use the exact table name returned by `inspect --deep`."),
        ),
    }
}

pub(crate) fn add_calculated_column_plan(
    docs: &[TableDocument],
    table_name: &str,
    definition: CalculatedColumnDefinition,
) -> CliResult<MutationPlan> {
    let doc = find_table(docs, table_name)?;
    if doc
        .columns
        .iter()
        .any(|column| same_name(&column.name, &definition.name))
    {
        return Err(CliError::invalid_args(format!(
            "column already exists: {}",
            column_handle(&doc.table, &definition.name)
        ))
        .with_hint("Use `model calculated-columns update` for existing calculated columns; base columns cannot be overwritten by this command.")
        .with_suggested_command(format!(
            "powerbi-cli model calculated-columns update --project <project-dir-or.pbip> --handle {} --expression <dax> --dry-run --json",
            shell_arg(&column_handle(&doc.table, &definition.name))
        )));
    }

    let after_lines = calculated_column_block_lines(&doc.table, &definition);
    let after_block = render_lines(&after_lines, &doc.newline, true);
    let mut lines = doc.lines.clone();
    let insert_at = column_insertion_index(doc);
    lines.splice(insert_at..insert_at, after_lines);

    Ok(MutationPlan {
        table: doc.table.clone(),
        name: definition.name.clone(),
        handle: column_handle(&doc.table, &definition.name),
        path: doc.path.clone(),
        before_block: None,
        after_block: Some(after_block),
        new_text: render_lines(&lines, &doc.newline, doc.had_final_newline),
    })
}

pub(crate) fn replace_calculated_column_plan(
    docs: &[TableDocument],
    selector: &ColumnSelector,
    definition: CalculatedColumnDefinition,
) -> CliResult<MutationPlan> {
    let existing = find_calculated_column(docs, selector)?;
    if let Some(line) = unsupported_calculated_column_line(existing) {
        return Err(CliError::unsupported_feature(format!(
            "calculated column update would drop unsupported TMDL line: {line}"
        ))
        .with_hint("This calculated column contains Desktop-authored metadata this alpha writer does not preserve yet; inspect the block and recreate only generated columns.")
        .with_suggested_command(format!(
            "powerbi-cli model calculated-columns show --project <project-dir-or.pbip> --handle {} --json",
            shell_arg(&existing.handle())
        )));
    }
    let doc = find_table(docs, &existing.table)?;
    let after_lines = calculated_column_block_lines(&doc.table, &definition);
    let after_block = render_lines(&after_lines, &doc.newline, true);
    let mut lines = doc.lines.clone();
    lines.splice(existing.start_line..existing.end_line, after_lines);

    Ok(MutationPlan {
        table: doc.table.clone(),
        name: existing.name.clone(),
        handle: existing.handle(),
        path: doc.path.clone(),
        before_block: Some(existing.block.clone()),
        after_block: Some(after_block),
        new_text: render_lines(&lines, &doc.newline, doc.had_final_newline),
    })
}

pub(crate) fn delete_calculated_column_plan(
    docs: &[TableDocument],
    selector: &ColumnSelector,
) -> CliResult<MutationPlan> {
    let existing = find_calculated_column(docs, selector)?;
    let doc = find_table(docs, &existing.table)?;
    let mut lines = doc.lines.clone();
    lines.drain(existing.start_line..existing.end_line);

    Ok(MutationPlan {
        table: doc.table.clone(),
        name: existing.name.clone(),
        handle: existing.handle(),
        path: doc.path.clone(),
        before_block: Some(existing.block.clone()),
        after_block: None,
        new_text: render_lines(&lines, &doc.newline, doc.had_final_newline),
    })
}

pub(crate) fn add_measure_plan(
    docs: &[TableDocument],
    table_name: &str,
    definition: MeasureDefinition,
) -> CliResult<MutationPlan> {
    let doc = find_table(docs, table_name)?;
    if doc
        .measures
        .iter()
        .any(|measure| same_name(&measure.name, &definition.name))
    {
        return Err(CliError::invalid_args(format!(
            "measure already exists: {}",
            measure_handle(&doc.table, &definition.name)
        ))
        .with_hint("Use `model measures update` for existing measures.")
        .with_suggested_command(format!(
            "powerbi-cli model measures update --project <project-dir-or.pbip> --handle {} --expression <dax> --dry-run --json",
            shell_arg(&measure_handle(&doc.table, &definition.name))
        )));
    }

    let after_lines = measure_block_lines(&doc.table, &definition);
    let after_block = render_lines(&after_lines, &doc.newline, true);
    let mut lines = doc.lines.clone();
    let insert_at = insertion_index(doc);
    lines.splice(insert_at..insert_at, after_lines);

    Ok(MutationPlan {
        table: doc.table.clone(),
        name: definition.name.clone(),
        handle: measure_handle(&doc.table, &definition.name),
        path: doc.path.clone(),
        before_block: None,
        after_block: Some(after_block),
        new_text: render_lines(&lines, &doc.newline, doc.had_final_newline),
    })
}

pub(crate) fn replace_measure_plan(
    docs: &[TableDocument],
    selector: &MeasureSelector,
    definition: MeasureDefinition,
) -> CliResult<MutationPlan> {
    let existing = find_measure(docs, selector)?;
    if let Some(line) = unsupported_measure_line(existing) {
        return Err(CliError::unsupported_feature(format!(
            "measure update would drop unsupported TMDL line: {line}"
        ))
        .with_hint("This measure contains Desktop-authored metadata this alpha writer does not preserve yet; inspect the block and recreate only generated measures.")
        .with_suggested_command(format!(
            "powerbi-cli model measures show --project <project-dir-or.pbip> --handle {} --json",
            shell_arg(&existing.handle())
        )));
    }
    let doc = find_table(docs, &existing.table)?;
    let after_lines = measure_block_lines(&doc.table, &definition);
    let after_block = render_lines(&after_lines, &doc.newline, true);
    let mut lines = doc.lines.clone();
    lines.splice(existing.start_line..existing.end_line, after_lines);

    Ok(MutationPlan {
        table: doc.table.clone(),
        name: existing.name.clone(),
        handle: existing.handle(),
        path: doc.path.clone(),
        before_block: Some(existing.block.clone()),
        after_block: Some(after_block),
        new_text: render_lines(&lines, &doc.newline, doc.had_final_newline),
    })
}

pub(crate) fn delete_measure_plan(
    docs: &[TableDocument],
    selector: &MeasureSelector,
) -> CliResult<MutationPlan> {
    let existing = find_measure(docs, selector)?;
    let doc = find_table(docs, &existing.table)?;
    let mut lines = doc.lines.clone();
    lines.drain(existing.start_line..existing.end_line);

    Ok(MutationPlan {
        table: doc.table.clone(),
        name: existing.name.clone(),
        handle: existing.handle(),
        path: doc.path.clone(),
        before_block: Some(existing.block.clone()),
        after_block: None,
        new_text: render_lines(&lines, &doc.newline, doc.had_final_newline),
    })
}

pub(crate) fn replace_partition_source_plan(
    docs: &[TableDocument],
    selector: &PartitionSelector,
    source: &str,
) -> CliResult<MutationPlan> {
    let existing = find_partition(docs, selector)?;
    let source_start = existing.source_start_line.ok_or_else(|| {
        CliError::unsupported_feature(format!(
            "partition has no replaceable source block: {}",
            existing.handle()
        ))
        .with_hint("Only TMDL partitions with an existing `source =` block can be materialized.")
        .with_suggested_command(format!(
            "powerbi-cli model partitions show --project <project-dir-or.pbip> --handle {} --json",
            shell_arg(&existing.handle())
        ))
    })?;
    let source_end = existing.source_end_line.ok_or_else(|| {
        CliError::unsupported_feature(format!(
            "partition source range is incomplete: {}",
            existing.handle()
        ))
    })?;
    let normalized = source
        .trim_start_matches('\u{feff}')
        .trim_matches(['\r', '\n']);
    if normalized.trim().is_empty() {
        return Err(CliError::invalid_args(
            "partition source expression must not be empty",
        ));
    }

    let doc = find_table(docs, &existing.table)?;
    let mut after_lines = vec!["        source =".to_string()];
    for line in normalized.replace("\r\n", "\n").replace('\r', "\n").lines() {
        after_lines.push(format!("            {}", line.trim_end()));
    }
    let after_block = render_lines(&after_lines, &doc.newline, true);
    let before_block = render_lines(&doc.lines[source_start..source_end], &doc.newline, true);
    let mut lines = doc.lines.clone();
    lines.splice(source_start..source_end, after_lines);

    Ok(MutationPlan {
        table: doc.table.clone(),
        name: existing.name.clone(),
        handle: existing.handle(),
        path: doc.path.clone(),
        before_block: Some(before_block),
        after_block: Some(after_block),
        new_text: render_lines(&lines, &doc.newline, doc.had_final_newline),
    })
}

pub(crate) fn selector_parts(selector: &MeasureSelector) -> CliResult<(String, String)> {
    if let Some(handle) = &selector.handle {
        return parse_measure_handle(handle);
    }
    let table = selector.table.clone().ok_or_else(|| {
        CliError::invalid_args("measure selector requires --table")
            .with_hint("Use `--handle <measure-handle>` or `--table <table> --name <measure>`.")
            .with_suggested_command(
                "powerbi-cli model measures show --project <project-dir-or.pbip> --handle <measure-handle> --json",
            )
    })?;
    let name = selector.name.clone().ok_or_else(|| {
        CliError::invalid_args("measure selector requires --name")
            .with_hint("Use `--handle <measure-handle>` or `--table <table> --name <measure>`.")
            .with_suggested_command(
                "powerbi-cli model measures show --project <project-dir-or.pbip> --handle <measure-handle> --json",
            )
    })?;
    Ok((table, name))
}

pub(crate) fn column_selector_parts(selector: &ColumnSelector) -> CliResult<(String, String)> {
    if let Some(handle) = &selector.handle {
        return parse_column_handle(handle);
    }
    let table = selector.table.clone().ok_or_else(|| {
        CliError::invalid_args("column selector requires --table")
            .with_hint("Use `--handle <column-handle>` or `--table <table> --name <column>`.")
            .with_suggested_command(
                "powerbi-cli model calculated-columns show --project <project-dir-or.pbip> --handle <column-handle> --json",
            )
    })?;
    let name = selector.name.clone().ok_or_else(|| {
        CliError::invalid_args("column selector requires --name")
            .with_hint("Use `--handle <column-handle>` or `--table <table> --name <column>`.")
            .with_suggested_command(
                "powerbi-cli model calculated-columns show --project <project-dir-or.pbip> --handle <column-handle> --json",
            )
    })?;
    Ok((table, name))
}

pub(crate) fn partition_selector_parts(
    selector: &PartitionSelector,
) -> CliResult<(String, String)> {
    if let Some(handle) = &selector.handle {
        return parse_partition_handle(handle);
    }
    let table = selector.table.clone().ok_or_else(|| {
        CliError::invalid_args("partition selector requires --table")
            .with_hint("Use `--handle <partition-handle>` or `--table <table> --name <partition>`.")
            .with_suggested_command(
                "powerbi-cli model partitions show --project <project-dir-or.pbip> --handle <partition-handle> --json",
            )
    })?;
    let name = selector.name.clone().ok_or_else(|| {
        CliError::invalid_args("partition selector requires --name")
            .with_hint("Use `--handle <partition-handle>` or `--table <table> --name <partition>`.")
            .with_suggested_command(
                "powerbi-cli model partitions show --project <project-dir-or.pbip> --handle <partition-handle> --json",
            )
    })?;
    Ok((table, name))
}

pub(crate) fn measure_handle(table: &str, name: &str) -> String {
    format!(
        "measure:{}:{}",
        encode_handle_component(table),
        encode_handle_component(name)
    )
}

pub(crate) fn table_handle(name: &str) -> String {
    format!("table:{}", encode_handle_component(name))
}

pub(crate) fn column_handle(table: &str, name: &str) -> String {
    format!(
        "column:{}:{}",
        encode_handle_component(table),
        encode_handle_component(name)
    )
}

pub(crate) fn partition_handle(table: &str, name: &str) -> String {
    format!(
        "partition:{}:{}",
        encode_handle_component(table),
        encode_handle_component(name)
    )
}

fn encode_handle_component(value: &str) -> String {
    value.replace('%', "%25").replace(':', "%3A")
}

pub(crate) fn partition_source_kind_is_external(source_kind: &str) -> bool {
    matches!(
        source_kind,
        "postgresqlDatabase" | "sqlDatabase" | "odbcDataSource" | "webContents" | "externalFile"
    )
}

pub(crate) fn same_name(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

pub(crate) fn parse_table_document(path: PathBuf) -> CliResult<TableDocument> {
    let text = fs::read_to_string(&path)
        .map_err(|err| CliError::file_not_found(format!("read {}: {err}", path.display())))?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" }.to_string();
    let had_final_newline = text.ends_with('\n');
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = normalized
        .split('\n')
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if had_final_newline && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }

    let fallback_name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Table")
        .to_string();
    let mut table = fallback_name;
    for line in &lines {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("table ") {
            if let Some((name, _)) = parse_tmdl_object(rest) {
                table = name;
            }
            break;
        }
    }

    let mut columns = Vec::new();
    let mut measures = Vec::new();
    let mut partitions = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if is_column_start(&lines[index]) {
            let start = description_start(&lines, index);
            let mut end = index + 1;
            while end < lines.len() && !is_table_child_boundary(&lines[end]) {
                end += 1;
            }
            if let Some(record) =
                parse_column_block(&table, &path, &newline, &lines[start..end], start)
            {
                columns.push(record);
            }
            index = end.max(index + 1);
        } else if is_measure_start(&lines[index]) {
            let start = description_start(&lines, index);
            let mut end = index + 1;
            while end < lines.len() && !is_table_child_boundary(&lines[end]) {
                end += 1;
            }
            if let Some(record) =
                parse_measure_block(&table, &path, &newline, &lines[start..end], start)
            {
                measures.push(record);
            }
            index = end.max(index + 1);
        } else if is_partition_start(&lines[index]) {
            let start = index;
            let mut end = start + 1;
            while end < lines.len() && !is_table_child_start(&lines[end]) {
                end += 1;
            }
            if let Some(record) =
                parse_partition_block(&table, &path, &newline, &lines[start..end], start, &columns)
            {
                partitions.push(record);
            }
            index = end;
        } else {
            index += 1;
        }
    }

    Ok(TableDocument {
        table,
        path,
        newline,
        had_final_newline,
        lines,
        columns,
        measures,
        partitions,
    })
}

fn parse_column_block(
    table: &str,
    path: &Path,
    newline: &str,
    lines: &[String],
    start_line: usize,
) -> Option<ColumnRecord> {
    let (leading_description, object_index) = leading_description(lines);
    let object_lines = lines.get(object_index..)?;
    let first = object_lines.first()?.trim_start();
    let rest = first.strip_prefix("column ")?;
    let (name, tail) = parse_tmdl_object(rest)?;
    let expression_head = tail.trim_start().strip_prefix('=').map(str::trim_start);
    let mut expression_lines = Vec::new();
    if let Some(head) = expression_head
        && !head.is_empty()
    {
        expression_lines.push(head.to_string());
    }

    let mut data_type = None;
    let mut lineage_tag = None;
    let mut format_string = None;
    let mut summarize_by = None;
    let mut source_column = None;
    let mut display_folder = None;
    let mut description = leading_description;
    let mut is_hidden = false;
    let mut is_key = false;
    let mut seen_property = false;

    for line in object_lines.iter().skip(1) {
        let trimmed = line.trim_start();
        if trimmed == "isHidden" {
            is_hidden = true;
            seen_property = true;
            continue;
        }
        if trimmed == "isKey" {
            is_key = true;
            seen_property = true;
            continue;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let value = value.trim();
            match key.trim() {
                "dataType" | "datatype" => {
                    data_type = Some(value.to_string());
                    seen_property = true;
                    continue;
                }
                "lineageTag" => {
                    lineage_tag = Some(value.to_string());
                    seen_property = true;
                    continue;
                }
                "formatString" => {
                    format_string = Some(tmdl_string_value(value));
                    seen_property = true;
                    continue;
                }
                "summarizeBy" => {
                    summarize_by = Some(value.to_string());
                    seen_property = true;
                    continue;
                }
                "sourceColumn" => {
                    source_column = Some(tmdl_string_value(value));
                    seen_property = true;
                    continue;
                }
                "displayFolder" => {
                    display_folder = Some(tmdl_string_value(value));
                    seen_property = true;
                    continue;
                }
                "description" => {
                    description = Some(tmdl_string_value(value));
                    seen_property = true;
                    continue;
                }
                _ => {}
            }
        }
        if expression_head.is_some()
            && !seen_property
            && (!trimmed.is_empty() || !expression_lines.is_empty())
        {
            expression_lines.push(strip_expression_indent(line));
        }
    }

    Some(ColumnRecord {
        table: table.to_string(),
        name,
        expression: expression_head.map(|_| expression_lines.join("\n").trim().to_string()),
        data_type,
        lineage_tag,
        format_string,
        summarize_by,
        source_column,
        display_folder,
        description,
        is_hidden,
        is_key,
        path: path.to_path_buf(),
        start_line,
        end_line: start_line + lines.len(),
        block: render_lines(lines, newline, true),
    })
}

fn parse_measure_block(
    table: &str,
    path: &Path,
    newline: &str,
    lines: &[String],
    start_line: usize,
) -> Option<MeasureRecord> {
    let (leading_description, object_index) = leading_description(lines);
    let object_lines = lines.get(object_index..)?;
    let first = object_lines.first()?.trim_start();
    let rest = first.strip_prefix("measure ")?;
    let (name, tail) = parse_tmdl_object(rest)?;
    let expression_head = tail
        .trim_start()
        .strip_prefix('=')
        .map(str::trim_start)
        .unwrap_or_default();
    let mut expression_lines = Vec::new();
    if !expression_head.is_empty() {
        expression_lines.push(expression_head.to_string());
    }

    let mut lineage_tag = None;
    let mut format_string = None;
    let mut display_folder = None;
    let mut description = leading_description;
    for line in object_lines.iter().skip(1) {
        let trimmed = line.trim_start();
        if let Some((key, value)) = trimmed.split_once(':') {
            let value = value.trim();
            match key.trim() {
                "lineageTag" => {
                    lineage_tag = Some(value.to_string());
                    continue;
                }
                "formatString" => {
                    format_string = Some(tmdl_string_value(value));
                    continue;
                }
                "displayFolder" => {
                    display_folder = Some(tmdl_string_value(value));
                    continue;
                }
                "description" => {
                    description = Some(tmdl_string_value(value));
                    continue;
                }
                _ => {}
            }
        }
        if !trimmed.is_empty() || !expression_lines.is_empty() {
            expression_lines.push(strip_expression_indent(line));
        }
    }

    Some(MeasureRecord {
        table: table.to_string(),
        name,
        expression: expression_lines.join("\n").trim().to_string(),
        lineage_tag,
        format_string,
        display_folder,
        description,
        path: path.to_path_buf(),
        start_line,
        end_line: start_line + lines.len(),
        block: render_lines(lines, newline, true),
    })
}

fn parse_partition_block(
    table: &str,
    path: &Path,
    newline: &str,
    lines: &[String],
    start_line: usize,
    columns: &[ColumnRecord],
) -> Option<PartitionRecord> {
    let first = lines.first()?.trim_start();
    let rest = first.strip_prefix("partition ")?;
    let (name, tail) = parse_tmdl_object(rest)?;
    let expression_kind = tail
        .trim_start()
        .strip_prefix('=')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let mut mode = None;
    let mut source_lines = Vec::new();
    let mut source_start = None;
    let mut source_end = None;
    let mut in_source = false;
    for (offset, line) in lines.iter().enumerate().skip(1) {
        let trimmed = line.trim_start();
        if !in_source
            && let Some((key, value)) = trimmed.split_once(':')
            && key.trim() == "mode"
        {
            mode = Some(value.trim().to_string());
            continue;
        }
        if !in_source && let Some(rest) = trimmed.strip_prefix("source =") {
            in_source = true;
            source_start = Some(start_line + offset);
            if !rest.trim().is_empty() {
                source_lines.push(rest.trim_start().to_string());
            }
            source_end = Some(start_line + offset + 1);
            continue;
        }
        if in_source {
            source_lines.push(strip_source_indent(line));
            source_end = Some(start_line + offset + 1);
        }
    }
    let source = if source_lines.is_empty() {
        None
    } else {
        Some(source_lines.join("\n").trim().to_string())
    };
    let expected_source_columns = columns
        .iter()
        .filter(|column| !column.is_calculated())
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let (source_kind, safety) =
        classify_partition_source(source.as_deref(), &expected_source_columns);

    Some(PartitionRecord {
        table: table.to_string(),
        name,
        expression_kind,
        mode,
        source,
        source_kind,
        safety,
        path: path.to_path_buf(),
        start_line,
        end_line: start_line + lines.len(),
        source_start_line: source_start,
        source_end_line: source_end,
        block: render_lines(lines, newline, true),
    })
}

pub(crate) fn measure_block_lines(table: &str, definition: &MeasureDefinition) -> Vec<String> {
    let mut lines = Vec::new();
    let name = tmdl_object_name(&definition.name);
    let expression = definition
        .expression
        .trim_start_matches('\u{feff}')
        .trim_end_matches(['\r', '\n']);
    push_tmdl_description(&mut lines, "    ", definition.description.as_deref());
    if expression.contains('\n') || expression.contains('\r') {
        lines.push(format!("    measure {name} ="));
        for line in expression.replace("\r\n", "\n").replace('\r', "\n").lines() {
            lines.push(format!("        {}", line.trim_end()));
        }
    } else {
        lines.push(format!("    measure {name} = {}", expression.trim()));
    }
    lines.push(format!(
        "        lineageTag: {}",
        definition
            .lineage_tag
            .clone()
            .unwrap_or_else(|| stable_guid(&format!("measure:{table}:{}", definition.name)))
    ));
    if let Some(format_string) = &definition.format_string {
        lines.push(format!(
            "        formatString: {}",
            tmdl_string_literal(format_string)
        ));
    }
    if let Some(display_folder) = &definition.display_folder {
        lines.push(format!(
            "        displayFolder: {}",
            tmdl_string_literal(display_folder)
        ));
    }
    lines.push(String::new());
    lines
}

fn calculated_column_block_lines(
    table: &str,
    definition: &CalculatedColumnDefinition,
) -> Vec<String> {
    let mut lines = Vec::new();
    let name = tmdl_object_name(&definition.name);
    let expression = definition
        .expression
        .trim_start_matches('\u{feff}')
        .trim_end_matches(['\r', '\n']);
    push_tmdl_description(&mut lines, "    ", definition.description.as_deref());
    if expression.contains('\n') || expression.contains('\r') {
        lines.push(format!("    column {name} ="));
        for line in expression.replace("\r\n", "\n").replace('\r', "\n").lines() {
            lines.push(format!("        {}", line.trim_end()));
        }
    } else {
        lines.push(format!("    column {name} = {}", expression.trim()));
    }
    lines.push(format!("        dataType: {}", definition.data_type));
    lines.push(format!(
        "        lineageTag: {}",
        definition
            .lineage_tag
            .clone()
            .unwrap_or_else(|| stable_guid(&format!(
                "calculated-column:{table}:{}",
                definition.name
            )))
    ));
    lines.push(format!(
        "        summarizeBy: {}",
        definition
            .summarize_by
            .clone()
            .unwrap_or_else(|| "none".to_string())
    ));
    if definition.is_hidden {
        lines.push("        isHidden".to_string());
    }
    if let Some(format_string) = &definition.format_string {
        lines.push(format!(
            "        formatString: {}",
            tmdl_string_literal(format_string)
        ));
    }
    if let Some(display_folder) = &definition.display_folder {
        lines.push(format!(
            "        displayFolder: {}",
            tmdl_string_literal(display_folder)
        ));
    }
    lines.push(String::new());
    lines
}

fn unsupported_measure_line(record: &MeasureRecord) -> Option<String> {
    unsupported_expression_object_line(
        &record.block,
        &["lineageTag", "formatString", "displayFolder", "description"],
        &[],
    )
}

fn unsupported_calculated_column_line(record: &ColumnRecord) -> Option<String> {
    unsupported_expression_object_line(
        &record.block,
        &[
            "dataType",
            "datatype",
            "lineageTag",
            "formatString",
            "summarizeBy",
            "displayFolder",
            "description",
        ],
        &["isHidden"],
    )
}

fn unsupported_expression_object_line(
    block: &str,
    allowed_properties: &[&str],
    allowed_flags: &[&str],
) -> Option<String> {
    let mut seen_property = false;
    let mut seen_object_declaration = false;
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("///") && !seen_object_declaration {
            continue;
        }
        if !seen_object_declaration {
            seen_object_declaration = true;
            continue;
        }
        if allowed_flags.contains(&trimmed) {
            seen_property = true;
            continue;
        }
        if let Some((key, _)) = trimmed.split_once(':') {
            let key = key.trim();
            if allowed_properties.contains(&key) {
                seen_property = true;
                continue;
            }
            if seen_property || looks_like_tmdl_property_key(key) {
                return Some(trimmed.to_string());
            }
            continue;
        }
        if seen_property || looks_like_tmdl_child_line(trimmed) {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn looks_like_tmdl_property_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn looks_like_tmdl_child_line(line: &str) -> bool {
    line == "isHidden"
        || line == "isKey"
        || line.starts_with("annotation ")
        || line.starts_with("changedProperty ")
        || line.starts_with("extendedProperty ")
        || line.starts_with("formatStringDefinition")
        || line.starts_with("variation ")
}

fn is_column_start(line: &str) -> bool {
    line.starts_with("    column ")
}

fn is_measure_start(line: &str) -> bool {
    line.starts_with("    measure ")
}

fn is_partition_start(line: &str) -> bool {
    line.starts_with("    partition ")
}

fn is_table_child_start(line: &str) -> bool {
    line.starts_with("    column ")
        || line.starts_with("    measure ")
        || line.starts_with("    partition ")
        || line.starts_with("    hierarchy ")
        || line.starts_with("    annotation ")
}

fn is_table_child_boundary(line: &str) -> bool {
    is_table_child_start(line) || is_description_line(line)
}

fn is_description_line(line: &str) -> bool {
    line.trim_start().starts_with("///")
}

fn description_start(lines: &[String], object_index: usize) -> usize {
    let mut start = object_index;
    while start > 0 && is_description_line(&lines[start - 1]) {
        start -= 1;
    }
    start
}

fn leading_description(lines: &[String]) -> (Option<String>, usize) {
    let mut description = Vec::new();
    let mut index = 0;
    while let Some(line) = lines.get(index) {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("///") else {
            break;
        };
        description.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        index += 1;
    }
    let description = (!description.is_empty()).then(|| description.join("\n"));
    (description, index)
}

fn push_tmdl_description(lines: &mut Vec<String>, indent: &str, description: Option<&str>) {
    let Some(description) = description else {
        return;
    };
    for line in description
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
    {
        if line.is_empty() {
            lines.push(format!("{indent}///"));
        } else {
            lines.push(format!("{indent}/// {line}"));
        }
    }
}

fn strip_expression_indent(line: &str) -> String {
    line.strip_prefix("        ")
        .or_else(|| line.strip_prefix("    "))
        .unwrap_or(line)
        .to_string()
}

fn strip_source_indent(line: &str) -> String {
    line.strip_prefix("            ")
        .or_else(|| line.strip_prefix("        "))
        .or_else(|| line.strip_prefix("    "))
        .unwrap_or(line)
        .to_string()
}

fn classify_partition_source(
    source: Option<&str>,
    expected_columns: &[String],
) -> (String, PartitionSafety) {
    let Some(source) = source else {
        return (
            "missing".to_string(),
            PartitionSafety {
                status: "unsafe".to_string(),
                findings: vec![PartitionSafetyFinding {
                    code: "partition.source_missing".to_string(),
                    severity: "error".to_string(),
                    message: "partition has no source expression".to_string(),
                }],
            },
        );
    };
    let normalized = source.to_ascii_lowercase();
    let mut findings = Vec::new();
    let source_kind = if normalized.contains("postgresql.database(") {
        findings.push(partition_finding(
            "partition.real_connector.postgres",
            "error",
            "partition source uses PostgreSQL.Database; replace with dummy #table before home handoff",
        ));
        "postgresqlDatabase"
    } else if normalized.contains("sql.database(") {
        findings.push(partition_finding(
            "partition.real_connector.sql",
            "error",
            "partition source uses Sql.Database; replace with dummy #table before home handoff",
        ));
        "sqlDatabase"
    } else if normalized.contains("odbc.datasource(") {
        findings.push(partition_finding(
            "partition.real_connector.odbc",
            "error",
            "partition source uses Odbc.DataSource; replace with dummy #table before home handoff",
        ));
        "odbcDataSource"
    } else if normalized.contains("web.contents(") {
        findings.push(partition_finding(
            "partition.real_connector.web",
            "error",
            "partition source uses Web.Contents; replace with dummy #table before home handoff",
        ));
        "webContents"
    } else if normalized.contains("file.contents(")
        || normalized.contains("folder.files(")
        || normalized.contains("csv.document(")
        || normalized.contains("excel.workbook(")
    {
        findings.push(partition_finding(
            "partition.real_connector.file",
            "error",
            "partition source reads an external file; replace with dummy #table before home handoff",
        ));
        "externalFile"
    } else if normalized.contains("#table") {
        let table_safety = generated_m_table_safety(source, expected_columns);
        if !table_safety.valid_generator_shape {
            findings.push(partition_finding(
                "partition.dummy_table_shape_unverified",
                "warning",
                "partition contains #table text but does not match the model column list and generated literal row shape",
            ));
            "unknown"
        } else {
            if table_safety.pii_suspect {
                findings.push(partition_finding(
                    "partition.pii_suspect_literal",
                    "warning",
                    "dummy #table row literals may contain personal or long free-text data; review before offline handoff",
                ));
            }
            "dummyMTable"
        }
    } else {
        findings.push(partition_finding(
            "partition.source_unknown",
            "warning",
            "partition source is not a recognized dummy #table expression",
        ));
        "unknown"
    };

    if contains_credential_like_text_str(source) {
        findings.push(partition_finding(
            "partition.credential_like_text",
            "error",
            "partition source contains credential-like text",
        ));
    }

    let status = if findings.iter().any(|finding| finding.severity == "error") {
        "unsafe"
    } else if findings.is_empty() {
        "safe"
    } else {
        "review"
    };
    (
        source_kind.to_string(),
        PartitionSafety {
            status: status.to_string(),
            findings,
        },
    )
}

fn partition_finding(code: &str, severity: &str, message: &str) -> PartitionSafetyFinding {
    PartitionSafetyFinding {
        code: code.to_string(),
        severity: severity.to_string(),
        message: message.to_string(),
    }
}

fn insertion_index(doc: &TableDocument) -> usize {
    doc.lines
        .iter()
        .position(|line| line.starts_with("    partition "))
        .unwrap_or(doc.lines.len())
}

fn column_insertion_index(doc: &TableDocument) -> usize {
    doc.lines
        .iter()
        .position(|line| line.starts_with("    measure ") || line.starts_with("    partition "))
        .unwrap_or(doc.lines.len())
}

fn render_lines(lines: &[String], newline: &str, final_newline: bool) -> String {
    let mut text = lines.join(newline);
    if final_newline {
        text.push_str(newline);
    }
    text
}

pub(crate) fn parse_measure_handle(handle: &str) -> CliResult<(String, String)> {
    parse_two_component_handle(
        handle,
        "measure",
        "Measure handles look like `measure:<table>:<measure name>`; literal `%` and `:` in components are encoded as `%25` and `%3A`.",
        "powerbi-cli model measures list --project <project-dir-or.pbip> --json",
    )
}

fn parse_column_handle(handle: &str) -> CliResult<(String, String)> {
    parse_two_component_handle(
        handle,
        "column",
        "Column handles look like `column:<table>:<column name>`; literal `%` and `:` in components are encoded as `%25` and `%3A`.",
        "powerbi-cli model calculated-columns list --project <project-dir-or.pbip> --json",
    )
}

fn parse_partition_handle(handle: &str) -> CliResult<(String, String)> {
    parse_two_component_handle(
        handle,
        "partition",
        "Partition handles look like `partition:<table>:<partition name>`; literal `%` and `:` in components are encoded as `%25` and `%3A`.",
        "powerbi-cli model partitions list --project <project-dir-or.pbip> --json",
    )
}

fn parse_two_component_handle(
    handle: &str,
    kind: &str,
    hint: &str,
    suggested_command: &str,
) -> CliResult<(String, String)> {
    let invalid = || {
        CliError::invalid_args(format!("invalid {kind} handle: {handle}"))
            .with_hint(hint)
            .with_suggested_command(suggested_command)
    };
    let rest = handle
        .strip_prefix(&format!("{kind}:"))
        .ok_or_else(invalid)?;
    let (table, name) = rest.split_once(':').ok_or_else(invalid)?;
    if table.is_empty() || name.is_empty() || name.contains(':') {
        return Err(invalid());
    }
    let table = decode_handle_component(table).map_err(|_| invalid())?;
    let name = decode_handle_component(name).map_err(|_| invalid())?;
    Ok((table, name))
}

fn decode_handle_component(value: &str) -> Result<String, ()> {
    let mut decoded = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            decoded.push(ch);
            continue;
        }
        let first = chars.next().ok_or(())?;
        let second = chars.next().ok_or(())?;
        match (first, second) {
            ('2', '5') => decoded.push('%'),
            ('3', 'A' | 'a') => decoded.push(':'),
            _ => return Err(()),
        }
    }
    Ok(decoded)
}

fn parse_tmdl_object(value: &str) -> Option<(String, &str)> {
    let trimmed = value.trim_start();
    if trimmed.starts_with('\'') {
        parse_quoted_tmdl_object(trimmed)
    } else {
        let end = trimmed
            .char_indices()
            .find_map(|(index, ch)| ch.is_whitespace().then_some(index))
            .unwrap_or(trimmed.len());
        if end == 0 {
            None
        } else {
            Some((trimmed[..end].to_string(), &trimmed[end..]))
        }
    }
}

fn parse_quoted_tmdl_object(value: &str) -> Option<(String, &str)> {
    let mut name = String::new();
    let mut chars = value.char_indices().peekable();
    let (_, first) = chars.next()?;
    if first != '\'' {
        return None;
    }
    while let Some((index, ch)) = chars.next() {
        if ch == '\'' {
            if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                name.push('\'');
                chars.next();
            } else {
                return Some((name, &value[index + ch.len_utf8()..]));
            }
        } else {
            name.push(ch);
        }
    }
    None
}

fn tmdl_string_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].replace("\"\"", "\"")
    } else {
        trimmed.to_string()
    }
}

fn tmdl_object_name(name: &str) -> String {
    if is_simple_identifier(name) {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

fn tmdl_string_literal(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn is_simple_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}

fn stable_guid(value: &str) -> String {
    let a = hash_hex(value);
    let b = hash_hex(&format!("{value}:powerbi-cli"));
    let hex = format!("{a}{b}");
    format!(
        "{}-{}-4{}-a{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[16..19],
        &hex[19..31]
    )
}

fn hash_hex(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
