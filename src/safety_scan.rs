use serde_json::Value;

pub(crate) const CREDENTIAL_NEEDLES: &[&str] = &[
    "password",
    "pwd",
    "pass",
    "credential",
    "secret",
    "token",
    "apikey",
    "api key",
    "api_key",
    "x-api-key",
    "accesskey",
    "access key",
    "access_key",
    "accountkey",
    "account key",
    "sas token",
    "sharedaccesssignature",
    "shared access signature",
    "sig",
    "user",
    "username",
    "user id",
    "userid",
    "uid",
    "authorization",
    "ghp_",
    "github_pat_",
    "akia",
];

pub(crate) const STYLE_CREDENTIAL_NEEDLES: &[&str] = &[
    "password",
    "token",
    "secret",
    "credential",
    "apikey",
    "api key",
];

const ASSIGNMENT_KEYS: &[&str] = &[
    "password",
    "pwd",
    "pass",
    "credential",
    "secret",
    "token",
    "apikey",
    "xapikey",
    "accesskey",
    "accountkey",
    "sharedaccesssignature",
    "sastoken",
    "sig",
    "user",
    "username",
    "userid",
    "uid",
];

const LONG_FREE_TEXT_CHARS: usize = 80;

type ParsedMTable = (Vec<String>, Vec<Vec<Option<String>>>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MTableSafety {
    pub(crate) valid_generator_shape: bool,
    pub(crate) pii_suspect: bool,
}

#[derive(Debug, Clone, Copy)]
struct CredentialAssignment {
    redact_start: usize,
    redact_end: usize,
}

pub(crate) fn contains_credential_like_text_str(text: &str) -> bool {
    !credential_assignments(text).is_empty() || !recognizable_token_ranges(text).is_empty()
}

pub(crate) fn redact_credential_values(text: &str) -> String {
    let mut ranges = credential_assignments(text)
        .into_iter()
        .filter_map(|assignment| {
            (assignment.redact_start < assignment.redact_end)
                .then_some((assignment.redact_start, assignment.redact_end))
        })
        .chain(recognizable_token_ranges(text))
        .collect::<Vec<_>>();
    if ranges.is_empty() {
        return text.to_string();
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some((_, previous_end)) = merged.last_mut()
            && start <= *previous_end
        {
            *previous_end = (*previous_end).max(end);
        } else {
            merged.push((start, end));
        }
    }

    let mut redacted = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in merged {
        redacted.push_str(&text[cursor..start]);
        redacted.push_str("***");
        cursor = end;
    }
    redacted.push_str(&text[cursor..]);
    redacted
}

pub(crate) fn redact_credential_parameter(key: &str, value: &str) -> String {
    if is_credential_key(key) {
        "***".to_string()
    } else {
        redact_credential_values(value)
    }
}

pub(crate) fn contains_pii_suspect_text(text: &str) -> bool {
    if contains_email_like(text) {
        return true;
    }
    serde_json::from_str::<Value>(text)
        .ok()
        .is_some_and(|value| json_rows_contain_pii(&value, false, None))
}

pub(crate) fn generated_m_table_safety(source: &str, expected_columns: &[String]) -> MTableSafety {
    let Some((columns, rows)) = parse_generated_m_table(source) else {
        return MTableSafety {
            valid_generator_shape: false,
            pii_suspect: false,
        };
    };
    if columns != expected_columns {
        return MTableSafety {
            valid_generator_shape: false,
            pii_suspect: false,
        };
    }
    let pii_suspect = rows.iter().any(|row| {
        row.iter().enumerate().any(|(index, literal)| {
            literal.as_deref().is_some_and(|value| {
                pii_suspect_literal(columns.get(index).map(String::as_str), value)
            })
        })
    });
    MTableSafety {
        valid_generator_shape: true,
        pii_suspect,
    }
}

fn credential_assignments(text: &str) -> Vec<CredentialAssignment> {
    let lowered = text.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    let mut assignments = Vec::new();
    for start in 0..bytes.len() {
        if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
            continue;
        }
        for key in ASSIGNMENT_KEYS {
            if let Some(after_key) = match_canonical_key(bytes, start, key)
                && let Some(after_delimiter) = assignment_delimiter(bytes, after_key)
            {
                assignments.push(value_assignment(text, &lowered, after_delimiter, false));
                break;
            }
        }
        if let Some(after_key) = match_canonical_key(bytes, start, "authorization")
            && let Some(after_delimiter) = assignment_delimiter(bytes, after_key)
            && let Some(assignment) = bearer_assignment(text, &lowered, after_delimiter)
        {
            assignments.push(assignment);
        }
    }
    assignments
}

fn match_canonical_key(bytes: &[u8], start: usize, canonical: &str) -> Option<usize> {
    let mut index = start;
    for (position, expected) in canonical.bytes().enumerate() {
        if position > 0 {
            while index < bytes.len() && matches!(bytes[index], b' ' | b'\t' | b'_' | b'-') {
                index += 1;
            }
        }
        if bytes.get(index).copied() != Some(expected) {
            return None;
        }
        index += 1;
    }
    Some(index)
}

fn assignment_delimiter(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    if matches!(bytes.get(index), Some(b'\'' | b'"')) {
        index += 1;
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
    }
    if !matches!(bytes.get(index), Some(b'=' | b':')) {
        return None;
    }
    Some(index + 1)
}

fn value_assignment(
    text: &str,
    lowered: &str,
    after_delimiter: usize,
    bearer_only: bool,
) -> CredentialAssignment {
    let bytes = text.as_bytes();
    let lower_bytes = lowered.as_bytes();
    let mut index = after_delimiter;
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    let quote = matches!(bytes.get(index), Some(b'\'' | b'"')).then(|| bytes[index]);
    if quote.is_some() {
        index += 1;
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
    }
    if bearer_only {
        let bearer = b"bearer";
        if lower_bytes.get(index..index + bearer.len()) != Some(bearer) {
            return CredentialAssignment {
                redact_start: index,
                redact_end: index,
            };
        }
        index += bearer.len();
        if lower_bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        {
            return CredentialAssignment {
                redact_start: index,
                redact_end: index,
            };
        }
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
    }
    let start = index;
    let end = if let Some(quote) = quote {
        quoted_value_end(bytes, start, quote)
    } else {
        unquoted_value_end(bytes, start)
    };
    CredentialAssignment {
        redact_start: start,
        redact_end: end,
    }
}

fn bearer_assignment(
    text: &str,
    lowered: &str,
    after_delimiter: usize,
) -> Option<CredentialAssignment> {
    let assignment = value_assignment(text, lowered, after_delimiter, true);
    (assignment.redact_start < assignment.redact_end).then_some(assignment)
}

fn quoted_value_end(bytes: &[u8], mut index: usize, quote: u8) -> usize {
    while index < bytes.len() {
        if bytes[index] == quote {
            if bytes.get(index + 1) == Some(&quote) {
                index += 2;
                continue;
            }
            return index;
        }
        index += 1;
    }
    index
}

fn unquoted_value_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len()
        && !matches!(
            bytes[index],
            b';' | b',' | b'\r' | b'\n' | b']' | b'}' | b')' | b'\'' | b'"' | b'&'
        )
    {
        index += 1;
    }
    index
}

fn recognizable_token_ranges(text: &str) -> Vec<(usize, usize)> {
    let lowered = text.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    let mut ranges = Vec::new();
    for index in 0..bytes.len() {
        if index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_') {
            continue;
        }
        for (prefix, minimum_suffix) in [("ghp_", 20usize), ("github_pat_", 10usize)] {
            let prefix_bytes = prefix.as_bytes();
            if bytes.get(index..index + prefix_bytes.len()) == Some(prefix_bytes) {
                let mut end = index + prefix_bytes.len();
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                {
                    end += 1;
                }
                if end - index - prefix_bytes.len() >= minimum_suffix {
                    ranges.push((index, end));
                }
            }
        }
        if bytes.get(index..index + 4) == Some(b"akia") {
            let mut end = index + 4;
            while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
                end += 1;
            }
            if end - index >= 20 {
                ranges.push((index, end));
            }
        }
    }
    ranges
}

fn is_credential_key(key: &str) -> bool {
    let canonical = key
        .bytes()
        .filter(|byte| !matches!(byte, b' ' | b'\t' | b'_' | b'-'))
        .map(|byte| byte.to_ascii_lowercase())
        .collect::<Vec<_>>();
    ASSIGNMENT_KEYS
        .iter()
        .chain([&"authorization"])
        .any(|expected| canonical == expected.as_bytes())
}

fn parse_generated_m_table(source: &str) -> Option<ParsedMTable> {
    let trimmed = source.trim();
    let lowered = trimmed.to_ascii_lowercase();
    let table_positions = lowered
        .match_indices("#table")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let [table_start] = table_positions.as_slice() else {
        return None;
    };
    let prefix = trimmed[..*table_start]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !prefix.eq_ignore_ascii_case("let Source =") {
        return None;
    }
    let mut open = *table_start + "#table".len();
    let bytes = trimmed.as_bytes();
    while open < bytes.len() && bytes[open].is_ascii_whitespace() {
        open += 1;
    }
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let close = matching_delimiter(trimmed, open, b'(', b')')?;
    let suffix = trimmed[close + 1..]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !suffix.eq_ignore_ascii_case("in Source") {
        return None;
    }
    let arguments = split_top_level(&trimmed[open + 1..close], b',')?;
    let [type_expression, row_expression] = arguments.as_slice() else {
        return None;
    };
    let columns = parse_m_table_columns(type_expression)?;
    let rows = parse_m_table_rows(row_expression, columns.len())?;
    Some((columns, rows))
}

fn parse_m_table_columns(expression: &str) -> Option<Vec<String>> {
    let trimmed = expression.trim();
    let lowered = trimmed.to_ascii_lowercase();
    let rest = lowered
        .strip_prefix("type table")
        .map(|_| &trimmed["type table".len()..])?
        .trim();
    if rest.as_bytes().first() != Some(&b'[') {
        return None;
    }
    let close = matching_delimiter(rest, 0, b'[', b']')?;
    if !rest[close + 1..].trim().is_empty() {
        return None;
    }
    let definitions = split_top_level(&rest[1..close], b',')?;
    if definitions.is_empty() {
        return None;
    }
    definitions
        .into_iter()
        .map(|definition| {
            let (name, data_type) = split_top_level_once(definition, b'=')?;
            if !matches!(
                data_type.trim().to_ascii_lowercase().as_str(),
                "text" | "number" | "date" | "datetime" | "logical"
            ) {
                return None;
            }
            parse_m_identifier(name)
        })
        .collect()
}

fn parse_m_table_rows(expression: &str, column_count: usize) -> Option<Vec<Vec<Option<String>>>> {
    let trimmed = expression.trim();
    if trimmed.as_bytes().first() != Some(&b'{') {
        return None;
    }
    let close = matching_delimiter(trimmed, 0, b'{', b'}')?;
    if !trimmed[close + 1..].trim().is_empty() {
        return None;
    }
    let content = trimmed[1..close].trim();
    if content.is_empty() {
        return Some(Vec::new());
    }
    split_top_level(content, b',')?
        .into_iter()
        .map(|row| {
            let row = row.trim();
            if row.as_bytes().first() != Some(&b'{') {
                return None;
            }
            let row_close = matching_delimiter(row, 0, b'{', b'}')?;
            if !row[row_close + 1..].trim().is_empty() {
                return None;
            }
            let values = split_top_level(&row[1..row_close], b',')?;
            if values.len() != column_count {
                return None;
            }
            values.into_iter().map(parse_m_literal).collect()
        })
        .collect()
}

fn parse_m_identifier(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if let Some(quoted) = trimmed
        .strip_prefix("#\"")
        .and_then(|rest| rest.strip_suffix('"'))
        && quoted_m_string_is_complete(quoted)
    {
        return Some(quoted.replace("\"\"", "\""));
    }
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    ((first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_'))
    .then(|| trimmed.to_string())
}

fn parse_m_literal(value: &str) -> Option<Option<String>> {
    let trimmed = value.trim();
    if let Some(inner) = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        && quoted_m_string_is_complete(inner)
    {
        return Some(Some(inner.replace("\"\"", "\"")));
    }
    let lowered = trimmed.to_ascii_lowercase();
    if matches!(lowered.as_str(), "null" | "true" | "false")
        || trimmed.parse::<f64>().is_ok()
        || valid_date_literal(&lowered, "#date", 3)
        || valid_date_literal(&lowered, "#datetime", 6)
    {
        return Some(None);
    }
    None
}

fn quoted_m_string_is_complete(inner: &str) -> bool {
    let bytes = inner.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            if bytes.get(index + 1) != Some(&b'"') {
                return false;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    true
}

fn valid_date_literal(value: &str, prefix: &str, parts: usize) -> bool {
    let Some(arguments) = value
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('('))
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return false;
    };
    let values = arguments.split(',').map(str::trim).collect::<Vec<_>>();
    values.len() == parts
        && values
            .iter()
            .all(|part| !part.is_empty() && part.parse::<i64>().is_ok())
}

fn matching_delimiter(text: &str, open: usize, opening: u8, closing: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&opening) {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut index = open;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            if in_string && bytes.get(index + 1) == Some(&b'"') {
                index += 2;
                continue;
            }
            in_string = !in_string;
            index += 1;
            continue;
        }
        if !in_string {
            if bytes[index] == opening {
                depth += 1;
            } else if bytes[index] == closing {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
        }
        index += 1;
    }
    None
}

fn split_top_level(text: &str, separator: u8) -> Option<Vec<&str>> {
    if text.trim().is_empty() {
        return Some(Vec::new());
    }
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut parens = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    let mut in_string = false;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            if in_string && bytes.get(index + 1) == Some(&b'"') {
                index += 2;
                continue;
            }
            in_string = !in_string;
            index += 1;
            continue;
        }
        if !in_string {
            match bytes[index] {
                b'(' => parens += 1,
                b')' => parens = parens.checked_sub(1)?,
                b'[' => brackets += 1,
                b']' => brackets = brackets.checked_sub(1)?,
                b'{' => braces += 1,
                b'}' => braces = braces.checked_sub(1)?,
                byte if byte == separator && parens == 0 && brackets == 0 && braces == 0 => {
                    parts.push(text[start..index].trim());
                    start = index + 1;
                }
                _ => {}
            }
        }
        index += 1;
    }
    if in_string || parens != 0 || brackets != 0 || braces != 0 {
        return None;
    }
    parts.push(text[start..].trim());
    (!parts.iter().any(|part| part.is_empty())).then_some(parts)
}

fn split_top_level_once(text: &str, separator: u8) -> Option<(&str, &str)> {
    let parts = split_top_level(text, separator)?;
    let [left, right] = parts.as_slice() else {
        return None;
    };
    Some((left, right))
}

fn json_rows_contain_pii(value: &Value, under_rows: bool, key: Option<&str>) -> bool {
    match value {
        Value::String(text) => under_rows && pii_suspect_literal(key, text),
        Value::Array(values) => values
            .iter()
            .any(|value| json_rows_contain_pii(value, under_rows, key)),
        Value::Object(values) => values.iter().any(|(child_key, value)| {
            json_rows_contain_pii(
                value,
                under_rows || child_key.eq_ignore_ascii_case("rows"),
                Some(child_key),
            )
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn pii_suspect_literal(column: Option<&str>, value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || looks_explicitly_synthetic(trimmed) {
        return false;
    }
    if contains_email_like(trimmed)
        || (trimmed.chars().count() >= LONG_FREE_TEXT_CHARS
            && trimmed.split_whitespace().count() >= 8)
    {
        return true;
    }
    let column = column
        .unwrap_or_default()
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        column.as_str(),
        "firstname"
            | "lastname"
            | "fullname"
            | "personname"
            | "employeename"
            | "customername"
            | "contactname"
            | "patientname"
    ) || (column == "name" && looks_like_person_name(trimmed))
        || (matches!(
            column.as_str(),
            "email"
                | "emailaddress"
                | "phone"
                | "phonenumber"
                | "mobile"
                | "ssn"
                | "socialsecuritynumber"
        ) && !trimmed.contains('<'))
}

fn looks_explicitly_synthetic(value: &str) -> bool {
    value
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .any(|part| {
            matches!(
                part.to_ascii_lowercase().as_str(),
                "sample" | "synthetic" | "example" | "demo" | "test" | "fake" | "anonymous"
            )
        })
}

fn looks_like_person_name(value: &str) -> bool {
    let words = value.split_whitespace().collect::<Vec<_>>();
    (2..=4).contains(&words.len())
        && words.iter().all(|word| {
            let mut chars = word.chars().filter(|ch| !matches!(ch, '-' | '\''));
            chars.next().is_some_and(char::is_uppercase) && chars.all(|ch| ch.is_lowercase())
        })
}

fn contains_email_like(value: &str) -> bool {
    value
        .split(|ch: char| {
            ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '\'' | '(' | ')' | ',' | ';')
        })
        .any(|token| {
            let Some((local, domain)) = token.split_once('@') else {
                return false;
            };
            !local.is_empty()
                && domain
                    .split_once('.')
                    .is_some_and(|(host, suffix)| !host.is_empty() && suffix.len() >= 2)
        })
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SafetyScan {
    pub(crate) contains_selectors: bool,
    pub(crate) contains_data_selectors: bool,
    pub(crate) contains_literal_text: bool,
    pub(crate) contains_colors: bool,
    pub(crate) contains_external_uris: bool,
    pub(crate) contains_credential_like_text: bool,
    pub(crate) contains_conditional_formatting_signals: bool,
    pub(crate) may_contain_data_values: bool,
    pub(crate) literal_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringNormalization {
    Raw,
    FormattingLiteral,
}

#[derive(Debug, Clone, Copy)]
struct ScanOptions {
    normalization: StringNormalization,
    credential_needles: &'static [&'static str],
    credentials_in_object_keys: bool,
    scan_external_uris: bool,
    scan_formatting_literals: bool,
    scan_conditional_formatting: bool,
    scan_selectors: bool,
    sensitive_keys: &'static [&'static str],
    count_all_literals: bool,
}

pub(crate) fn formatting_safety<'a>(
    values: impl IntoIterator<Item = &'a Value>,
    credential_needles: &'static [&'static str],
    scan_conditional_formatting: bool,
) -> SafetyScan {
    scan_values(
        values,
        ScanOptions {
            normalization: StringNormalization::FormattingLiteral,
            credential_needles,
            credentials_in_object_keys: false,
            scan_external_uris: true,
            scan_formatting_literals: true,
            scan_conditional_formatting,
            scan_selectors: true,
            sensitive_keys: &[],
            count_all_literals: false,
        },
    )
}

pub(crate) fn data_value_safety(
    value: &Value,
    sensitive_keys: &'static [&'static str],
) -> SafetyScan {
    scan_values(
        [value],
        ScanOptions {
            normalization: StringNormalization::Raw,
            credential_needles: &[],
            credentials_in_object_keys: false,
            scan_external_uris: false,
            scan_formatting_literals: false,
            scan_conditional_formatting: false,
            scan_selectors: false,
            sensitive_keys,
            count_all_literals: false,
        },
    )
}

pub(crate) fn contains_external_uri(value: &Value) -> bool {
    scan_values(
        [value],
        ScanOptions {
            normalization: StringNormalization::Raw,
            credential_needles: &[],
            credentials_in_object_keys: false,
            scan_external_uris: true,
            scan_formatting_literals: false,
            scan_conditional_formatting: false,
            scan_selectors: false,
            sensitive_keys: &[],
            count_all_literals: false,
        },
    )
    .contains_external_uris
}

pub(crate) fn contains_credential_like_text(
    value: &Value,
    credential_needles: &'static [&'static str],
    include_object_keys: bool,
) -> bool {
    scan_values(
        [value],
        ScanOptions {
            normalization: StringNormalization::Raw,
            credential_needles,
            credentials_in_object_keys: include_object_keys,
            scan_external_uris: false,
            scan_formatting_literals: false,
            scan_conditional_formatting: false,
            scan_selectors: false,
            sensitive_keys: &[],
            count_all_literals: false,
        },
    )
    .contains_credential_like_text
}

pub(crate) fn count_literals(value: &Value) -> usize {
    scan_values(
        [value],
        ScanOptions {
            normalization: StringNormalization::Raw,
            credential_needles: &[],
            credentials_in_object_keys: false,
            scan_external_uris: false,
            scan_formatting_literals: false,
            scan_conditional_formatting: false,
            scan_selectors: false,
            sensitive_keys: &[],
            count_all_literals: true,
        },
    )
    .literal_count
}

fn scan_values<'a>(
    values: impl IntoIterator<Item = &'a Value>,
    options: ScanOptions,
) -> SafetyScan {
    let mut scan = SafetyScan::default();
    for value in values {
        scan_value(value, &mut scan, options, false, false);
    }
    scan
}

fn scan_value(
    value: &Value,
    scan: &mut SafetyScan,
    options: ScanOptions,
    under_selector: bool,
    under_sensitive_key: bool,
) {
    match value {
        Value::Null => {}
        Value::Bool(_) | Value::Number(_) => {
            count_literal(scan, options, under_sensitive_key);
        }
        Value::String(text) => {
            count_literal(scan, options, under_sensitive_key);
            scan_string(text, scan, options, under_selector);
        }
        Value::Array(items) => {
            for item in items {
                scan_value(item, scan, options, under_selector, under_sensitive_key);
            }
        }
        Value::Object(object) => {
            if options.scan_selectors
                && let Some(selector) = object.get("selector")
            {
                scan.contains_selectors = true;
                if selector_has_specific_data(selector) {
                    scan.contains_data_selectors = true;
                }
            }
            for (key, value) in object {
                if options.credentials_in_object_keys {
                    if options.credential_needles == CREDENTIAL_NEEDLES && is_credential_key(key) {
                        scan.contains_credential_like_text = true;
                    } else {
                        scan_credentials(key, scan, options);
                    }
                }
                if options.scan_conditional_formatting && is_conditional_formatting_key(key) {
                    scan.contains_conditional_formatting_signals = true;
                }
                let sensitive_key = options
                    .sensitive_keys
                    .iter()
                    .any(|expected| key.eq_ignore_ascii_case(expected));
                if sensitive_key {
                    scan.may_contain_data_values = true;
                }
                scan_value(
                    value,
                    scan,
                    options,
                    under_selector || (options.scan_selectors && key == "selector"),
                    under_sensitive_key || sensitive_key,
                );
            }
        }
    }
}

fn count_literal(scan: &mut SafetyScan, options: ScanOptions, under_sensitive_key: bool) {
    if options.count_all_literals || under_sensitive_key {
        scan.literal_count += 1;
    }
}

fn scan_string(text: &str, scan: &mut SafetyScan, options: ScanOptions, under_selector: bool) {
    let normalized = normalize_string(text, options.normalization);
    let lowered = normalized.to_ascii_lowercase();
    if options.scan_external_uris
        && (lowered.starts_with("http://") || lowered.starts_with("https://"))
    {
        scan.contains_external_uris = true;
    }
    scan_credentials(&normalized, scan, options);
    if options.scan_conditional_formatting
        && (lowered.contains("conditional")
            || lowered.contains("gradient")
            || lowered.contains("rules"))
    {
        scan.contains_conditional_formatting_signals = true;
    }
    if options.scan_formatting_literals {
        if is_color_literal(&normalized) {
            scan.contains_colors = true;
        } else if !under_selector && !is_non_text_literal(&normalized) && !normalized.is_empty() {
            scan.contains_literal_text = true;
        }
    }
}

fn scan_credentials(text: &str, scan: &mut SafetyScan, options: ScanOptions) {
    if !scan.contains_credential_like_text {
        scan.contains_credential_like_text = if options.credential_needles == CREDENTIAL_NEEDLES {
            contains_credential_like_text_str(text)
        } else {
            let lowered = text.to_ascii_lowercase();
            options
                .credential_needles
                .iter()
                .any(|needle| lowered.contains(needle))
        };
    }
}

fn normalize_string(text: &str, normalization: StringNormalization) -> String {
    match normalization {
        StringNormalization::Raw => text.to_string(),
        StringNormalization::FormattingLiteral => text
            .trim()
            .trim_matches('\'')
            .trim_matches('"')
            .trim()
            .to_string(),
    }
}

fn is_color_literal(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_non_text_literal(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    lowered == "true" || lowered == "false" || lowered == "null" || value.parse::<f64>().is_ok()
}

fn selector_has_specific_data(selector: &Value) -> bool {
    if let Some(data) = selector.get("data").and_then(Value::as_array) {
        return data.iter().any(|item| {
            !item
                .as_object()
                .is_some_and(|object| object.contains_key("dataViewWildcard"))
        });
    }
    selector.get("metadata").is_some()
        || selector.get("id").is_some()
        || selector.get("expr").is_some()
}

fn is_conditional_formatting_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "rule" | "rules" | "gradient" | "conditionalformatting" | "datareductionalgorithm"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formatting_profile_preserves_literal_and_selector_rules() {
        let value = json!({
            "title": " 'Executive Summary' ",
            "color": "#12abEF",
            "selector": {"data": [{"dataViewWildcard": {}}]},
            "nested": " https://example.test/report ",
            "secret": "api_key=redacted"
        });

        let scan = formatting_safety([&value], CREDENTIAL_NEEDLES, false);
        assert!(scan.contains_selectors);
        assert!(!scan.contains_data_selectors);
        assert!(scan.contains_literal_text);
        assert!(scan.contains_colors);
        assert!(scan.contains_external_uris);
        assert!(scan.contains_credential_like_text);
    }

    #[test]
    fn data_value_profile_counts_only_literals_below_sensitive_keys() {
        let value = json!({
            "displayName": "Bookmark",
            "state": {
                "value": [1, true, null, {"label": "selected"}],
                "other": "ignored"
            }
        });

        let scan = data_value_safety(&value, &["value"]);
        assert!(scan.may_contain_data_values);
        assert_eq!(scan.literal_count, 3);
    }

    #[test]
    fn raw_uri_and_key_credential_profiles_keep_their_distinct_semantics() {
        let value = json!({"api_key": "redacted", "url": " https://example.test"});
        assert!(!contains_external_uri(&value));
        assert!(contains_credential_like_text(
            &value,
            CREDENTIAL_NEEDLES,
            true
        ));
        assert!(!contains_credential_like_text(
            &value,
            CREDENTIAL_NEEDLES,
            false
        ));
    }

    #[test]
    fn credential_matcher_covers_assignment_headers_and_token_families() {
        for sample in [
            "Password=hunter2",
            "pwd = hunter2",
            "pass: hunter2",
            "Account Key = abc",
            "SharedAccessSignature=abc",
            "shared access signature = abc",
            "sas_token=abc",
            "https://example.test/?sig=abc",
            "Authorization: Bearer abc.def",
            "x-api-key: abc",
            "x api key = abc",
            "apikey=abc",
            "api_key = abc",
            "user=alice",
            "username = alice",
            "user id=alice",
            "userid=alice",
            "uid=alice",
            "credential=abc",
            "secret = abc",
            "token=abc",
            "accesskey=abc",
            "access_key = abc",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "github_pat_abcdefghijklmnopqrstuvwxyz",
            "AKIA1234567890ABCDEF",
        ] {
            assert!(
                contains_credential_like_text_str(sample),
                "expected credential match for {sample:?}"
            );
        }
    }

    #[test]
    fn credential_matcher_anchors_keywords_to_credential_syntax() {
        for sample in [
            "Passwort andern",
            "passwort=anzeigen",
            "compassion=value",
            "authorization is required",
            "Authorization: Basic abc.def",
            "user experience",
            "signature status",
            "the secret garden",
        ] {
            assert!(
                !contains_credential_like_text_str(sample),
                "unexpected credential match for {sample:?}"
            );
        }
    }

    #[test]
    fn credential_redaction_preserves_keys_and_syntax() {
        let source = r#"Server=corp;Password=hunter2;User = alice;Authorization: Bearer abc.def
{"x-api-key":"top-secret","label":"safe"}"#;
        let redacted = redact_credential_values(source);
        assert_eq!(
            redacted,
            r#"Server=corp;Password=***;User = ***;Authorization: Bearer ***
{"x-api-key":"***","label":"safe"}"#
        );
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("alice"));
        assert!(!redacted.contains("abc.def"));
        assert!(!redacted.contains("top-secret"));
        assert_eq!(redact_credential_parameter("password", "hunter2"), "***");
        assert_eq!(
            redact_credential_parameter("server", "corp;pwd=hunter2"),
            "corp;pwd=***"
        );
        assert_eq!(
            redact_credential_values("Password=hunter two"),
            "Password=***"
        );
    }

    #[test]
    fn generated_m_table_scan_requires_shape_and_reviews_pii_literals() {
        let safe = r#"let
    Source = #table(
        type table [Name = text, Count = number, Created = date],
        {{"Sample Customer", 1, #date(2026, 1, 1)}}
    )
in
    Source"#;
        assert_eq!(
            generated_m_table_safety(
                safe,
                &[
                    "Name".to_string(),
                    "Count".to_string(),
                    "Created".to_string()
                ]
            ),
            MTableSafety {
                valid_generator_shape: true,
                pii_suspect: false
            }
        );

        let mismatched =
            r#"let Source = #table(type table [Name = text], {{"Alice", 1}}) in Source"#;
        assert!(!generated_m_table_safety(mismatched, &["Name".to_string()]).valid_generator_shape);

        let wrong_columns =
            r#"let Source = #table(type table [Alias = text], {{"Sample"}}) in Source"#;
        assert!(
            !generated_m_table_safety(wrong_columns, &["Name".to_string()]).valid_generator_shape
        );

        let pii = r#"let Source = #table(type table [Name = text], {{"Alice Smith"}}) in Source"#;
        assert_eq!(
            generated_m_table_safety(pii, &["Name".to_string()]),
            MTableSafety {
                valid_generator_shape: true,
                pii_suspect: true
            }
        );
    }

    #[test]
    fn json_row_pii_scan_ignores_non_row_prose() {
        let pii = json!({"tables": [{"rows": [{"Name": "Alice Smith"}]}]});
        assert!(contains_pii_suspect_text(&pii.to_string()));
        let email = json!({"rows": [{"Email": "alice@testcorp.example"}]});
        assert!(contains_pii_suspect_text(&email.to_string()));
        let non_marker_name = json!({"rows": [{"Name": "Contest Winner"}]});
        assert!(contains_pii_suspect_text(&non_marker_name.to_string()));

        let prose = json!({
            "description": "This intentionally long documentation sentence is outside tabular row data and is not classified as a PII-bearing row literal even when it exceeds the review threshold."
        });
        assert!(!contains_pii_suspect_text(&prose.to_string()));
    }
}
