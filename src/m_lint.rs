use crate::tmdl::load_table_documents;
use crate::{CliError, CliResult, ResolvedProject, canonical_display};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn buffer_reuse_findings(resolved: &ResolvedProject) -> CliResult<Vec<Value>> {
    let mut findings = Vec::new();
    let docs = match load_table_documents(resolved) {
        Ok(docs) => docs,
        Err(error) if error.code == "file_not_found" => Vec::new(),
        Err(error) => return Err(error),
    };
    for partition in docs.iter().flat_map(|doc| doc.partitions.iter()) {
        let Some(source) = partition.source.as_deref() else {
            continue;
        };
        findings.extend(document_findings(
            source,
            "partition",
            &partition.handle(),
            &partition.path,
        ));
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
                "code": "m.unbuffered_reuse",
                "severity": "warning",
                "message": format!(
                    "M step `{}` is referenced {} times by later steps without Table.Buffer",
                    reuse.step, reuse.reference_count
                ),
                "handle": handle,
                "path": canonical_display(path),
                "documentKind": document_kind,
                "step": reuse.step,
                "referenceCount": reuse.reference_count,
                "analysisBoundary": "heuristic"
            })
        })
        .collect()
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
    Let,
    In,
    Equal,
    Comma,
    Open(char),
    Close(char),
    Dot,
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
}

fn analyze_let_steps(source: &str) -> Vec<Reuse> {
    let tokens = lex_m(source);
    let Some(let_index) = tokens.iter().position(|token| *token == Token::Let) else {
        return Vec::new();
    };
    let steps = parse_steps(&tokens[let_index + 1..]);
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
        let buffered_definition = starts_with_table_buffer(&step.rhs);
        let buffered_later = later
            .iter()
            .any(|candidate| table_buffer_wraps(&candidate.rhs, &step.name));
        if !buffered_definition && !buffered_later {
            reuse.push(Reuse {
                step: step.name.clone(),
                reference_count,
            });
        }
    }
    reuse
}

fn parse_steps(tokens: &[Token]) -> Vec<Step> {
    let mut chunks = Vec::<Vec<Token>>::new();
    let mut current = Vec::new();
    let mut delimiter_depth = 0_usize;
    let mut let_depth = 1_usize;
    for token in tokens {
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
                    chunks.push(current);
                }
                break;
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

    chunks
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
        .collect()
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
            index = skip_quoted_text(&chars, index + 1);
            continue;
        }
        if character == '#' && chars.get(index + 1) == Some(&'"') {
            let (name, next) = quoted_identifier(&chars, index + 2);
            tokens.push(Token::Identifier(name));
            index = next;
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

fn skip_quoted_text(chars: &[char], mut index: usize) -> usize {
    while index < chars.len() {
        if chars[index] == '"' {
            if chars.get(index + 1) == Some(&'"') {
                index += 2;
            } else {
                return index + 1;
            }
        } else {
            index += 1;
        }
    }
    index
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
    use super::{Reuse, analyze_let_steps};

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
                reference_count: 2
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
}
