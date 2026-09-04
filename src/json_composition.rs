//! Deterministic, bounded `$include` composition for schema and dashboard spec JSON.

use crate::input_safety::{IncludeGuard, InputKind, read_utf8};
use crate::{CliError, CliResult, EXIT_VALIDATION_FAILED, command_arg};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompositionKind {
    Schema,
    DashboardSpec,
}

impl CompositionKind {
    fn input_kind(self) -> InputKind {
        match self {
            Self::Schema => InputKind::Schema,
            Self::DashboardSpec => InputKind::DashboardSpec,
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Schema => "schema normalize",
            Self::DashboardSpec => "report spec normalize",
        }
    }

    fn validation_command(self) -> &'static str {
        match self {
            Self::Schema => "schema validate",
            Self::DashboardSpec => "report spec validate",
        }
    }
}

#[derive(Debug)]
pub(crate) struct NormalizedDocument {
    pub(crate) value: Value,
    pub(crate) normalized_from: Vec<String>,
}

pub(crate) fn normalize_schema_file(path: &Path) -> CliResult<NormalizedDocument> {
    normalize_file(path, CompositionKind::Schema)
}

pub(crate) fn normalize_spec_file(path: &Path) -> CliResult<NormalizedDocument> {
    normalize_file(path, CompositionKind::DashboardSpec)
}

fn normalize_file(path: &Path, kind: CompositionKind) -> CliResult<NormalizedDocument> {
    let guard = IncludeGuard::new(path)?;
    let root_path = fs::canonicalize(path).map_err(|error| {
        CliError::file_not_found(format!(
            "resolve {} {}: {error}",
            kind.input_kind().id(),
            path.display()
        ))
    })?;
    let text = read_utf8(path, kind.input_kind())?;
    let value = parse_json(&text, &root_path, kind, false, "")?;
    let mut resolver = Resolver {
        guard,
        kind,
        root_path: root_path.clone(),
        normalized_from: BTreeSet::new(),
    };
    let mut active_stack = vec![root_path.clone()];
    let value = match kind {
        CompositionKind::Schema => {
            resolver.schema_root(value, &root_path, &mut active_stack, "")?
        }
        CompositionKind::DashboardSpec => {
            resolver.spec_root(value, &root_path, &mut active_stack, "")?
        }
    };
    reject_unresolved_include(&value, "", kind, &resolver.root_path)?;
    Ok(NormalizedDocument {
        value: canonicalize_json(value),
        normalized_from: resolver.normalized_from.into_iter().collect(),
    })
}

struct Resolver {
    guard: IncludeGuard,
    kind: CompositionKind,
    root_path: PathBuf,
    normalized_from: BTreeSet<String>,
}

impl Resolver {
    fn schema_root(
        &mut self,
        value: Value,
        file: &Path,
        active_stack: &mut Vec<PathBuf>,
        pointer: &str,
    ) -> CliResult<Value> {
        let Some(object) = value.as_object() else {
            return Ok(value);
        };
        let includes = self.read_includes(object, file, active_stack, pointer)?;
        let include_pointer = format!("{pointer}/$include");
        let mut merged = Map::new();
        for (included, included_file) in includes {
            let included = self.with_active_file(
                included,
                &included_file,
                active_stack,
                &include_pointer,
                |resolver, value, file, stack| resolver.schema_root(value, file, stack, pointer),
            )?;
            merge_object(&mut merged, included, &["tables", "relationships"]);
        }
        let mut local = object.clone();
        local.remove("$include");
        if let Some(tables) = local.get_mut("tables").and_then(Value::as_array_mut) {
            for (index, table) in tables.iter_mut().enumerate() {
                let table_pointer = format!("{pointer}/tables/{index}");
                let normalized =
                    self.schema_table(table.clone(), file, active_stack, &table_pointer)?;
                *table = normalized;
            }
        }
        merge_object(
            &mut merged,
            Value::Object(local),
            &["tables", "relationships"],
        );
        Ok(Value::Object(merged))
    }

    fn schema_table(
        &mut self,
        value: Value,
        file: &Path,
        active_stack: &mut Vec<PathBuf>,
        pointer: &str,
    ) -> CliResult<Value> {
        let Some(object) = value.as_object() else {
            return Ok(value);
        };
        let includes = self.read_includes(object, file, active_stack, pointer)?;
        let include_pointer = format!("{pointer}/$include");
        let mut merged = Map::new();
        for (included, included_file) in includes {
            let included = self.with_active_file(
                included,
                &included_file,
                active_stack,
                &include_pointer,
                |resolver, value, file, stack| resolver.schema_table(value, file, stack, pointer),
            )?;
            merge_object(&mut merged, included, &["columns", "measures", "rows"]);
        }
        let mut local = object.clone();
        local.remove("$include");
        merge_object(
            &mut merged,
            Value::Object(local),
            &["columns", "measures", "rows"],
        );
        Ok(Value::Object(merged))
    }

    fn spec_root(
        &mut self,
        value: Value,
        file: &Path,
        active_stack: &mut Vec<PathBuf>,
        pointer: &str,
    ) -> CliResult<Value> {
        let Some(object) = value.as_object() else {
            return Ok(value);
        };
        let mut normalized = object.clone();
        if let Some(model) = object.get("model") {
            let model_pointer = format!("{pointer}/model");
            normalized.insert(
                "model".to_string(),
                self.spec_section(
                    model.clone(),
                    file,
                    active_stack,
                    &model_pointer,
                    &[
                        "measures",
                        "measurePatterns",
                        "calculatedColumns",
                        "relationships",
                        "staticTables",
                        "sortBy",
                        "formatStrings",
                    ],
                )?,
            );
        }
        if let Some(style) = object.get("style") {
            let style_pointer = format!("{pointer}/style");
            normalized.insert(
                "style".to_string(),
                self.spec_section(style.clone(), file, active_stack, &style_pointer, &[])?,
            );
        }
        if let Some(pages) = object.get("pages").and_then(Value::as_array) {
            let mut normalized_pages = Vec::with_capacity(pages.len());
            for (index, page) in pages.iter().enumerate() {
                let page_pointer = format!("{pointer}/pages/{index}");
                normalized_pages.push(self.spec_section(
                    page.clone(),
                    file,
                    active_stack,
                    &page_pointer,
                    &["filters", "slicers", "visuals", "interactions"],
                )?);
            }
            normalized.insert("pages".to_string(), Value::Array(normalized_pages));
        }
        Ok(Value::Object(normalized))
    }

    fn spec_section(
        &mut self,
        value: Value,
        file: &Path,
        active_stack: &mut Vec<PathBuf>,
        pointer: &str,
        append_arrays: &[&str],
    ) -> CliResult<Value> {
        let Some(object) = value.as_object() else {
            return Ok(value);
        };
        let includes = self.read_includes(object, file, active_stack, pointer)?;
        let include_pointer = format!("{pointer}/$include");
        let mut merged = Map::new();
        for (included, included_file) in includes {
            let included = self.with_active_file(
                included,
                &included_file,
                active_stack,
                &include_pointer,
                |resolver, value, file, stack| {
                    resolver.spec_section(value, file, stack, pointer, append_arrays)
                },
            )?;
            merge_object(&mut merged, included, append_arrays);
        }
        let mut local = object.clone();
        local.remove("$include");
        merge_object(&mut merged, Value::Object(local), append_arrays);
        Ok(Value::Object(merged))
    }

    fn with_active_file<F>(
        &mut self,
        value: Value,
        file: &Path,
        active_stack: &mut Vec<PathBuf>,
        pointer: &str,
        f: F,
    ) -> CliResult<Value>
    where
        F: FnOnce(&mut Self, Value, &Path, &mut Vec<PathBuf>) -> CliResult<Value>,
    {
        active_stack.push(file.to_path_buf());
        let result = if value.is_object() {
            f(self, value, file, active_stack)
        } else {
            Err(CliError::new(
                "include.invalid",
                EXIT_VALIDATION_FAILED,
                "included JSON fragments must contain an object at the supported composition point",
            ))
        };
        active_stack.pop();
        result.map_err(|error| self.attach_include_context(error, pointer, Some(file)))
    }

    fn read_includes(
        &mut self,
        object: &Map<String, Value>,
        including_file: &Path,
        active_stack: &[PathBuf],
        pointer: &str,
    ) -> CliResult<Vec<(Value, PathBuf)>> {
        let Some(raw) = object.get("$include") else {
            return Ok(Vec::new());
        };
        let include_pointer = format!("{pointer}/$include");
        let requests = match raw {
            Value::String(path) => vec![path.as_str()],
            Value::Array(paths) => paths
                .iter()
                .map(|path| {
                    path.as_str().ok_or_else(|| {
                        self.include_argument_error(
                            &include_pointer,
                            "$include array entries must be strings",
                        )
                    })
                })
                .collect::<CliResult<Vec<_>>>()?,
            _ => {
                return Err(self.include_argument_error(
                    &include_pointer,
                    "$include must be a relative path string or an array of relative path strings",
                ));
            }
        };
        let mut fragments = Vec::with_capacity(requests.len());
        for request in requests {
            let requested = PathBuf::from(request);
            let resolved = self
                .guard
                .resolve(including_file, &requested, active_stack.len(), active_stack)
                .map_err(|error| {
                    let mut mapped = self.attach_include_context(error, &include_pointer, None);
                    if mapped.code == "include.cycle" {
                        let chain = active_stack
                            .iter()
                            .map(|path| self.relative_source(path))
                            .chain(std::iter::once(request.to_string()))
                            .collect::<Vec<_>>()
                            .join(" -> ");
                        mapped.message = format!("{}; chain: {chain}", mapped.message);
                    }
                    mapped
                })?;
            let text = read_utf8(&resolved, InputKind::IncludeFragment).map_err(|error| {
                self.attach_include_context(error, &include_pointer, Some(&resolved))
            })?;
            let value = parse_json(&text, &resolved, self.kind, true, &include_pointer)?;
            self.normalized_from.insert(self.relative_source(&resolved));
            fragments.push((value, resolved));
        }
        Ok(fragments)
    }

    fn relative_source(&self, path: &Path) -> String {
        let root = self.root_path.parent().unwrap_or_else(|| Path::new("."));
        path.strip_prefix(root)
            .unwrap_or(path)
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }

    fn include_argument_error(&self, pointer: &str, message: &str) -> CliError {
        CliError::new("include.invalid", EXIT_VALIDATION_FAILED, message)
            .with_pointer(pointer)
            .with_hint(format!(
                "Use a relative JSON fragment path under the root document, then rerun `powerbi-cli {} ... --json`.",
                self.kind.command()
            ))
            .with_suggested_command(format!(
                "powerbi-cli {} {} --json",
                self.kind.validation_command(),
                command_arg(&self.root_path)
            ))
    }

    fn attach_include_context(
        &self,
        error: CliError,
        pointer: &str,
        source: Option<&Path>,
    ) -> CliError {
        let lower = error.message.to_ascii_lowercase();
        let code = if lower.contains("cycle") {
            "include.cycle"
        } else if lower.contains("outside")
            || lower.contains("relative")
            || lower.contains("parentdir")
            || lower.contains("must not contain `..`")
        {
            "include.path_escape"
        } else {
            error.code
        };
        let source_note = source
            .map(|path| format!(" ({})", path.display()))
            .unwrap_or_default();
        let mut mapped = CliError::new(
            code,
            error.exit_code,
            format!("{}{}", error.message, source_note),
        )
        .with_pointer(pointer)
        .with_hint(format!(
            "Resolve the include under the root document and rerun `powerbi-cli {} ... --json`.",
            self.kind.command()
        ))
        .with_suggested_command(format!(
            "powerbi-cli {} {} --json",
            self.kind.validation_command(),
            command_arg(&self.root_path)
        ));
        if let Some(hint) = error.hint {
            mapped = mapped.with_hint(hint);
        }
        for command in error.suggested_commands {
            mapped = mapped.with_suggested_command(command);
        }
        mapped
    }
}

fn parse_json(
    text: &str,
    path: &Path,
    kind: CompositionKind,
    included: bool,
    pointer: &str,
) -> CliResult<Value> {
    serde_json::from_str(text).map_err(|error| {
        let (code, exit_code) = if included {
            ("include.parse", EXIT_VALIDATION_FAILED)
        } else {
            ("invalid_args", 2)
        };
        let mut result = CliError::new(
            code,
            exit_code,
            format!(
                "parse {} {}: {error}",
                kind.input_kind().id(),
                path.display()
            ),
        )
        .with_hint(format!(
            "Fix the JSON fragment and rerun `powerbi-cli {} ... --json`.",
            kind.command()
        ))
        .with_suggested_command(format!(
            "powerbi-cli {} {} --json",
            kind.validation_command(),
            command_arg(path)
        ));
        if included {
            result = result.with_pointer(pointer);
        }
        result
    })
}

fn merge_object(target: &mut Map<String, Value>, incoming: Value, append_arrays: &[&str]) {
    let Some(incoming) = incoming.as_object() else {
        return;
    };
    for (key, value) in incoming {
        let Some(existing) = target.get_mut(key) else {
            target.insert(key.clone(), value.clone());
            continue;
        };
        match (existing, value) {
            (Value::Object(existing), Value::Object(incoming)) => {
                merge_object(existing, Value::Object(incoming.clone()), append_arrays);
            }
            (Value::Array(existing), Value::Array(incoming))
                if append_arrays.contains(&key.as_str()) =>
            {
                existing.extend(incoming.iter().cloned());
            }
            (existing, incoming) => {
                *existing = incoming.clone();
            }
        }
    }
}

fn reject_unresolved_include(
    value: &Value,
    pointer: &str,
    kind: CompositionKind,
    root_path: &Path,
) -> CliResult<()> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_pointer = format!("{pointer}/{}", escape_pointer(key));
                if key == "$include" {
                    return Err(CliError::new(
                        "include.unsupported_location",
                        EXIT_VALIDATION_FAILED,
                        format!("$include is not allowed at {child_pointer}"),
                    )
                    .with_pointer(child_pointer)
                    .with_hint(format!(
                        "Use $include only at the documented schema/spec composition points, then rerun `powerbi-cli {} ... --json`.",
                        kind.command()
                    ))
                    .with_suggested_command(format!(
                        "powerbi-cli {} {} --json",
                        kind.validation_command(),
                        command_arg(root_path)
                    )));
                }
                reject_unresolved_include(child, &child_pointer, kind, root_path)?;
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                reject_unresolved_include(child, &format!("{pointer}/{index}"), kind, root_path)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        scalar => scalar,
    }
}
