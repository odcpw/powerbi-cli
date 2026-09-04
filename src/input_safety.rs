//! Uniform safety contract for files supplied at CLI input boundaries.
//!
//! Existing readers should call [`read_bytes`] or [`read_utf8`] with a typed
//! [`InputKind`] instead of reading an unbounded file. Profile row inference
//! uses [`read_rows`]; future `$include`, image, ops, snapshot, and
//! Desktop-reference-harvesting commands must use the purpose-built APIs in
//! this module. The module does not add command stubs: it provides the limits
//! and refusal behavior that the owning command beads must call when those
//! surfaces land.

use crate::{CliError, CliResult, EXIT_VALIDATION_FAILED};
use serde_json::{Value, json};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

pub(crate) const INPUT_SAFETY_ERROR_CODE: &str = "input_safety_violation";

pub(crate) const MAX_SCHEMA_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_PROFILE_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_DASHBOARD_SPEC_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_JSON_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_PROJECT_TEXT_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_SOURCE_TEXT_BYTES: u64 = 2 * 1024 * 1024;
pub(crate) const MAX_INTENT_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_ROWS_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_ROWS: usize = 100_000;
pub(crate) const MAX_COLUMNS: usize = 512;
pub(crate) const MAX_PNG_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_OPS_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_HARVESTED_FRAGMENT_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_INCLUDE_DEPTH: usize = 8;
pub(crate) const MAX_RESOLVED_FRAGMENTS: usize = 200;
pub(crate) const MAX_SNAPSHOT_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_FILES: usize = 10_000;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputKind {
    Schema,
    Profile,
    DashboardSpec,
    JsonArtifact,
    ProjectText,
    SourceText,
    Intent,
    Rows,
    PngImage,
    Ops,
    Snapshot,
    IncludeFragment,
    HarvestedFragment,
}

impl InputKind {
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Schema => "schema",
            Self::Profile => "profile",
            Self::DashboardSpec => "dashboard-spec",
            Self::JsonArtifact => "json-artifact",
            Self::ProjectText => "project-text",
            Self::SourceText => "source-text",
            Self::Intent => "intent",
            Self::Rows => "rows",
            Self::PngImage => "png-image",
            Self::Ops => "ops",
            Self::Snapshot => "snapshot",
            Self::IncludeFragment => "include-fragment",
            Self::HarvestedFragment => "harvested-fragment",
        }
    }

    pub(crate) const fn max_bytes(self) -> u64 {
        match self {
            Self::Schema => MAX_SCHEMA_BYTES,
            Self::Profile => MAX_PROFILE_BYTES,
            Self::DashboardSpec | Self::IncludeFragment => MAX_DASHBOARD_SPEC_BYTES,
            Self::JsonArtifact => MAX_JSON_ARTIFACT_BYTES,
            Self::ProjectText => MAX_PROJECT_TEXT_BYTES,
            Self::SourceText => MAX_SOURCE_TEXT_BYTES,
            Self::Intent => MAX_INTENT_BYTES,
            Self::Rows => MAX_ROWS_FILE_BYTES,
            Self::PngImage => MAX_PNG_BYTES,
            Self::Ops => MAX_OPS_BYTES,
            Self::Snapshot => MAX_SNAPSHOT_BYTES,
            Self::HarvestedFragment => MAX_HARVESTED_FRAGMENT_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RowsDocument {
    Csv(Vec<Vec<String>>),
    Json(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoundedRows {
    pub(crate) document: RowsDocument,
    pub(crate) row_count: usize,
    pub(crate) column_count: usize,
}

/// Stateful include accounting for the future schema/spec composition surface.
///
/// The caller supplies the current recursion depth and canonical active stack;
/// this guard owns the root-containment and total-fragment counters. The root
/// document itself is not counted among the 200 resolved fragments.
#[derive(Debug)]
pub(crate) struct IncludeGuard {
    root: PathBuf,
    resolved_fragments: usize,
}

impl IncludeGuard {
    pub(crate) fn new(root_document: &Path) -> CliResult<Self> {
        let root_document = canonical_plain_file(root_document, InputKind::IncludeFragment)?;
        let root = root_document
            .parent()
            .ok_or_else(|| refusal(InputKind::IncludeFragment, "include root has no parent"))?
            .to_path_buf();
        Ok(Self {
            root,
            resolved_fragments: 0,
        })
    }

    pub(crate) fn resolve(
        &mut self,
        including_document: &Path,
        requested: &Path,
        depth: usize,
        active_stack: &[PathBuf],
    ) -> CliResult<PathBuf> {
        if depth > MAX_INCLUDE_DEPTH {
            return Err(refusal(
                InputKind::IncludeFragment,
                format!("include depth {depth} exceeds {MAX_INCLUDE_DEPTH}"),
            ));
        }
        if self.resolved_fragments >= MAX_RESOLVED_FRAGMENTS {
            return Err(refusal(
                InputKind::IncludeFragment,
                format!("resolved fragment count would exceed {MAX_RESOLVED_FRAGMENTS}"),
            ));
        }
        validate_relative_path(requested, InputKind::IncludeFragment)?;
        let including_document = fs::canonicalize(including_document).map_err(|error| {
            CliError::file_not_found(format!(
                "resolve including document {}: {error}",
                including_document.display()
            ))
        })?;
        if !including_document.starts_with(&self.root) {
            return Err(refusal(
                InputKind::IncludeFragment,
                "including document is outside the include root",
            ));
        }
        let parent = including_document.parent().ok_or_else(|| {
            refusal(
                InputKind::IncludeFragment,
                "including document has no parent directory",
            )
        })?;
        let mut candidate = parent.to_path_buf();
        for component in requested.components() {
            if let Component::Normal(part) = component {
                candidate.push(part);
                let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
                    CliError::file_not_found(format!(
                        "inspect include fragment {}: {error}",
                        candidate.display()
                    ))
                })?;
                if metadata_is_link_or_reparse(&metadata) {
                    return Err(refusal(
                        InputKind::IncludeFragment,
                        format!(
                            "symlink or reparse-point include is refused: {}",
                            candidate.display()
                        ),
                    ));
                }
            }
        }
        let candidate = canonical_plain_file(&candidate, InputKind::IncludeFragment)?;
        if !candidate.starts_with(&self.root) {
            return Err(refusal(
                InputKind::IncludeFragment,
                "include fragment resolves outside the include root",
            ));
        }
        if active_stack.iter().any(|active| active == &candidate) {
            return Err(refusal(
                InputKind::IncludeFragment,
                format!("include cycle detected at {}", candidate.display()),
            ));
        }
        self.resolved_fragments += 1;
        Ok(candidate)
    }
}

pub(crate) fn limits_json() -> Value {
    json!({
        "errorCode": INPUT_SAFETY_ERROR_CODE,
        "reservedApis": reserved_api_contract(),
        "schema": { "maxBytes": MAX_SCHEMA_BYTES, "utf8": true, "symlinks": "refused" },
        "profile": { "maxBytes": MAX_PROFILE_BYTES, "utf8": true, "symlinks": "refused" },
        "dashboardSpec": { "maxBytes": MAX_DASHBOARD_SPEC_BYTES, "utf8": true, "symlinks": "refused" },
        "jsonArtifact": { "maxBytes": MAX_JSON_ARTIFACT_BYTES, "utf8": true, "symlinks": "refused" },
        "projectText": { "maxBytesPerFile": MAX_PROJECT_TEXT_BYTES, "utf8": true, "symlinks": "refused" },
        "sourceText": { "maxBytes": MAX_SOURCE_TEXT_BYTES, "utf8": true, "symlinks": "refused" },
        "include": {
            "relativeOnly": true,
            "canonicalized": true,
            "symlinks": "refused",
            "cycles": "refused",
            "maxDepth": MAX_INCLUDE_DEPTH,
            "maxResolvedFragments": MAX_RESOLVED_FRAGMENTS,
            "maxFragmentBytes": MAX_DASHBOARD_SPEC_BYTES
        },
        "rows": {
            "maxFileBytes": MAX_ROWS_FILE_BYTES,
            "maxRows": MAX_ROWS,
            "maxColumns": MAX_COLUMNS,
            "utf8": true,
            "decodeErrors": "refused",
            "leadingFormulaCharacters": "preserved-verbatim"
        },
        "intent": {
            "maxBytes": MAX_INTENT_BYTES,
            "utf8": true,
            "includeAndExecDirectives": "refused"
        },
        "images": {
            "formats": ["png"],
            "maxBytes": MAX_PNG_BYTES,
            "magicByteSniffed": true,
            "externalUrls": "refused"
        },
        "ops": {
            "maxBytes": MAX_OPS_BYTES,
            "schema": "powerbi-cli.ops.v1",
            "schemaValidationBeforeApply": true,
            "unknownOpKinds": "refused"
        },
        "snapshots": {
            "maxFiles": MAX_SNAPSHOT_FILES,
            "maxTotalBytes": MAX_SNAPSHOT_BYTES,
            "location": "sibling-or-explicit-snapshot-dir",
            "unwritableDestination": "refused"
        },
        "harvestedFragments": {
            "maxBytes": MAX_HARVESTED_FRAGMENT_BYTES,
            "persistedDataValues": "refused",
            "silentStripping": false
        }
    })
}

fn reserved_api_contract() -> Vec<&'static str> {
    // Function-pointer assignments make the capabilities declaration a
    // compile-time check that every reserved surface still has a callable API.
    let _: fn(&Path) -> CliResult<BoundedRows> = read_rows;
    let _: fn(&Path) -> CliResult<Vec<u8>> = read_png;
    let _: fn(&Path, &[&str]) -> CliResult<Value> = read_ops;
    let _: fn(&Path, Option<&Path>) -> CliResult<PathBuf> = snapshot_destination;
    let _: fn(&Path) -> CliResult<Value> = read_harvested_fragment;
    let _: fn(&Path) -> CliResult<IncludeGuard> = IncludeGuard::new;
    let _: fn(&mut IncludeGuard, &Path, &Path, usize, &[PathBuf]) -> CliResult<PathBuf> =
        IncludeGuard::resolve;
    vec![
        "IncludeGuard::resolve",
        "read_rows",
        "read_png",
        "read_ops",
        "snapshot_destination",
        "read_harvested_fragment",
    ]
}

pub(crate) fn read_bytes(path: &Path, kind: InputKind) -> CliResult<Vec<u8>> {
    let path = canonical_plain_file(path, kind)?;
    let mut file = File::open(&path).map_err(|error| {
        CliError::file_not_found(format!("open {} {}: {error}", kind.id(), path.display()))
    })?;
    let expected_len = file
        .metadata()
        .map_err(|error| {
            CliError::unexpected(format!("inspect {} {}: {error}", kind.id(), path.display()))
        })?
        .len();
    let mut bytes = Vec::with_capacity(expected_len.min(kind.max_bytes()) as usize);
    file.by_ref()
        .take(kind.max_bytes().saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::file_not_found(format!("read {} {}: {error}", kind.id(), path.display()))
        })?;
    if bytes.len() as u64 > kind.max_bytes() || bytes.len() as u64 != expected_len {
        return Err(refusal(
            kind,
            format!(
                "file changed length while being read or exceeds {} bytes: {}",
                kind.max_bytes(),
                path.display()
            ),
        ));
    }
    Ok(bytes)
}

pub(crate) fn read_utf8(path: &Path, kind: InputKind) -> CliResult<String> {
    let bytes = read_bytes(path, kind)?;
    let text = String::from_utf8(bytes).map_err(|_| {
        refusal(
            kind,
            format!("input is not valid UTF-8: {}", path.display()),
        )
    })?;
    validate_text(&text, kind)?;
    Ok(text)
}

pub(crate) fn read_utf8_stream(
    reader: &mut impl Read,
    kind: InputKind,
    label: &str,
) -> CliResult<String> {
    let mut bytes = Vec::new();
    reader
        .take(kind.max_bytes().saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| CliError::unexpected(format!("read {label}: {error}")))?;
    if bytes.len() as u64 > kind.max_bytes() {
        return Err(refusal(
            kind,
            format!("{label} exceeds {} bytes", kind.max_bytes()),
        ));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| refusal(kind, format!("{label} is not valid UTF-8")))?;
    validate_text(&text, kind)?;
    Ok(text)
}

pub(crate) fn validate_text(text: &str, kind: InputKind) -> CliResult<()> {
    if text.len() as u64 > kind.max_bytes() {
        return Err(refusal(
            kind,
            format!("inline input exceeds {} bytes", kind.max_bytes()),
        ));
    }
    if text.contains('\0') {
        return Err(refusal(kind, "text input contains a NUL character"));
    }
    if kind == InputKind::Intent {
        reject_intent_directives(text)?;
    }
    Ok(())
}

pub(crate) fn read_rows(path: &Path) -> CliResult<BoundedRows> {
    let text = read_utf8(path, InputKind::Rows)?;
    let is_json = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    if is_json {
        parse_json_rows(&text)
    } else {
        parse_csv_rows(&text)
    }
}

pub(crate) fn read_png(path: &Path) -> CliResult<Vec<u8>> {
    let path_text = path.to_string_lossy();
    if path_text.contains("://") || path_text.starts_with("data:") {
        return Err(refusal(
            InputKind::PngImage,
            "external and data URLs are not accepted; supply a local PNG file",
        ));
    }
    let bytes = read_bytes(path, InputKind::PngImage)?;
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err(refusal(
            InputKind::PngImage,
            "image is not a PNG according to its magic bytes",
        ));
    }
    Ok(bytes)
}

pub(crate) fn read_ops(path: &Path, known_kinds: &[&str]) -> CliResult<Value> {
    let text = read_utf8(path, InputKind::Ops)?;
    let value: Value = serde_json::from_str(&text).map_err(|error| {
        refusal(
            InputKind::Ops,
            format!("ops file is not valid JSON: {error}"),
        )
    })?;
    if value.get("schema").and_then(Value::as_str) != Some("powerbi-cli.ops.v1") {
        return Err(refusal(
            InputKind::Ops,
            "ops file schema must be powerbi-cli.ops.v1",
        ));
    }
    let ops = value
        .get("ops")
        .and_then(Value::as_array)
        .ok_or_else(|| refusal(InputKind::Ops, "ops file must contain an ops array"))?;
    for (index, op) in ops.iter().enumerate() {
        // Durable operation plans use the typed IR's `op` tag. Keep accepting
        // the historical safety-harness spelling `kind` so this boundary can
        // validate both forms before the owning parser normalizes them.
        let (kind_value, field) = match op.get("op") {
            Some(value) => (Some(value), "op"),
            None => (op.get("kind"), "kind"),
        };
        let kind = kind_value.and_then(Value::as_str).ok_or_else(|| {
            refusal(
                InputKind::Ops,
                format!("ops[{index}].{field} must be a string"),
            )
        })?;
        if !known_kinds.contains(&kind) {
            return Err(refusal(
                InputKind::Ops,
                format!("ops[{index}] uses unknown op kind `{kind}`"),
            ));
        }
    }
    Ok(value)
}

pub(crate) fn snapshot_destination(
    project_root: &Path,
    requested: Option<&Path>,
) -> CliResult<PathBuf> {
    let source_metadata = fs::symlink_metadata(project_root).map_err(|error| {
        CliError::file_not_found(format!(
            "inspect snapshot source {}: {error}",
            project_root.display()
        ))
    })?;
    if !source_metadata.is_dir() || metadata_is_link_or_reparse(&source_metadata) {
        return Err(refusal(
            InputKind::Snapshot,
            "snapshot source must be an ordinary non-symlink directory",
        ));
    }
    let project_root = fs::canonicalize(project_root).map_err(|error| {
        CliError::file_not_found(format!(
            "resolve snapshot source {}: {error}",
            project_root.display()
        ))
    })?;
    validate_snapshot_tree(&project_root)?;
    let default_name = project_root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| refusal(InputKind::Snapshot, "snapshot source needs a Unicode name"))?;
    let candidate = requested.map(Path::to_path_buf).unwrap_or_else(|| {
        project_root
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{default_name}.snapshot"))
    });
    if candidate.exists() {
        return Err(refusal(
            InputKind::Snapshot,
            format!(
                "snapshot destination already exists: {}",
                candidate.display()
            ),
        ));
    }
    let parent = candidate
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let metadata = fs::symlink_metadata(parent).map_err(|error| {
        refusal(
            InputKind::Snapshot,
            format!("snapshot destination parent is unavailable: {error}"),
        )
    })?;
    if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
        return Err(refusal(
            InputKind::Snapshot,
            "snapshot destination parent must be an ordinary directory",
        ));
    }
    let parent = fs::canonicalize(parent).map_err(|error| {
        refusal(
            InputKind::Snapshot,
            format!("resolve snapshot destination parent: {error}"),
        )
    })?;
    let candidate = parent.join(
        candidate
            .file_name()
            .ok_or_else(|| refusal(InputKind::Snapshot, "snapshot destination needs a name"))?,
    );
    if candidate.starts_with(&project_root) {
        return Err(refusal(
            InputKind::Snapshot,
            "snapshot destination must be a sibling or outside the project",
        ));
    }
    probe_writable(&parent)?;
    Ok(candidate)
}

pub(crate) fn read_harvested_fragment(path: &Path) -> CliResult<Value> {
    let text = read_utf8(path, InputKind::HarvestedFragment)?;
    let value: Value = serde_json::from_str(&text).map_err(|error| {
        refusal(
            InputKind::HarvestedFragment,
            format!("harvested fragment is not valid JSON: {error}"),
        )
    })?;
    if let Some(pointer) = persisted_data_pointer(&value, "", false) {
        return Err(refusal(
            InputKind::HarvestedFragment,
            format!("persisted data values remain at {pointer}"),
        ));
    }
    Ok(value)
}

fn canonical_plain_file(path: &Path, kind: InputKind) -> CliResult<PathBuf> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliError::file_not_found(format!("inspect {} {}: {error}", kind.id(), path.display()))
    })?;
    if !metadata.is_file() {
        return Err(refusal(
            kind,
            format!("input must be an ordinary file: {}", path.display()),
        ));
    }
    if metadata_is_link_or_reparse(&metadata) {
        return Err(refusal(
            kind,
            format!(
                "symbolic links and reparse points are refused: {}",
                path.display()
            ),
        ));
    }
    if metadata.len() > kind.max_bytes() {
        return Err(refusal(
            kind,
            format!(
                "input is {} bytes; maximum is {} bytes: {}",
                metadata.len(),
                kind.max_bytes(),
                path.display()
            ),
        ));
    }
    fs::canonicalize(path).map_err(|error| {
        CliError::file_not_found(format!("resolve {} {}: {error}", kind.id(), path.display()))
    })
}

fn validate_relative_path(path: &Path, kind: InputKind) -> CliResult<()> {
    if path.as_os_str().is_empty()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(refusal(
            kind,
            format!(
                "path must be relative and must not contain `..`: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn reject_intent_directives(text: &str) -> CliResult<()> {
    for (line_index, line) in text.lines().enumerate() {
        let normalized = line.trim_start().to_ascii_lowercase();
        let is_directive = [
            "$include", "@include", "!include", "#include", "include:", "$exec", "@exec", "!exec",
            "#exec", "exec:",
        ]
        .iter()
        .any(|prefix| normalized.starts_with(prefix));
        if is_directive {
            return Err(refusal(
                InputKind::Intent,
                format!(
                    "include/exec directive is refused at line {}",
                    line_index + 1
                ),
            ));
        }
    }
    if let Ok(value) = serde_json::from_str::<Value>(text)
        && let Some(pointer) = intent_json_directive_pointer(&value, "")
    {
        return Err(refusal(
            InputKind::Intent,
            format!("include/exec directive is refused at {pointer}"),
        ));
    }
    Ok(())
}

fn intent_json_directive_pointer(value: &Value, path: &str) -> Option<String> {
    match value {
        Value::Object(object) => object.iter().find_map(|(key, child)| {
            let pointer = format!("{path}/{}", escape_pointer(key));
            let normalized = key
                .trim()
                .trim_start_matches(['$', '@', '!', '#'])
                .to_ascii_lowercase();
            if matches!(normalized.as_str(), "include" | "exec") {
                Some(pointer)
            } else {
                intent_json_directive_pointer(child, &pointer)
            }
        }),
        Value::Array(items) => items.iter().enumerate().find_map(|(index, child)| {
            intent_json_directive_pointer(child, &format!("{path}/{index}"))
        }),
        _ => None,
    }
}

fn parse_json_rows(text: &str) -> CliResult<BoundedRows> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| refusal(InputKind::Rows, format!("rows JSON decode failed: {error}")))?;
    let rows = value
        .as_array()
        .ok_or_else(|| refusal(InputKind::Rows, "rows JSON root must be an array"))?;
    if rows.len() > MAX_ROWS {
        return Err(refusal(
            InputKind::Rows,
            format!(
                "rows JSON contains {} rows; maximum is {MAX_ROWS}",
                rows.len()
            ),
        ));
    }
    let mut column_count = 0;
    for (index, row) in rows.iter().enumerate() {
        let columns = row
            .as_array()
            .map(Vec::len)
            .or_else(|| row.as_object().map(serde_json::Map::len))
            .ok_or_else(|| {
                refusal(
                    InputKind::Rows,
                    format!("rows JSON item {index} must be an array or object"),
                )
            })?;
        if columns > MAX_COLUMNS {
            return Err(refusal(
                InputKind::Rows,
                format!("rows JSON item {index} has {columns} columns; maximum is {MAX_COLUMNS}"),
            ));
        }
        column_count = column_count.max(columns);
    }
    let row_count = rows.len();
    Ok(BoundedRows {
        document: RowsDocument::Json(value),
        row_count,
        column_count,
    })
}

fn parse_csv_rows(text: &str) -> CliResult<BoundedRows> {
    let mut rows = Vec::<Vec<String>>::new();
    let mut row = Vec::<String>::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    let mut quote_closed = false;
    let mut at_field_start = true;
    let mut record_started = false;
    while let Some(character) = chars.next() {
        if quoted {
            if character == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    quote_closed = true;
                }
            } else {
                field.push(character);
            }
            continue;
        }
        if quote_closed && !matches!(character, ',' | '\r' | '\n') {
            return Err(refusal(
                InputKind::Rows,
                "CSV decode failed: characters followed a closing quote",
            ));
        }
        match character {
            '"' if at_field_start => {
                quoted = true;
                at_field_start = false;
                record_started = true;
            }
            ',' => {
                row.push(std::mem::take(&mut field));
                at_field_start = true;
                quote_closed = false;
                record_started = true;
                check_column_count(row.len())?;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                push_csv_row(&mut rows, &mut row, &mut field)?;
                at_field_start = true;
                quote_closed = false;
                record_started = false;
            }
            '\n' => {
                push_csv_row(&mut rows, &mut row, &mut field)?;
                at_field_start = true;
                quote_closed = false;
                record_started = false;
            }
            '"' => {
                return Err(refusal(
                    InputKind::Rows,
                    "CSV decode failed: quote appeared inside an unquoted field",
                ));
            }
            other => {
                field.push(other);
                at_field_start = false;
                record_started = true;
            }
        }
    }
    if quoted {
        return Err(refusal(
            InputKind::Rows,
            "CSV decode failed: unterminated quoted field",
        ));
    }
    if record_started || !field.is_empty() || !row.is_empty() {
        push_csv_row(&mut rows, &mut row, &mut field)?;
    }
    let column_count = rows.iter().map(Vec::len).max().unwrap_or_default();
    Ok(BoundedRows {
        row_count: rows.len(),
        column_count,
        document: RowsDocument::Csv(rows),
    })
}

fn push_csv_row(
    rows: &mut Vec<Vec<String>>,
    row: &mut Vec<String>,
    field: &mut String,
) -> CliResult<()> {
    row.push(std::mem::take(field));
    check_column_count(row.len())?;
    if rows.len() >= MAX_ROWS {
        return Err(refusal(
            InputKind::Rows,
            format!("CSV row count would exceed {MAX_ROWS}"),
        ));
    }
    rows.push(std::mem::take(row));
    Ok(())
}

fn check_column_count(columns: usize) -> CliResult<()> {
    if columns > MAX_COLUMNS {
        return Err(refusal(
            InputKind::Rows,
            format!("CSV column count would exceed {MAX_COLUMNS}"),
        ));
    }
    Ok(())
}

fn validate_snapshot_tree(project_root: &Path) -> CliResult<()> {
    validate_snapshot_tree_with_limits(project_root, MAX_SNAPSHOT_FILES, MAX_SNAPSHOT_BYTES)
}

fn validate_snapshot_tree_with_limits(
    project_root: &Path,
    max_files: usize,
    max_bytes: u64,
) -> CliResult<()> {
    let mut files = 0usize;
    let mut bytes = 0u64;
    for entry in WalkDir::new(project_root).follow_links(false) {
        let entry = entry.map_err(|error| {
            refusal(
                InputKind::Snapshot,
                format!("walk snapshot source: {error}"),
            )
        })?;
        let metadata = entry.metadata().map_err(|error| {
            refusal(
                InputKind::Snapshot,
                format!("inspect snapshot source: {error}"),
            )
        })?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(refusal(
                InputKind::Snapshot,
                format!(
                    "snapshot source contains a link: {}",
                    entry.path().display()
                ),
            ));
        }
        if metadata.is_file() {
            files += 1;
            bytes = bytes.saturating_add(metadata.len());
            if files > max_files || bytes > max_bytes {
                return Err(refusal(
                    InputKind::Snapshot,
                    format!("snapshot source exceeds {max_files} files or {max_bytes} bytes"),
                ));
            }
        }
    }
    Ok(())
}

fn probe_writable(parent: &Path) -> CliResult<()> {
    let probe = parent.join(format!(".powerbi-cli-write-probe-{}", uuid::Uuid::new_v4()));
    let result = OpenOptions::new().write(true).create_new(true).open(&probe);
    match result {
        Ok(_) => {
            fs::remove_file(&probe).map_err(|error| {
                CliError::unexpected(format!("remove snapshot writability probe: {error}"))
            })?;
            Ok(())
        }
        Err(error) => Err(refusal(
            InputKind::Snapshot,
            format!("snapshot destination is not writable: {error}"),
        )),
    }
}

fn persisted_data_pointer(value: &Value, path: &str, in_filter: bool) -> Option<String> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let pointer = format!("{}/{}", path, escape_pointer(key));
                let lower = key.to_ascii_lowercase();
                let child_in_filter = in_filter
                    || matches!(
                        lower.as_str(),
                        "filter" | "filterconfig" | "where" | "condition" | "in"
                    );
                if matches!(
                    lower.as_str(),
                    "cachedvalueitems"
                        | "cachedvalues"
                        | "selecteditems"
                        | "selectedvalues"
                        | "selectionstate"
                        | "persistedselection"
                ) || (lower == "values" && child_in_filter)
                {
                    return Some(pointer);
                }
                if let Some(found) = persisted_data_pointer(child, &pointer, child_in_filter) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().enumerate().find_map(|(index, child)| {
            persisted_data_pointer(child, &format!("{path}/{index}"), in_filter)
        }),
        _ => None,
    }
}

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn refusal(kind: InputKind, detail: impl Into<String>) -> CliError {
    CliError::new(
        INPUT_SAFETY_ERROR_CODE,
        EXIT_VALIDATION_FAILED,
        format!("{} input refused: {}", kind.id(), detail.into()),
    )
    .with_hint("Inspect the documented input limits and supply a bounded ordinary file; limits are not silently raised.")
    .with_suggested_command("powerbi-cli --json capabilities")
}

fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_sized(path: &Path, bytes: u64) {
        let file = File::create(path).expect("create bounded input");
        file.set_len(bytes).expect("size bounded input");
    }

    #[test]
    fn every_per_file_byte_budget_accepts_the_limit_and_refuses_one_byte_over() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cases = [
            InputKind::Schema,
            InputKind::Profile,
            InputKind::DashboardSpec,
            InputKind::JsonArtifact,
            InputKind::ProjectText,
            InputKind::SourceText,
            InputKind::Intent,
            InputKind::Rows,
            InputKind::PngImage,
            InputKind::Ops,
            InputKind::IncludeFragment,
            InputKind::HarvestedFragment,
        ];
        for kind in cases {
            let at_limit = temp.path().join(format!("{}-at", kind.id()));
            write_sized(&at_limit, kind.max_bytes());
            assert_eq!(
                read_bytes(&at_limit, kind).expect("at-limit input").len() as u64,
                kind.max_bytes()
            );
            let over_limit = temp.path().join(format!("{}-over", kind.id()));
            write_sized(&over_limit, kind.max_bytes() + 1);
            let error = read_bytes(&over_limit, kind).expect_err("over-limit input");
            assert_eq!(error.code, INPUT_SAFETY_ERROR_CODE);
            assert_eq!(error.exit_code, EXIT_VALIDATION_FAILED);
            assert!(error.hint.is_some());
            assert_eq!(
                error.suggested_commands,
                ["powerbi-cli --json capabilities"]
            );
        }
    }

    #[test]
    fn rows_limits_and_formula_prefixes_are_exact() {
        let text = "name,value\n=SUM(A1),+one\n-minus,@mention\n";
        let parsed = parse_csv_rows(text).expect("CSV rows");
        let RowsDocument::Csv(rows) = parsed.document else {
            panic!("expected CSV rows");
        };
        assert_eq!(rows[1], ["=SUM(A1)", "+one"]);
        assert_eq!(rows[2], ["-minus", "@mention"]);

        let at_column_limit = format!("{}\n", vec!["x"; MAX_COLUMNS].join(","));
        assert_eq!(
            parse_csv_rows(&at_column_limit)
                .expect("column limit")
                .column_count,
            MAX_COLUMNS
        );
        let too_many_columns = format!("{}\n", vec!["x"; MAX_COLUMNS + 1].join(","));
        assert_eq!(
            parse_csv_rows(&too_many_columns)
                .expect_err("column budget")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
        let at_row_limit = "x\n".repeat(MAX_ROWS);
        assert_eq!(
            parse_csv_rows(&at_row_limit).expect("row limit").row_count,
            MAX_ROWS
        );
        let too_many_rows = "x\n".repeat(MAX_ROWS + 1);
        assert_eq!(
            parse_csv_rows(&too_many_rows).expect_err("row budget").code,
            INPUT_SAFETY_ERROR_CODE
        );
    }

    #[test]
    fn rows_decode_errors_are_not_lossy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("rows.csv");
        fs::write(&path, [0xff, 0xfe]).expect("invalid UTF-8 rows");
        assert_eq!(
            read_rows(&path).expect_err("invalid UTF-8").code,
            INPUT_SAFETY_ERROR_CODE
        );
        assert_eq!(
            parse_csv_rows("a\n\"unterminated")
                .expect_err("invalid CSV")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
        assert_eq!(
            parse_csv_rows("\"closed\"trailing")
                .expect_err("invalid trailing text")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
    }

    #[test]
    fn intent_directives_are_refused() {
        validate_text("Explain monthly revenue", InputKind::Intent).expect("plain intent");
        for directive in [
            "$include hidden.md",
            "@exec shell",
            "  #include x",
            "exec: tool",
        ] {
            assert_eq!(
                validate_text(directive, InputKind::Intent)
                    .expect_err("directive")
                    .code,
                INPUT_SAFETY_ERROR_CODE
            );
        }
        assert_eq!(
            validate_text(
                r#"{"nested":{"$include":"hidden.json"}}"#,
                InputKind::Intent
            )
            .expect_err("JSON directive")
            .code,
            INPUT_SAFETY_ERROR_CODE
        );
    }

    #[test]
    fn png_is_sniffed_by_magic_bytes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let valid = temp.path().join("valid.bin");
        fs::write(&valid, PNG_SIGNATURE).expect("PNG signature");
        assert_eq!(read_png(&valid).expect("valid PNG"), PNG_SIGNATURE);
        let at_limit = temp.path().join("at-limit.png");
        let mut bytes = vec![0; MAX_PNG_BYTES as usize];
        bytes[..PNG_SIGNATURE.len()].copy_from_slice(PNG_SIGNATURE);
        fs::write(&at_limit, bytes).expect("at-limit PNG");
        assert_eq!(
            read_png(&at_limit).expect("PNG at byte limit").len() as u64,
            MAX_PNG_BYTES
        );
        let fake = temp.path().join("fake.png");
        fs::write(&fake, b"not a png").expect("fake PNG");
        assert_eq!(
            read_png(&fake).expect_err("magic bytes").code,
            INPUT_SAFETY_ERROR_CODE
        );
        assert_eq!(
            read_png(Path::new("https://example.invalid/image.png"))
                .expect_err("external URL")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
    }

    #[test]
    fn ops_schema_and_unknown_kinds_are_refused_before_apply() {
        let temp = tempfile::tempdir().expect("tempdir");
        let valid = temp.path().join("valid.ops.json");
        fs::write(
            &valid,
            r#"{"schema":"powerbi-cli.ops.v1","ops":[{"kind":"AddMeasure"}]}"#,
        )
        .expect("valid ops");
        read_ops(&valid, &["AddMeasure"]).expect("known op");
        let error = read_ops(&valid, &["AddVisual"]).expect_err("unknown op");
        assert_eq!(error.code, INPUT_SAFETY_ERROR_CODE);

        let wrong_schema = temp.path().join("wrong-schema.ops.json");
        fs::write(&wrong_schema, r#"{"schema":"powerbi-cli.ops.v2","ops":[]}"#)
            .expect("wrong-schema ops");
        assert_eq!(
            read_ops(&wrong_schema, &[])
                .expect_err("wrong ops schema")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
    }

    #[test]
    fn harvested_fragments_are_never_silently_stripped() {
        let temp = tempfile::tempdir().expect("tempdir");
        let safe = temp.path().join("safe.json");
        fs::write(
            &safe,
            r#"{"visual":{"objects":{"title":{"text":"Heading"}}}}"#,
        )
        .expect("safe fragment");
        read_harvested_fragment(&safe).expect("safe fragment");
        let unsafe_path = temp.path().join("unsafe.json");
        fs::write(
            &unsafe_path,
            r#"{"filterConfig":{"filters":[{"filter":{"Where":[{"Condition":{"In":{"Values":[1]}}}]}}]}}"#,
        )
        .expect("unsafe fragment");
        let error = read_harvested_fragment(&unsafe_path).expect_err("persisted values");
        assert_eq!(error.code, INPUT_SAFETY_ERROR_CODE);
        assert!(error.message.contains("/Values"));
    }

    #[test]
    fn include_limits_traversal_and_cycles_are_refused() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("root.json");
        let child = temp.path().join("child.json");
        fs::write(&root, "{}").expect("root");
        fs::write(&child, "{}").expect("child");
        let mut guard = IncludeGuard::new(&root).expect("include guard");
        assert_eq!(
            guard
                .resolve(&root, Path::new("../escape.json"), 1, &[])
                .expect_err("traversal")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
        assert!(
            guard
                .resolve(&root, Path::new("child.json"), MAX_INCLUDE_DEPTH, &[])
                .is_ok()
        );
        assert_eq!(
            guard
                .resolve(&root, Path::new("child.json"), MAX_INCLUDE_DEPTH + 1, &[])
                .expect_err("depth")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
        let canonical_child = fs::canonicalize(&child).expect("canonical child");
        assert_eq!(
            guard
                .resolve(&root, Path::new("child.json"), 1, &[canonical_child])
                .expect_err("cycle")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );

        let mut count_guard = IncludeGuard::new(&root).expect("count guard");
        for _ in 0..MAX_RESOLVED_FRAGMENTS {
            count_guard
                .resolve(&root, Path::new("child.json"), 1, &[])
                .expect("fragment at limit");
        }
        assert_eq!(
            count_guard
                .resolve(&root, Path::new("child.json"), 1, &[])
                .expect_err("fragment over limit")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
    }

    #[cfg(unix)]
    #[test]
    fn direct_and_include_symlinks_are_refused() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("root.json");
        let target = temp.path().join("target.json");
        let link = temp.path().join("link.json");
        fs::write(&root, "{}").expect("root");
        fs::write(&target, "{}").expect("target");
        symlink(&target, &link).expect("symlink");
        assert_eq!(
            read_bytes(&link, InputKind::JsonArtifact)
                .expect_err("direct symlink")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
        let mut guard = IncludeGuard::new(&root).expect("guard");
        assert_eq!(
            guard
                .resolve(&root, Path::new("link.json"), 1, &[])
                .expect_err("include symlink")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );

        let project = temp.path().join("project");
        let project_link = temp.path().join("project-link");
        fs::create_dir(&project).expect("snapshot project");
        symlink(&project, &project_link).expect("snapshot source symlink");
        assert_eq!(
            snapshot_destination(&project_link, None)
                .expect_err("snapshot source symlink")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
    }

    #[test]
    fn snapshots_are_sibling_bounded_and_destination_checked() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir(&project).expect("project");
        fs::write(project.join("one.json"), "{}").expect("project file");
        let destination = snapshot_destination(&project, None).expect("sibling snapshot");
        assert_eq!(destination, temp.path().join("project.snapshot"));
        let inside = project.join("snap");
        assert_eq!(
            snapshot_destination(&project, Some(&inside))
                .expect_err("inside snapshot")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
        fs::write(temp.path().join("existing.snapshot"), "occupied").expect("existing snapshot");
        assert_eq!(
            snapshot_destination(&project, Some(&temp.path().join("existing.snapshot")))
                .expect_err("existing snapshot")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );

        validate_snapshot_tree_with_limits(&project, 1, 2).expect("snapshot at limits");
        assert_eq!(
            validate_snapshot_tree_with_limits(&project, 0, 2)
                .expect_err("snapshot file count over limit")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );
        assert_eq!(
            validate_snapshot_tree_with_limits(&project, 1, 1)
                .expect_err("snapshot bytes over limit")
                .code,
            INPUT_SAFETY_ERROR_CODE
        );

        let not_a_directory = temp.path().join("not-a-directory");
        fs::write(&not_a_directory, "x").expect("non-directory parent");
        assert_eq!(
            snapshot_destination(
                &project,
                Some(&not_a_directory.join("cannot-write-snapshot"))
            )
            .expect_err("unusable destination")
            .code,
            INPUT_SAFETY_ERROR_CODE
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_unwritable_destination_is_refused() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let locked = temp.path().join("locked");
        fs::create_dir(&project).expect("project");
        fs::create_dir(&locked).expect("locked destination parent");
        fs::write(project.join("one.json"), "{}").expect("project file");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("lock destination");
        let result = snapshot_destination(&project, Some(&locked.join("snapshot")));
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))
            .expect("restore destination permissions");
        assert_eq!(
            result.expect_err("unwritable destination").code,
            INPUT_SAFETY_ERROR_CODE
        );
    }
}
