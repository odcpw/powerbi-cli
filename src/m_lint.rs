use crate::rules;
use crate::tmdl::{ColumnRecord, load_table_documents};
use crate::{CliError, CliResult, ResolvedProject, canonical_display};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn buffer_reuse_findings(resolved: &ResolvedProject) -> CliResult<Vec<Value>> {
    let mut findings = Vec::new();
    let docs = match load_table_documents(resolved) {
        Ok(docs) => docs,
        Err(error) if error.code == "file_not_found" => Vec::new(),
        Err(error) => return Err(error),
    };
    for doc in &docs {
        let numeric_columns = numeric_source_columns(&doc.columns);
        for partition in &doc.partitions {
            let Some(source) = partition.source.as_deref() else {
                continue;
            };
            let handle = partition.handle();
            findings.extend(document_findings(
                source,
                "partition",
                &handle,
                &partition.path,
            ));
            findings.extend(untyped_expansion_document_findings(
                source,
                "partition",
                &handle,
                &partition.path,
                &numeric_columns,
            ));
        }
    }

    for expression in load_named_m_expressions(&resolved.semantic_model_dir)? {
        findings.extend(document_findings(
            &expression.source,
            "expression",
            &format!("expression:{}", expression.name),
            &expression.path,
        ));
    }
    Ok(findings)
}

fn document_findings(source: &str, document_kind: &str, handle: &str, path: &Path) -> Vec<Value> {
    analyze_let_steps(source)
        .into_iter()
        .map(|reuse| {
            json!({
                "code": rules::M_UNBUFFERED_REUSE,
                "severity": "warning",
                "message": format!(
                    "M step `{}` is referenced {} times by later steps without Table.Buffer",
                    reuse.step, reuse.reference_count
                ),
                "handle": handle,
                "path": canonical_display(path),
                "documentKind": document_kind,
                "step": reuse.step,
                "stepKind": reuse.step_kind.as_str(),
                "referenceCount": reuse.reference_count,
                "analysisBoundary": "heuristic"
            })
        })
        .collect()
}

fn untyped_expansion_document_findings(
    source: &str,
    document_kind: &str,
    handle: &str,
    path: &Path,
    numeric_source_columns: &BTreeSet<String>,
) -> Vec<Value> {
    analyze_untyped_expansions(source, numeric_source_columns)
        .into_iter()
        .map(|expansion| {
            json!({
                "code": rules::M_UNTYPED_EXPANSION,
                "severity": "warning",
                "message": format!(
                    "M step `{}` expands column `{}` without Table.TransformColumnTypes; expanded columns are untyped and can load as text despite a numeric TMDL declaration",
                    expansion.step, expansion.column
                ),
                "handle": handle,
                "path": canonical_display(path),
                "documentKind": document_kind,
                "step": expansion.step,
                "stepKind": expansion.step_kind.as_str(),
                "column": expansion.column,
                "analysisBoundary": "heuristic"
            })
        })
        .collect()
}

fn numeric_source_columns(columns: &[ColumnRecord]) -> BTreeSet<String> {
    columns
        .iter()
        .filter(|column| !column.is_calculated())
        .filter(|column| {
            column
                .data_type
                .as_deref()
                .is_some_and(is_numeric_tmdl_type)
        })
        .filter_map(|column| column.source_column.clone())
        .collect()
}

fn is_numeric_tmdl_type(data_type: &str) -> bool {
    matches!(
        data_type.trim().to_ascii_lowercase().as_str(),
        "double" | "int64" | "decimal"
    )
}

#[derive(Debug)]
struct NamedMExpression {
    name: String,
    source: String,
    path: PathBuf,
}

fn load_named_m_expressions(semantic_model_dir: &Path) -> CliResult<Vec<NamedMExpression>> {
    let definition = semantic_model_dir.join("definition");
    let mut paths = Vec::new();
    let root = definition.join("expressions.tmdl");
    if root.is_file() {
        paths.push(root);
    }
    let folder = definition.join("expressions");
    if folder.is_dir() {
        for entry in fs::read_dir(&folder)
            .map_err(|error| CliError::unexpected(format!("read {}: {error}", folder.display())))?
        {
            let path = entry
                .map_err(|error| {
                    CliError::unexpected(format!("read {}: {error}", folder.display()))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) == Some("tmdl") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths.dedup();

    let mut expressions = Vec::new();
    for path in paths {
        let text = fs::read_to_string(&path)
            .map_err(|error| CliError::unexpected(format!("read {}: {error}", path.display())))?;
        let lines = text.lines().collect::<Vec<_>>();
        let starts = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| expression_name(line).map(|name| (index, name)))
            .collect::<Vec<_>>();
        for (ordinal, (start, name)) in starts.iter().enumerate() {
            let end = starts
                .get(ordinal + 1)
                .map(|(index, _)| *index)
                .unwrap_or(lines.len());
            expressions.push(NamedMExpression {
                name: name.clone(),
                source: lines[*start..end].join("\n"),
                path: path.clone(),
            });
        }
    }
    Ok(expressions)
}

fn expression_name(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("expression ")?.trim_start();
    if let Some(rest) = rest.strip_prefix('\'') {
        let mut name = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(character) = chars.next() {
            if character == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    name.push('\'');
                } else {
                    return Some(name);
                }
            } else {
                name.push(character);
            }
        }
        return None;
    }
    rest.split(|character: char| character.is_whitespace() || character == '=')
        .find(|part| !part.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Identifier(String),
    String(String),
    Number,
    Let,
    In,
    Equal,
    Arrow,
    Comma,
    Open(char),
    Close(char),
    Dot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepKind {
    FunctionDefinition,
    ScalarLiteral,
    RecordLiteral,
    ListLiteral,
    TableLiteral,
    Navigation,
    Other,
}

impl StepKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::FunctionDefinition => "functionDefinition",
            Self::ScalarLiteral => "scalarLiteral",
            Self::RecordLiteral => "recordLiteral",
            Self::ListLiteral => "listLiteral",
            Self::TableLiteral => "tableLiteral",
            Self::Navigation => "navigation",
            Self::Other => "other",
        }
    }

    fn fires_unbuffered_reuse(self) -> bool {
        matches!(self, Self::Other | Self::TableLiteral | Self::Navigation)
    }
}

#[derive(Debug)]
struct Step {
    name: String,
    rhs: Vec<Token>,
}

#[derive(Debug, PartialEq, Eq)]
struct Reuse {
    step: String,
    reference_count: usize,
    step_kind: StepKind,
}

#[derive(Debug, PartialEq, Eq)]
struct UntypedExpansion {
    step: String,
    column: String,
    step_kind: StepKind,
}

fn analyze_let_steps(source: &str) -> Vec<Reuse> {
    let tokens = lex_m(source);
    let Some(let_index) = tokens.iter().position(|token| *token == Token::Let) else {
        return Vec::new();
    };
    let (steps, _) = parse_let_body(&tokens[let_index + 1..]);
    let mut reuse = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let later = &steps[index + 1..];
        let reference_count = later
            .iter()
            .flat_map(|candidate| candidate.rhs.iter())
            .filter(|token| matches!(token, Token::Identifier(name) if name == &step.name))
            .count();
        if reference_count < 2 {
            continue;
        }
        let step_kind = classify_step_kind(&step.rhs);
        if !step_kind.fires_unbuffered_reuse() {
            continue;
        }
        let buffered_definition = starts_with_table_buffer(&step.rhs);
        let buffered_later = later
            .iter()
            .any(|candidate| table_buffer_wraps(&candidate.rhs, &step.name));
        if !buffered_definition && !buffered_later {
            reuse.push(Reuse {
                step: step.name.clone(),
                reference_count,
                step_kind,
            });
        }
    }
    reuse
}

fn analyze_untyped_expansions(
    source: &str,
    numeric_source_columns: &BTreeSet<String>,
) -> Vec<UntypedExpansion> {
    if numeric_source_columns.is_empty() {
        return Vec::new();
    }
    let tokens = lex_m(source);
    let Some(let_index) = tokens.iter().position(|token| *token == Token::Let) else {
        return Vec::new();
    };
    let (steps, in_expr) = parse_let_body(&tokens[let_index + 1..]);
    let retyped = steps
        .iter()
        .flat_map(|step| transform_column_type_names(&step.rhs))
        .chain(transform_column_type_names(&in_expr))
        .collect::<BTreeSet<_>>();
    let mut expansions = Vec::new();
    for step in &steps {
        for column in expand_output_columns(&step.rhs) {
            if retyped.contains(&column) || !numeric_source_columns.contains(&column) {
                continue;
            }
            expansions.push(UntypedExpansion {
                step: step.name.clone(),
                column,
                step_kind: classify_step_kind(&step.rhs),
            });
        }
    }
    expansions
}

fn parse_let_body(tokens: &[Token]) -> (Vec<Step>, Vec<Token>) {
    let mut chunks = Vec::<Vec<Token>>::new();
    let mut current = Vec::new();
    let mut delimiter_depth = 0_usize;
    let mut let_depth = 1_usize;
    let mut in_expr = Vec::new();
    let mut in_in = false;
    for token in tokens {
        if in_in {
            in_expr.push(token.clone());
            continue;
        }
        match token {
            Token::Open(_) => {
                delimiter_depth += 1;
                current.push(token.clone());
            }
            Token::Close(_) => {
                delimiter_depth = delimiter_depth.saturating_sub(1);
                current.push(token.clone());
            }
            Token::Let if delimiter_depth == 0 => {
                let_depth += 1;
                current.push(token.clone());
            }
            Token::In if delimiter_depth == 0 && let_depth == 1 => {
                if !current.is_empty() {
                    chunks.push(std::mem::take(&mut current));
                }
                in_in = true;
            }
            Token::In if delimiter_depth == 0 => {
                let_depth = let_depth.saturating_sub(1);
                current.push(token.clone());
            }
            Token::Comma if delimiter_depth == 0 && let_depth == 1 => {
                if !current.is_empty() {
                    chunks.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(token.clone()),
        }
    }

    let steps = chunks
        .into_iter()
        .filter_map(|chunk| {
            let equal = chunk.iter().position(|token| *token == Token::Equal)?;
            let name = chunk[..equal].iter().find_map(|token| match token {
                Token::Identifier(name) => Some(name.clone()),
                _ => None,
            })?;
            Some(Step {
                name,
                rhs: chunk[equal + 1..].to_vec(),
            })
        })
        .collect();
    (steps, in_expr)
}

fn expand_output_columns(tokens: &[Token]) -> Vec<String> {
    let mut columns = Vec::new();
    for args in table_function_arg_lists(tokens, "ExpandTableColumn") {
        let Some(names) = args
            .get(3)
            .or(args.get(2))
            .and_then(|arg| literal_string_list(arg))
        else {
            continue;
        };
        columns.extend(names);
    }
    columns
}

fn transform_column_type_names(tokens: &[Token]) -> Vec<String> {
    table_function_arg_lists(tokens, "TransformColumnTypes")
        .into_iter()
        .filter_map(|args| args.get(1).copied())
        .filter_map(spec_list_column_names)
        .flatten()
        .collect()
}

fn table_function_arg_lists<'a>(tokens: &'a [Token], function: &str) -> Vec<Vec<&'a [Token]>> {
    let mut calls = Vec::new();
    let mut index = 0;
    while index + 3 < tokens.len() {
        match (
            &tokens[index],
            &tokens[index + 1],
            &tokens[index + 2],
            &tokens[index + 3],
        ) {
            (Token::Identifier(table), Token::Dot, Token::Identifier(name), Token::Open('('))
                if table.eq_ignore_ascii_case("Table") && name.eq_ignore_ascii_case(function) =>
            {
                if let Some(close) = matching_close(tokens, index + 3) {
                    calls.push(split_top_level_args(&tokens[index + 4..close]));
                    index = close + 1;
                    continue;
                }
            }
            _ => {}
        }
        index += 1;
    }
    calls
}

fn matching_close(tokens: &[Token], open_index: usize) -> Option<usize> {
    let Token::Open(open) = tokens.get(open_index)? else {
        return None;
    };
    let close = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        _ => return None,
    };
    let mut depth = 0_usize;
    for (offset, token) in tokens[open_index..].iter().enumerate() {
        match token {
            Token::Open(character) if character == open => depth += 1,
            Token::Close(character) if *character == close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open_index + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_args(tokens: &[Token]) -> Vec<&[Token]> {
    let mut args = Vec::new();
    let mut start = 0_usize;
    let mut depth = 0_usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Open(_) => depth += 1,
            Token::Close(_) => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                args.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < tokens.len() || !args.is_empty() {
        args.push(&tokens[start..]);
    }
    args
}

fn wrapped_list_inner(tokens: &[Token]) -> Option<&[Token]> {
    if !matches!(tokens.first(), Some(Token::Open('{'))) {
        return None;
    }
    let close = matching_close(tokens, 0)?;
    if close + 1 != tokens.len() {
        return None;
    }
    Some(&tokens[1..close])
}

fn literal_string_list(tokens: &[Token]) -> Option<Vec<String>> {
    let inner = wrapped_list_inner(tokens)?;
    let mut names = Vec::new();
    for item in split_top_level_args(inner) {
        match item {
            [Token::String(name)] => names.push(name.clone()),
            [] => {}
            _ => return None,
        }
    }
    Some(names)
}

fn spec_list_column_names(tokens: &[Token]) -> Option<Vec<String>> {
    let inner = wrapped_list_inner(tokens)?;
    let mut names = Vec::new();
    for item in split_top_level_args(inner) {
        if let Some(pair_inner) = wrapped_list_inner(item) {
            if let Some([Token::String(name)]) = split_top_level_args(pair_inner).first() {
                names.push((*name).clone());
            }
        } else if let [Token::String(name)] = item {
            names.push(name.clone());
        }
    }
    Some(names)
}

fn classify_step_kind(tokens: &[Token]) -> StepKind {
    if tokens.is_empty() {
        return StepKind::Other;
    }
    if matches!(&tokens[0], Token::Identifier(name) if name == "each") {
        return StepKind::FunctionDefinition;
    }
    if matches!(&tokens[0], Token::Open('('))
        && let Some(close) = matching_close(tokens, 0)
    {
        // A lambda may carry a return-type ascription between the parameter
        // list and the arrow: `(value as any) as nullable text => ...`.
        let mut index = close + 1;
        if matches!(tokens.get(index), Some(Token::Identifier(name)) if name == "as") {
            index += 1;
            while matches!(tokens.get(index), Some(Token::Identifier(_))) {
                index += 1;
            }
        }
        if tokens.get(index) == Some(&Token::Arrow) {
            return StepKind::FunctionDefinition;
        }
    }
    if tokens.len() == 1 {
        match &tokens[0] {
            Token::Number | Token::String(_) => return StepKind::ScalarLiteral,
            Token::Identifier(name) if name == "true" || name == "false" => {
                return StepKind::ScalarLiteral;
            }
            _ => {}
        }
    }
    if matches!(&tokens[0], Token::Identifier(name) if name.eq_ignore_ascii_case("#table"))
        && tokens.get(1) == Some(&Token::Open('('))
    {
        return StepKind::TableLiteral;
    }
    if matches!(&tokens[0], Token::Open('[')) && matching_close(tokens, 0) == Some(tokens.len() - 1)
    {
        return StepKind::RecordLiteral;
    }
    if matches!(&tokens[0], Token::Open('{')) && matching_close(tokens, 0) == Some(tokens.len() - 1)
    {
        return StepKind::ListLiteral;
    }
    if is_navigation(tokens) {
        return StepKind::Navigation;
    }
    StepKind::Other
}

fn is_navigation(tokens: &[Token]) -> bool {
    let Some(item_open) = tokens.windows(2).position(|window| {
        matches!(
            (&window[0], &window[1]),
            (Token::Open('{'), Token::Open('['))
        )
    }) else {
        return false;
    };
    if item_open == 0 {
        return false;
    }
    let Some(item_close) = matching_close(tokens, item_open) else {
        return false;
    };
    let rest = &tokens[item_close + 1..];
    if rest.len() < 3
        || !matches!(rest[0], Token::Open('['))
        || !matches!(rest[1], Token::Identifier(_))
        || !matches!(rest[2], Token::Close(']'))
    {
        return false;
    }
    let mut index = 3;
    while index < rest.len() {
        if index + 2 < rest.len()
            && matches!(rest[index], Token::Open('['))
            && matches!(rest[index + 1], Token::Identifier(_))
            && matches!(rest[index + 2], Token::Close(']'))
        {
            index += 3;
        } else {
            return false;
        }
    }
    true
}

fn starts_with_table_buffer(tokens: &[Token]) -> bool {
    table_buffer_calls(tokens).any(|(index, _)| index == 0)
}

fn table_buffer_wraps(tokens: &[Token], step: &str) -> bool {
    table_buffer_calls(tokens).any(|(_, argument)| argument == Some(step))
}

fn table_buffer_calls(tokens: &[Token]) -> impl Iterator<Item = (usize, Option<&str>)> {
    tokens.windows(5).enumerate().filter_map(|(index, window)| {
        let [
            Token::Identifier(table),
            Token::Dot,
            Token::Identifier(buffer),
            Token::Open('('),
            argument,
        ] = window
        else {
            return None;
        };
        if table.eq_ignore_ascii_case("Table") && buffer.eq_ignore_ascii_case("Buffer") {
            Some((
                index,
                match argument {
                    Token::Identifier(name) => Some(name.as_str()),
                    _ => None,
                },
            ))
        } else {
            None
        }
    })
}

fn lex_m(source: &str) -> Vec<Token> {
    let chars = source.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character.is_whitespace() {
            index += 1;
            continue;
        }
        if character == '/' && chars.get(index + 1) == Some(&'/') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if character == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;
            while index + 1 < chars.len() && !(chars[index] == '*' && chars[index + 1] == '/') {
                index += 1;
            }
            index = (index + 2).min(chars.len());
            continue;
        }
        if character == '"' {
            let (value, next) = quoted_identifier(&chars, index + 1);
            tokens.push(Token::String(value));
            index = next;
            continue;
        }
        if character == '#' && chars.get(index + 1) == Some(&'"') {
            let (name, next) = quoted_identifier(&chars, index + 2);
            tokens.push(Token::Identifier(name));
            index = next;
            continue;
        }
        if character == '#' {
            let start = index + 1;
            if start < chars.len() && (chars[start].is_alphabetic() || chars[start] == '_') {
                index = start + 1;
                while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_')
                {
                    index += 1;
                }
                let word = chars[start..index].iter().collect::<String>();
                tokens.push(Token::Identifier(format!("#{word}")));
                continue;
            }
        }
        if character.is_ascii_digit() {
            index += 1;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
            if index < chars.len()
                && chars[index] == '.'
                && chars
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_digit())
            {
                index += 1;
                while index < chars.len() && chars[index].is_ascii_digit() {
                    index += 1;
                }
            }
            if index < chars.len() && matches!(chars[index], 'L' | 'D' | 'M' | 'l' | 'd' | 'm') {
                index += 1;
            }
            tokens.push(Token::Number);
            continue;
        }
        if character.is_alphabetic() || character == '_' {
            let start = index;
            index += 1;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            let word = chars[start..index].iter().collect::<String>();
            tokens.push(match word.as_str() {
                "let" => Token::Let,
                "in" => Token::In,
                _ => Token::Identifier(word),
            });
            continue;
        }
        match character {
            '=' if chars.get(index + 1) == Some(&'>') => {
                tokens.push(Token::Arrow);
                index += 2;
                continue;
            }
            '=' => tokens.push(Token::Equal),
            ',' => tokens.push(Token::Comma),
            '(' | '[' | '{' => tokens.push(Token::Open(character)),
            ')' | ']' | '}' => tokens.push(Token::Close(character)),
            '.' => tokens.push(Token::Dot),
            _ => {}
        }
        index += 1;
    }
    tokens
}

fn quoted_identifier(chars: &[char], mut index: usize) -> (String, usize) {
    let mut name = String::new();
    while index < chars.len() {
        if chars[index] == '"' {
            if chars.get(index + 1) == Some(&'"') {
                name.push('"');
                index += 2;
            } else {
                return (name, index + 1);
            }
        } else {
            name.push(chars[index]);
            index += 1;
        }
    }
    (name, index)
}

#[cfg(test)]
mod tests {
    use super::{Reuse, StepKind, UntypedExpansion, analyze_let_steps, analyze_untyped_expansions};
    use std::collections::BTreeSet;

    #[test]
    fn flags_unbuffered_later_step_reuse() {
        let source = r#"
let
    Source = Some.Source(),
    #"Changed Type" = Transform(Source),
    Left = Use(#"Changed Type"),
    Right = Other(#"Changed Type")
in
    Left
"#;
        assert_eq!(
            analyze_let_steps(source),
            vec![Reuse {
                step: "Changed Type".to_string(),
                reference_count: 2,
                step_kind: StepKind::Other,
            }]
        );
    }

    #[test]
    fn ignores_reused_table_buffer_output() {
        let source = r#"
let
    Source = Some.Source(),
    Buffered = Table.Buffer(Source),
    Left = Use(Buffered),
    Right = Other(Buffered)
in
    Left
"#;
        assert!(analyze_let_steps(source).is_empty());
    }

    fn numeric(columns: &[&str]) -> BTreeSet<String> {
        columns.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn flags_expanded_numeric_column_without_retype() {
        let source = r#"
let
    Source = #table(type table [DateKey = Int64.Type, Revenue = Currency.Type], {}),
    Grouped = Table.Group(Source, {"DateKey"}, {{"Rows", each _, type table}}),
    Expanded = Table.ExpandTableColumn(Grouped, "Rows", {"Revenue"})
in
    Expanded
"#;
        assert_eq!(
            analyze_untyped_expansions(source, &numeric(&["Revenue"])),
            vec![UntypedExpansion {
                step: "Expanded".to_string(),
                column: "Revenue".to_string(),
                step_kind: StepKind::Other,
            }]
        );
    }

    #[test]
    fn uses_expand_new_name_list_when_present() {
        let source = r#"
let
    Source = Table.FromRows({}),
    Expanded = Table.ExpandTableColumn(Source, "Rows", {"Revenue"}, {"Rank"})
in
    Expanded
"#;
        assert_eq!(
            analyze_untyped_expansions(source, &numeric(&["Rank"])),
            vec![UntypedExpansion {
                step: "Expanded".to_string(),
                column: "Rank".to_string(),
                step_kind: StepKind::Other,
            }]
        );
    }

    #[test]
    fn ignores_expanded_column_after_transform_column_types() {
        let source = r#"
let
    Source = Table.FromRows({}),
    Expanded = Table.ExpandTableColumn(Source, "Rows", {"Revenue"}),
    Typed = Table.TransformColumnTypes(Expanded, {{"Revenue", type number}})
in
    Typed
"#;
        assert!(analyze_untyped_expansions(source, &numeric(&["Revenue"])).is_empty());
    }

    #[test]
    fn ignores_expanded_column_outside_numeric_source_set() {
        let source = r#"
let
    Source = Table.FromRows({}),
    Expanded = Table.ExpandTableColumn(Source, "Rows", {"Segment"})
in
    Expanded
"#;
        assert!(analyze_untyped_expansions(source, &numeric(&["Revenue"])).is_empty());
    }

    #[test]
    fn ignores_reused_function_and_scalar_steps() {
        let source = r#"
let
    Normalize = (value) => value,
    Scale = 1.5,
    Label = "ready",
    Flag = true,
    LeftFn = Normalize,
    RightFn = Normalize,
    LeftScale = Scale,
    RightScale = Scale,
    LeftLabel = Label,
    RightLabel = Label,
    LeftFlag = Flag,
    RightFlag = Flag
in
    LeftFn
"#;
        assert!(analyze_let_steps(source).is_empty());
    }

    #[test]
    fn flags_reused_table_literal_and_navigation_steps() {
        let source = r#"
let
    Shared = #table(type table [Value = Int64.Type], {}),
    Nav = Database{[Schema="dbo",Item="Sales"]}[Data],
    LeftTable = Table.FirstN(Shared, 1),
    RightTable = Table.LastN(Shared, 1),
    LeftNav = Table.FirstN(Nav, 1),
    RightNav = Table.LastN(Nav, 1)
in
    LeftTable
"#;
        assert_eq!(
            analyze_let_steps(source),
            vec![
                Reuse {
                    step: "Shared".to_string(),
                    reference_count: 2,
                    step_kind: StepKind::TableLiteral,
                },
                Reuse {
                    step: "Nav".to_string(),
                    reference_count: 2,
                    step_kind: StepKind::Navigation,
                }
            ]
        );
    }

    #[test]
    fn skips_expand_when_name_list_is_not_literal() {
        let source = r#"
let
    Source = Table.FromRows({}),
    Names = {"Revenue"},
    Expanded = Table.ExpandTableColumn(Source, "Rows", Names)
in
    Expanded
"#;
        assert!(analyze_untyped_expansions(source, &numeric(&["Revenue"])).is_empty());
    }
}
