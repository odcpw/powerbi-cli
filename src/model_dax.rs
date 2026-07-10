use crate::cli_support::{required_project, take_value};
use crate::tmdl::{ColumnRecord, MeasureRecord, TableDocument, load_table_documents};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct DaxOptions {
    project: Option<PathBuf>,
    engine: Option<String>,
}

pub(crate) fn dax_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(
            CliError::invalid_args("model dax requires a subcommand: bridge-plan")
                .with_hint("Run `model dax bridge-plan` before depending on DAX validation.")
                .with_suggested_command(
                    "powerbi-cli model dax bridge-plan --project <project-dir-or.pbip> --json",
                ),
        );
    };
    match action.as_str() {
        "bridge-plan" | "bridge" | "plan" | "validate-plan" => bridge_plan(rest),
        "dependencies" | "references" | "refs" => dependencies(rest),
        "lint" | "check" => lint(rest),
        other => Err(
            CliError::invalid_args(format!("unknown model dax command: {other}"))
                .with_hint("Supported DAX commands are `bridge-plan`, `dependencies`, and `lint`.")
                .with_suggested_command(
                    "powerbi-cli model dax dependencies --project <project-dir-or.pbip> --json",
                ),
        ),
    }
}

fn bridge_plan(args: &[String]) -> CliResult<Value> {
    let options = parse_args("model dax bridge-plan", args)?;
    let project = required_project(options.project, "model dax bridge-plan")?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let docs = load_table_documents(&resolved)?;
    let requested_engine = options.engine.clone();
    let measures = docs
        .iter()
        .flat_map(|doc| doc.measures.iter())
        .map(measure_json)
        .collect::<Vec<_>>();
    let calculated_columns = docs
        .iter()
        .flat_map(|doc| doc.columns.iter())
        .filter(|column| column.is_calculated())
        .map(calculated_column_json)
        .collect::<Vec<_>>();
    let ok = validation.errors.is_empty();
    let exit_code = if ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let project_arg = command_arg(&resolved.project_dir);
    Ok(json!({
        "schema": "powerbi-cli.model.dax.bridgePlan.v1",
        "ok": ok,
        "exitCode": exit_code,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "counts": {
            "tables": docs.len(),
            "measures": measures.len(),
            "calculatedColumns": calculated_columns.len()
        },
        "daxInventory": {
            "measures": measures,
            "calculatedColumns": calculated_columns
        },
        "bridge": {
            "required": true,
            "noFakeFallbacks": true,
            "requestedEngine": requested_engine,
            "supportedEngines": ["desktop", "xmla", "tabular-editor"],
            "status": "boundary-reported-not-validated"
        },
        "authoringCommands": {
            "measureAdd": format!("powerbi-cli model measures add --project {project_arg} --table <table> --name <measure> --expression-file <dax.txt> --dry-run --json"),
            "measureUpdate": format!("powerbi-cli model measures update --project {project_arg} --handle <measure-handle> --expression-file <dax.txt> --dry-run --json"),
            "calculatedColumnAdd": format!("powerbi-cli model calculated-columns add --project {project_arg} --table <table> --name <column> --expression-file <dax.txt> --data-type <type> --dry-run --json"),
            "calculatedColumnUpdate": format!("powerbi-cli model calculated-columns update --project {project_arg} --handle <column-handle> --expression-file <dax.txt> --dry-run --json")
        },
        "validationBridge": {
            "status": "boundary-reported-not-validated",
            "offlineDaxParser": {
                "available": false,
                "reason": "powerbi-cli does not ship a DAX parser or semantic engine; it will not pretend local TMDL text checks prove DAX compatibility."
            },
            "desktopOracle": {
                "available": false,
                "activation": "Run `POWERBI_DESKTOP_ORACLE=1 powerbi-cli desktop open-check <project> --json` on a Windows machine with Power BI Desktop installed.",
                "scope": "Power BI Desktop remains the compatibility oracle for DAX syntax and semantic binding."
            },
            "futureEngines": [
                {
                    "engine": "Power BI Desktop automation",
                    "status": "planned",
                    "proofRequired": "fixture-backed Desktop open/refresh/readback"
                },
                {
                    "engine": "XMLA/Analysis Services parser",
                    "status": "planned",
                    "proofRequired": "real service integration with credentials outside offline projects"
                }
            ]
        },
        "validation": {
            "ok": validation.errors.is_empty(),
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            format!("powerbi-cli model dax dependencies --project {project_arg} --json"),
            format!("powerbi-cli model dax lint --project {project_arg} --json"),
            format!("powerbi-cli model measures list --project {project_arg} --json"),
            format!("powerbi-cli model calculated-columns list --project {project_arg} --json"),
            format!("powerbi-cli desktop open-check {project_arg} --json")
        ]
    }))
}

fn dependencies(args: &[String]) -> CliResult<Value> {
    let options = parse_args("model dax dependencies", args)?;
    let project = required_project(options.project, "model dax dependencies")?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let docs = load_table_documents(&resolved)?;
    let analysis = analyze_dax(&docs);
    let project_arg = command_arg(&resolved.project_dir);
    let ok = validation.errors.is_empty();
    Ok(json!({
        "schema": "powerbi-cli.model.dax.dependencies.v1",
        "ok": ok,
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "analysisBoundary": {
            "kind": "offline-static-reference-extraction",
            "daxEngineValidated": false,
            "noFakeFallbacks": true,
            "limitations": [
                "This command extracts common DAX table/column and measure references.",
                "It does not parse the complete DAX grammar and does not prove engine semantics.",
                "Run bridge-plan before relying on Desktop/MCP/Fabric validation."
            ]
        },
        "counts": analysis.counts_json(),
        "expressions": analysis.expressions.iter().map(dax_expression_json).collect::<Vec<_>>(),
        "graph": graph_json(&analysis),
        "findings": analysis.findings,
        "validation": {
            "ok": validation.errors.is_empty(),
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            format!("powerbi-cli model dax lint --project {project_arg} --json"),
            format!("powerbi-cli model dax bridge-plan --project {project_arg} --json")
        ]
    }))
}

fn lint(args: &[String]) -> CliResult<Value> {
    let options = parse_args("model dax lint", args)?;
    let project = required_project(options.project, "model dax lint")?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let docs = load_table_documents(&resolved)?;
    let mut analysis = analyze_dax(&docs);
    add_cycle_findings(&mut analysis);
    let error_count = analysis
        .findings
        .iter()
        .filter(|finding| finding["severity"] == "error")
        .count()
        + validation.errors.len();
    let warning_count = analysis
        .findings
        .iter()
        .filter(|finding| finding["severity"] == "warning")
        .count()
        + validation.warnings.len();
    let project_arg = command_arg(&resolved.project_dir);
    Ok(json!({
        "schema": "powerbi-cli.model.dax.lint.v1",
        "ok": error_count == 0,
        "exitCode": if error_count == 0 { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "analysisBoundary": {
            "kind": "offline-static-reference-lint",
            "daxEngineValidated": false,
            "noFakeFallbacks": true,
            "engineValidationCommand": format!("powerbi-cli model dax bridge-plan --project {project_arg} --engine desktop --json")
        },
        "counts": {
            "expressions": analysis.expressions.len(),
            "errors": error_count,
            "warnings": warning_count,
            "findings": analysis.findings.len() + validation.errors.len() + validation.warnings.len()
        },
        "findings": analysis.findings,
        "validation": {
            "ok": validation.errors.is_empty(),
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            format!("powerbi-cli model dax dependencies --project {project_arg} --json"),
            format!("powerbi-cli model dax bridge-plan --project {project_arg} --json")
        ]
    }))
}

#[derive(Debug, Clone)]
pub(crate) struct DaxExpressionAnalysis {
    pub(crate) handle: String,
    pub(crate) kind: String,
    pub(crate) table: String,
    pub(crate) name: String,
    pub(crate) expression: String,
    pub(crate) table_columns: Vec<TableColumnRef>,
    pub(crate) measure_refs: Vec<MeasureRef>,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TableColumnRef {
    pub(crate) table: String,
    pub(crate) column: String,
    pub(crate) resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct MeasureRef {
    pub(crate) name: String,
    pub(crate) table: Option<String>,
    pub(crate) resolved_handles: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DaxAnalysis {
    pub(crate) expressions: Vec<DaxExpressionAnalysis>,
    pub(crate) findings: Vec<Value>,
}

struct DaxReferenceIndex<'a> {
    columns: &'a BTreeMap<String, (String, String)>,
    measures_by_name: &'a BTreeMap<String, Vec<String>>,
}

impl DaxAnalysis {
    fn counts_json(&self) -> Value {
        json!({
            "expressions": self.expressions.len(),
            "measureExpressions": self.expressions.iter().filter(|expr| expr.kind == "measure").count(),
            "calculatedColumnExpressions": self.expressions.iter().filter(|expr| expr.kind == "calculated-column").count(),
            "tableColumnReferences": self.expressions.iter().map(|expr| expr.table_columns.len()).sum::<usize>(),
            "measureReferences": self.expressions.iter().map(|expr| expr.measure_refs.len()).sum::<usize>(),
            "findings": self.findings.len()
        })
    }
}

pub(crate) fn analyze_dax(docs: &[TableDocument]) -> DaxAnalysis {
    let columns = docs
        .iter()
        .flat_map(|doc| {
            doc.columns.iter().map(|column| {
                (
                    canonical_key(&column.table, &column.name),
                    (column.table.clone(), column.name.clone()),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut measures_by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut measures_by_handle = BTreeSet::new();
    for measure in docs.iter().flat_map(|doc| doc.measures.iter()) {
        measures_by_name
            .entry(measure.name.to_ascii_lowercase())
            .or_default()
            .push(measure.handle());
        measures_by_handle.insert(measure.handle());
    }

    let mut expressions = Vec::new();
    for measure in docs.iter().flat_map(|doc| doc.measures.iter()) {
        let index = DaxReferenceIndex {
            columns: &columns,
            measures_by_name: &measures_by_name,
        };
        let expression = analyze_expression_text(
            &measure.handle(),
            "measure",
            &measure.table,
            &measure.name,
            &measure.expression,
            &measure.path,
            &index,
        );
        expressions.push(expression);
    }
    let index = DaxReferenceIndex {
        columns: &columns,
        measures_by_name: &measures_by_name,
    };
    for column in docs
        .iter()
        .flat_map(|doc| doc.columns.iter())
        .filter(|column| column.is_calculated())
    {
        expressions.push(analyze_expression_text(
            &column.handle(),
            "calculated-column",
            &column.table,
            &column.name,
            column.expression.as_deref().unwrap_or_default(),
            &column.path,
            &index,
        ));
    }

    let mut findings = Vec::new();
    for expression in &expressions {
        for table_column in &expression.table_columns {
            if !table_column.resolved {
                findings.push(dax_finding(
                    "dax.reference_missing_column",
                    "error",
                    format!(
                        "{} references missing column '{}'[{}]",
                        expression.handle, table_column.table, table_column.column
                    ),
                    &expression.handle,
                    &expression.path,
                ));
            }
        }
        for measure_ref in &expression.measure_refs {
            match measure_ref.resolved_handles.len() {
                0 => findings.push(dax_finding(
                    "dax.reference_missing_measure",
                    "error",
                    format!(
                        "{} references missing measure [{}]",
                        expression.handle, measure_ref.name
                    ),
                    &expression.handle,
                    &expression.path,
                )),
                1 => {
                    if measure_ref.resolved_handles[0] == expression.handle {
                        findings.push(dax_finding(
                            "dax.reference_self",
                            "error",
                            format!("{} references itself", expression.handle),
                            &expression.handle,
                            &expression.path,
                        ));
                    }
                }
                _ => findings.push(dax_finding(
                    "dax.reference_ambiguous_measure",
                    "warning",
                    format!(
                        "{} references ambiguous measure [{}] resolved to {} handles",
                        expression.handle,
                        measure_ref.name,
                        measure_ref.resolved_handles.len()
                    ),
                    &expression.handle,
                    &expression.path,
                )),
            }
        }
    }

    DaxAnalysis {
        expressions,
        findings,
    }
}

fn analyze_expression_text(
    handle: &str,
    kind: &str,
    table: &str,
    name: &str,
    expression: &str,
    path: &Path,
    index: &DaxReferenceIndex<'_>,
) -> DaxExpressionAnalysis {
    let raw_refs = extract_bracket_references(expression);
    let mut table_columns = BTreeSet::new();
    let mut measure_refs = BTreeSet::new();
    for raw in raw_refs {
        if let Some(table_name) = raw.table {
            let key = canonical_key(&table_name, &raw.name);
            let (resolved_table, resolved_column, resolved) = index
                .columns
                .get(&key)
                .map(|(table, column)| (table.clone(), column.clone(), true))
                .unwrap_or_else(|| (table_name, raw.name.clone(), false));
            table_columns.insert(TableColumnRef {
                table: resolved_table,
                column: resolved_column,
                resolved,
            });
        } else {
            let handles = index
                .measures_by_name
                .get(&raw.name.to_ascii_lowercase())
                .cloned()
                .unwrap_or_default();
            measure_refs.insert(MeasureRef {
                name: raw.name,
                table: None,
                resolved_handles: handles,
            });
        }
    }
    DaxExpressionAnalysis {
        handle: handle.to_string(),
        kind: kind.to_string(),
        table: table.to_string(),
        name: name.to_string(),
        expression: expression.to_string(),
        table_columns: table_columns.into_iter().collect(),
        measure_refs: measure_refs.into_iter().collect(),
        path: path.to_path_buf(),
    }
}

#[derive(Debug)]
struct RawBracketRef {
    table: Option<String>,
    name: String,
}

fn extract_bracket_references(expression: &str) -> Vec<RawBracketRef> {
    let chars = expression.chars().collect::<Vec<_>>();
    let mut refs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            i = skip_double_quoted(&chars, i + 1);
            continue;
        }
        if chars[i] == '['
            && let Some((name, end)) = read_bracket_name(&chars, i + 1)
        {
            refs.push(RawBracketRef {
                table: preceding_table_name(&chars, i),
                name,
            });
            i = end + 1;
            continue;
        }
        i += 1;
    }
    refs
}

fn skip_double_quoted(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() {
        if chars[i] == '"' {
            if i + 1 < chars.len() && chars[i + 1] == '"' {
                i += 2;
            } else {
                return i + 1;
            }
        } else {
            i += 1;
        }
    }
    i
}

fn read_bracket_name(chars: &[char], mut i: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    while i < chars.len() {
        if chars[i] == ']' {
            return Some((name.trim().to_string(), i));
        }
        name.push(chars[i]);
        i += 1;
    }
    None
}

fn preceding_table_name(chars: &[char], bracket_index: usize) -> Option<String> {
    if bracket_index == 0 {
        return None;
    }
    let mut i = bracket_index;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if i == 0 {
        return None;
    }
    if chars[i - 1] == '\'' {
        return preceding_quoted_identifier(chars, i - 1);
    }
    let end = i;
    while i > 0 && is_unquoted_identifier_char(chars[i - 1]) {
        i -= 1;
    }
    if i == end {
        return None;
    }
    Some(chars[i..end].iter().collect::<String>())
}

fn preceding_quoted_identifier(chars: &[char], quote_index: usize) -> Option<String> {
    let mut i = quote_index;
    let mut name = String::new();
    while i > 0 {
        i -= 1;
        if chars[i] == '\'' {
            let reversed = name.chars().rev().collect::<String>();
            return Some(reversed.replace("''", "'"));
        }
        name.push(chars[i]);
    }
    None
}

fn is_unquoted_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '.')
}

fn canonical_key(table: &str, name: &str) -> String {
    format!(
        "{}\u{1f}{}",
        table.to_ascii_lowercase(),
        name.to_ascii_lowercase()
    )
}

fn dax_expression_json(expression: &DaxExpressionAnalysis) -> Value {
    json!({
        "handle": expression.handle,
        "kind": expression.kind,
        "table": expression.table,
        "name": expression.name,
        "expression": expression.expression,
        "path": canonical_display(&expression.path),
        "references": {
            "tableColumns": expression.table_columns.iter().map(|reference| json!({
                "table": reference.table,
                "column": reference.column,
                "handle": crate::tmdl::column_handle(&reference.table, &reference.column),
                "resolved": reference.resolved
            })).collect::<Vec<_>>(),
            "measures": expression.measure_refs.iter().map(|reference| json!({
                "name": reference.name,
                "table": reference.table,
                "resolvedHandles": reference.resolved_handles,
                "resolved": reference.resolved_handles.len() == 1,
                "ambiguous": reference.resolved_handles.len() > 1
            })).collect::<Vec<_>>()
        }
    })
}

fn graph_json(analysis: &DaxAnalysis) -> Value {
    let edges = measure_edges(analysis);
    json!({
        "nodes": analysis.expressions.iter().map(|expression| expression.handle.clone()).collect::<Vec<_>>(),
        "edges": edges.iter().map(|(from, to)| json!({
            "from": from,
            "to": to,
            "kind": "measure-reference"
        })).collect::<Vec<_>>()
    })
}

fn measure_edges(analysis: &DaxAnalysis) -> Vec<(String, String)> {
    let mut edges = BTreeSet::new();
    for expression in analysis
        .expressions
        .iter()
        .filter(|expr| expr.kind == "measure")
    {
        for reference in &expression.measure_refs {
            for handle in &reference.resolved_handles {
                edges.insert((expression.handle.clone(), handle.clone()));
            }
        }
    }
    edges.into_iter().collect()
}

pub(crate) fn add_cycle_findings(analysis: &mut DaxAnalysis) {
    let edges = measure_edges(analysis);
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (from, to) in edges {
        graph.entry(from).or_default().push(to);
    }
    let nodes = graph.keys().cloned().collect::<Vec<_>>();
    for node in nodes {
        let mut stack = Vec::new();
        let mut seen = BTreeSet::new();
        if dfs_cycle(&graph, &node, &node, &mut seen, &mut stack) {
            analysis.findings.push(json!({
                "code": "dax.dependency_cycle",
                "severity": "error",
                "message": format!("DAX measure dependency cycle includes {node}"),
                "handle": node,
                "path": null,
                "cycle": stack
            }));
        }
    }
}

fn dfs_cycle(
    graph: &BTreeMap<String, Vec<String>>,
    start: &str,
    current: &str,
    seen: &mut BTreeSet<String>,
    stack: &mut Vec<String>,
) -> bool {
    if !seen.insert(current.to_string()) {
        return false;
    }
    stack.push(current.to_string());
    for next in graph.get(current).into_iter().flatten() {
        if next == start {
            stack.push(next.clone());
            return true;
        }
        if dfs_cycle(graph, start, next, seen, stack) {
            return true;
        }
    }
    stack.pop();
    false
}

fn dax_finding(code: &str, severity: &str, message: String, handle: &str, path: &Path) -> Value {
    json!({
        "code": code,
        "severity": severity,
        "message": message,
        "handle": handle,
        "path": canonical_display(path)
    })
}

fn parse_args(command: &str, args: &[String]) -> CliResult<DaxOptions> {
    let mut options = DaxOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--engine" => {
                let engine = take_value(args, &mut i, "--engine")?;
                options.engine = Some(parse_engine(&engine)?);
            }
            other if !other.starts_with('-') && options.project.is_none() => {
                options.project = Some(PathBuf::from(other));
                i += 1;
            }
            other => {
                return Err(
                    CliError::invalid_args(format!("unknown {command} flag: {other}"))
                        .with_hint("Run capabilities for the exact DAX bridge contract.")
                        .with_suggested_command(
                            "powerbi-cli --json capabilities --for \"model dax bridge-plan\"",
                        ),
                );
            }
        }
    }
    Ok(options)
}

fn parse_engine(value: &str) -> CliResult<String> {
    match value {
        "desktop" | "xmla" | "tabular-editor" => Ok(value.to_string()),
        other => Err(CliError::invalid_args(format!(
            "invalid DAX bridge engine: {other}"
        ))
        .with_hint("Use --engine desktop, --engine xmla, or --engine tabular-editor.")
        .with_suggested_command(
            "powerbi-cli model dax bridge-plan --project <project-dir-or.pbip> --engine desktop --json",
        )),
    }
}

fn measure_json(measure: &MeasureRecord) -> Value {
    json!({
        "handle": measure.handle(),
        "table": measure.table,
        "name": measure.name,
        "expression": measure.expression,
        "metadata": {
            "lineageTag": measure.lineage_tag,
            "formatString": measure.format_string,
            "displayFolder": measure.display_folder,
            "description": measure.description
        },
        "path": canonical_display(&measure.path),
        "lineRange": {
            "start": measure.start_line + 1,
            "end": measure.end_line
        }
    })
}

fn calculated_column_json(column: &ColumnRecord) -> Value {
    json!({
        "handle": column.handle(),
        "table": column.table,
        "name": column.name,
        "expression": column.expression,
        "dataType": column.data_type,
        "metadata": {
            "lineageTag": column.lineage_tag,
            "formatString": column.format_string,
            "summarizeBy": column.summarize_by,
            "sourceColumn": column.source_column,
            "displayFolder": column.display_folder,
            "description": column.description,
            "isHidden": column.is_hidden,
            "isKey": column.is_key
        },
        "path": canonical_display(&column.path),
        "lineRange": {
            "start": column.start_line + 1,
            "end": column.end_line
        }
    })
}
