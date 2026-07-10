use crate::tmdl::{TableDocument, find_table, load_table_documents, same_name};
use crate::{CliError, CliResult, ResolvedProject};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelationshipRecord {
    pub(crate) name: String,
    pub(crate) from_table: String,
    pub(crate) from_column: String,
    pub(crate) to_table: String,
    pub(crate) to_column: String,
    pub(crate) cross_filtering_behavior: String,
    pub(crate) is_active: bool,
    pub(crate) path: PathBuf,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) block: String,
}

impl RelationshipRecord {
    pub(crate) fn handle(&self) -> String {
        relationship_handle(&self.name)
    }
}

#[derive(Debug)]
pub(crate) struct RelationshipDocument {
    pub(crate) path: PathBuf,
    newline: String,
    had_final_newline: bool,
    lines: Vec<String>,
    pub(crate) relationships: Vec<RelationshipRecord>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct RelationshipSelector {
    pub(crate) handle: Option<String>,
    pub(crate) name: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RelationshipDefinition {
    pub(crate) name: String,
    pub(crate) from_table: String,
    pub(crate) from_column: String,
    pub(crate) to_table: String,
    pub(crate) to_column: String,
    pub(crate) cross_filtering_behavior: String,
    pub(crate) is_active: bool,
}

#[derive(Debug)]
pub(crate) struct RelationshipMutationPlan {
    pub(crate) name: String,
    pub(crate) handle: String,
    pub(crate) path: PathBuf,
    pub(crate) before_block: Option<String>,
    pub(crate) after_block: Option<String>,
    pub(crate) new_text: String,
}

pub(crate) fn load_relationship_document(
    resolved: &ResolvedProject,
) -> CliResult<RelationshipDocument> {
    let path = resolved
        .semantic_model_dir
        .join("definition")
        .join("relationships.tmdl");
    parse_relationship_document(path)
}

pub(crate) fn load_relationships_and_tables(
    resolved: &ResolvedProject,
) -> CliResult<(RelationshipDocument, Vec<TableDocument>)> {
    Ok((
        load_relationship_document(resolved)?,
        load_table_documents(resolved)?,
    ))
}

pub(crate) fn find_relationship<'a>(
    doc: &'a RelationshipDocument,
    selector: &RelationshipSelector,
) -> CliResult<&'a RelationshipRecord> {
    let name = selector_name(selector)?;
    let matches = doc
        .relationships
        .iter()
        .filter(|relationship| same_name(&relationship.name, &name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [record] => Ok(record),
        [] => Err(CliError::validation_failed(format!(
            "relationship not found: {}",
            relationship_handle(&name)
        ))
        .with_hint("Run `powerbi-cli model relationships list --project <project> --json` to get valid handles.")
        .with_suggested_command(
            "powerbi-cli model relationships list --project <project-dir-or.pbip> --json",
        )),
        _ => Err(CliError::validation_failed(format!(
            "relationship selector is ambiguous: {}",
            relationship_handle(&name)
        ))
        .with_hint("Use the exact handle returned by `model relationships list`.")),
    }
}

pub(crate) fn add_relationship_plan(
    doc: &RelationshipDocument,
    tables: &[TableDocument],
    definition: RelationshipDefinition,
) -> CliResult<RelationshipMutationPlan> {
    validate_relationship_definition(tables, &definition)?;
    reject_duplicate_relationship(doc, &definition, None)?;

    let after_lines = relationship_block_lines(&definition);
    let after_block = render_lines(&after_lines, &doc.newline, true);
    let mut lines = doc.lines.clone();
    if !lines.is_empty() && lines.last().is_some_and(|line| !line.is_empty()) {
        lines.push(String::new());
    }
    lines.extend(after_lines);

    Ok(RelationshipMutationPlan {
        name: definition.name.clone(),
        handle: relationship_handle(&definition.name),
        path: doc.path.clone(),
        before_block: None,
        after_block: Some(after_block),
        new_text: render_lines(&lines, &doc.newline, true),
    })
}

pub(crate) fn replace_relationship_plan(
    doc: &RelationshipDocument,
    tables: &[TableDocument],
    selector: &RelationshipSelector,
    definition: RelationshipDefinition,
) -> CliResult<RelationshipMutationPlan> {
    let existing = find_relationship(doc, selector)?;
    if let Some(line) = unsupported_relationship_line(existing) {
        return Err(CliError::unsupported_feature(format!(
            "relationship update would drop unsupported TMDL line: {line}"
        ))
        .with_hint("This relationship contains Desktop-authored metadata this alpha writer does not preserve yet; use delete+add for generated relationships or edit the block manually.")
        .with_suggested_command(
            "powerbi-cli model relationships show --project <project-dir-or.pbip> --handle <relationship-handle> --json",
        ));
    }
    validate_relationship_definition(tables, &definition)?;
    reject_duplicate_relationship(doc, &definition, Some(&existing.name))?;

    let after_lines = relationship_block_lines(&definition);
    let after_block = render_lines(&after_lines, &doc.newline, true);
    let mut lines = doc.lines.clone();
    lines.splice(existing.start_line..existing.end_line, after_lines);

    Ok(RelationshipMutationPlan {
        name: existing.name.clone(),
        handle: existing.handle(),
        path: doc.path.clone(),
        before_block: Some(existing.block.clone()),
        after_block: Some(after_block),
        new_text: render_lines(&lines, &doc.newline, doc.had_final_newline),
    })
}

fn unsupported_relationship_line(record: &RelationshipRecord) -> Option<String> {
    record
        .block
        .lines()
        .skip(1)
        .map(str::trim)
        .find(|line| {
            if line.is_empty() {
                return false;
            }
            match line.split_once(':').map(|(key, _)| key.trim()) {
                Some("fromColumn" | "toColumn" | "crossFilteringBehavior" | "isActive") => false,
                Some(_) | None => true,
            }
        })
        .map(ToOwned::to_owned)
}

pub(crate) fn delete_relationship_plan(
    doc: &RelationshipDocument,
    selector: &RelationshipSelector,
) -> CliResult<RelationshipMutationPlan> {
    let existing = find_relationship(doc, selector)?;
    let mut lines = doc.lines.clone();
    lines.drain(existing.start_line..existing.end_line);

    Ok(RelationshipMutationPlan {
        name: existing.name.clone(),
        handle: existing.handle(),
        path: doc.path.clone(),
        before_block: Some(existing.block.clone()),
        after_block: None,
        new_text: render_lines(&lines, &doc.newline, doc.had_final_newline),
    })
}

pub(crate) fn relationship_handle(name: &str) -> String {
    format!("relationship:{name}")
}

pub(crate) fn default_relationship_name(
    from_table: &str,
    from_column: &str,
    to_table: &str,
    to_column: &str,
) -> String {
    let label = format!("{from_table}_{from_column}_to_{to_table}_{to_column}");
    let slug = slug(&label);
    let hash = hash_hex(&format!("relationship:{label}"));
    let base = if slug.is_empty() {
        "rel".to_string()
    } else {
        format!("rel{slug}")
    };
    let suffix = &hash[..10];
    let mut clean_base = base
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .take(40)
        .collect::<String>();
    clean_base.push_str(suffix);
    clean_base
}

pub(crate) fn normalize_cross_filtering_behavior(value: &str) -> CliResult<String> {
    let normalized = match value.trim().to_ascii_lowercase().as_str() {
        "onedirection" | "one-direction" | "one" | "single" | "single-direction" => "oneDirection",
        "bothdirections" | "both-directions" | "both" | "bidirectional" | "bi-directional" => {
            "bothDirections"
        }
        "automatic" | "auto" => "automatic",
        other => {
            return Err(CliError::invalid_args(format!(
                "invalid relationship cross filtering behavior: {other}"
            ))
            .with_hint("Use one of: oneDirection, bothDirections, automatic.")
            .with_suggested_command(
                "powerbi-cli model relationships add --project <project-dir-or.pbip> --from-table <table> --from-column <column> --to-table <table> --to-column <column> --cross-filtering-behavior oneDirection --dry-run --json",
            ));
        }
    };
    Ok(normalized.to_string())
}

fn parse_relationship_document(path: PathBuf) -> CliResult<RelationshipDocument> {
    let text = fs::read_to_string(&path)
        .map_err(|err| CliError::file_not_found(format!("read {}: {err}", path.display())))?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" }.to_string();
    let had_final_newline = text.ends_with('\n');
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = if normalized.is_empty() {
        Vec::new()
    } else {
        normalized.split('\n').map(ToOwned::to_owned).collect()
    };
    if had_final_newline && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }

    let mut relationships = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if is_relationship_start(&lines[index]) {
            let start = index;
            let mut end = start + 1;
            while end < lines.len() && !is_relationship_start(&lines[end]) {
                end += 1;
            }
            if let Some(record) =
                parse_relationship_block(&path, &newline, &lines[start..end], start)
            {
                relationships.push(record);
            }
            index = end;
        } else {
            index += 1;
        }
    }

    Ok(RelationshipDocument {
        path,
        newline,
        had_final_newline,
        lines,
        relationships,
    })
}

fn parse_relationship_block(
    path: &Path,
    newline: &str,
    lines: &[String],
    start_line: usize,
) -> Option<RelationshipRecord> {
    let first = lines.first()?.trim_start();
    let rest = first.strip_prefix("relationship ")?;
    let (name, _) = parse_tmdl_object(rest)?;
    let mut from_table = None;
    let mut from_column = None;
    let mut to_table = None;
    let mut to_column = None;
    let mut cross_filtering_behavior = "oneDirection".to_string();
    let mut is_active = true;

    for line in lines.iter().skip(1) {
        let trimmed = line.trim_start();
        if let Some((key, value)) = trimmed.split_once(':') {
            let value = value.trim();
            match key.trim() {
                "fromColumn" => {
                    if let Some((table, column)) = parse_column_ref(value) {
                        from_table = Some(table);
                        from_column = Some(column);
                    }
                }
                "toColumn" => {
                    if let Some((table, column)) = parse_column_ref(value) {
                        to_table = Some(table);
                        to_column = Some(column);
                    }
                }
                "crossFilteringBehavior" => {
                    cross_filtering_behavior = value.to_string();
                }
                "isActive" => {
                    is_active = !value.eq_ignore_ascii_case("false");
                }
                _ => {}
            }
        }
    }

    Some(RelationshipRecord {
        name,
        from_table: from_table?,
        from_column: from_column?,
        to_table: to_table?,
        to_column: to_column?,
        cross_filtering_behavior,
        is_active,
        path: path.to_path_buf(),
        start_line,
        end_line: start_line + lines.len(),
        block: render_lines(lines, newline, true),
    })
}

fn relationship_block_lines(definition: &RelationshipDefinition) -> Vec<String> {
    let mut lines = vec![format!(
        "relationship {}",
        tmdl_object_name(&definition.name)
    )];
    lines.push(format!(
        "    fromColumn: {}.{}",
        tmdl_object_ref(&definition.from_table),
        tmdl_object_ref(&definition.from_column)
    ));
    lines.push(format!(
        "    toColumn: {}.{}",
        tmdl_object_ref(&definition.to_table),
        tmdl_object_ref(&definition.to_column)
    ));
    lines.push(format!(
        "    crossFilteringBehavior: {}",
        definition.cross_filtering_behavior
    ));
    if !definition.is_active {
        lines.push("    isActive: false".to_string());
    }
    lines.push(String::new());
    lines
}

fn reject_duplicate_relationship(
    doc: &RelationshipDocument,
    definition: &RelationshipDefinition,
    replacing_name: Option<&str>,
) -> CliResult<()> {
    if doc.relationships.iter().any(|relationship| {
        replacing_name.is_none_or(|name| !same_name(&relationship.name, name))
            && same_name(&relationship.name, &definition.name)
    }) {
        return Err(CliError::invalid_args(format!(
            "relationship already exists: {}",
            relationship_handle(&definition.name)
        ))
        .with_hint("Use `model relationships update` for existing relationships.")
        .with_suggested_command(format!(
            "powerbi-cli model relationships update --project <project-dir-or.pbip> --handle {} --dry-run --json",
            shell_arg(&relationship_handle(&definition.name))
        )));
    }

    if doc.relationships.iter().any(|relationship| {
        replacing_name.is_none_or(|name| !same_name(&relationship.name, name))
            && same_name(&relationship.from_table, &definition.from_table)
            && same_name(&relationship.from_column, &definition.from_column)
            && same_name(&relationship.to_table, &definition.to_table)
            && same_name(&relationship.to_column, &definition.to_column)
    }) {
        return Err(CliError::invalid_args(format!(
            "relationship endpoints already exist: {}.{} -> {}.{}",
            definition.from_table,
            definition.from_column,
            definition.to_table,
            definition.to_column
        ))
        .with_hint("Use `model relationships update` for the existing relationship or choose different endpoints.")
        .with_suggested_command(
            "powerbi-cli model relationships list --project <project-dir-or.pbip> --json",
        ));
    }
    Ok(())
}

fn validate_relationship_definition(
    tables: &[TableDocument],
    definition: &RelationshipDefinition,
) -> CliResult<()> {
    ensure_column_exists(
        tables,
        &definition.from_table,
        &definition.from_column,
        "fromColumn",
    )?;
    ensure_column_exists(
        tables,
        &definition.to_table,
        &definition.to_column,
        "toColumn",
    )?;
    normalize_cross_filtering_behavior(&definition.cross_filtering_behavior)?;
    Ok(())
}

fn ensure_column_exists(
    tables: &[TableDocument],
    table: &str,
    column: &str,
    role: &str,
) -> CliResult<()> {
    let doc = find_table(tables, table)?;
    if doc
        .columns
        .iter()
        .any(|record| same_name(&record.name, column))
    {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "relationship {role} column not found: {table}.{column}"
    ))
    .with_hint(
        "Run `powerbi-cli inspect --deep <project> --json` to get valid table and column handles.",
    )
    .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json"))
}

fn selector_name(selector: &RelationshipSelector) -> CliResult<String> {
    if let Some(handle) = &selector.handle {
        return parse_relationship_handle(handle);
    }
    selector.name.clone().ok_or_else(|| {
        CliError::invalid_args("relationship selector requires --handle or --name")
            .with_hint("Use handles from `powerbi-cli model relationships list --project <project> --json`.")
            .with_suggested_command(
                "powerbi-cli model relationships list --project <project-dir-or.pbip> --json",
            )
    })
}

fn parse_relationship_handle(handle: &str) -> CliResult<String> {
    let Some(name) = handle.strip_prefix("relationship:") else {
        return Err(
            CliError::invalid_args(format!("invalid relationship handle: {handle}"))
                .with_hint("Relationship handles look like `relationship:<relationship name>`.")
                .with_suggested_command(
                    "powerbi-cli model relationships list --project <project-dir-or.pbip> --json",
                ),
        );
    };
    if name.is_empty() {
        return Err(
            CliError::invalid_args(format!("invalid relationship handle: {handle}"))
                .with_hint("Relationship handles look like `relationship:<relationship name>`.")
                .with_suggested_command(
                    "powerbi-cli model relationships list --project <project-dir-or.pbip> --json",
                ),
        );
    }
    Ok(name.to_string())
}

fn is_relationship_start(line: &str) -> bool {
    line.starts_with("relationship ") || line.trim_start().starts_with("relationship ")
}

fn parse_column_ref(value: &str) -> Option<(String, String)> {
    let (table, rest) = parse_ref_part(value)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('.')?;
    let (column, _) = parse_ref_part(rest)?;
    Some((table, column))
}

fn parse_ref_part(value: &str) -> Option<(String, &str)> {
    let trimmed = value.trim_start();
    if trimmed.starts_with('\'') {
        parse_quoted_tmdl_object(trimmed)
    } else {
        let end = trimmed
            .char_indices()
            .find_map(|(index, ch)| (ch.is_whitespace() || ch == '.').then_some(index))
            .unwrap_or(trimmed.len());
        if end == 0 {
            None
        } else {
            Some((trimmed[..end].to_string(), &trimmed[end..]))
        }
    }
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

fn tmdl_object_name(name: &str) -> String {
    if is_simple_identifier(name) {
        name.to_string()
    } else {
        tmdl_object_ref(name)
    }
}

fn tmdl_object_ref(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

fn is_simple_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn render_lines(lines: &[String], newline: &str, final_newline: bool) -> String {
    let mut text = lines.join(newline);
    if final_newline {
        text.push_str(newline);
    }
    text
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut upper_next = true;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper_next {
                out.push(ch.to_ascii_uppercase());
            } else {
                out.push(ch);
            }
            upper_next = false;
        } else {
            upper_next = true;
        }
    }
    out
}

fn hash_hex(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
