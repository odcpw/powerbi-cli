use crate::cli_support::{
    MutationMode, mode_name, require_mode, required_project, set_mode, shell_arg, take_value,
    target_project,
};
use crate::handoff::handoff_command;
use crate::lint::lint_project;
use crate::pbir_bookmarks::{bookmark_record_json, list_report_bookmarks};
use crate::pbir_filters::{
    FilterOwner, ReportFilterRecord, filter_record_json, filter_target, list_report_filters,
};
use crate::pbir_interactions::{interaction_record_json, list_report_interactions};
use crate::pbir_slicers::{
    ReportSlicerRecord, is_slicer_visual_type, list_report_slicers, slicer_record_json,
    slicer_state_summary_from_bindings,
};
use crate::project_io::write_json_atomic;
use crate::report_filter_mutations::{ensure_filter_path_under_report, filter_array_pointer};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SLICER_FILTER_POINTERS: [&str; 2] = ["/filterConfig/filters", "/filters"];

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum HygieneProfile {
    AgentSafe,
    Handoff,
}

impl HygieneProfile {
    fn as_str(self) -> &'static str {
        match self {
            Self::AgentSafe => "agent-safe",
            Self::Handoff => "handoff",
        }
    }
}

#[derive(Debug, Default)]
struct HygieneOptions {
    project: Option<PathBuf>,
    profile: Option<HygieneProfile>,
    include_raw: bool,
    confirm: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct HygienePlan {
    project_fingerprint: String,
    plan_fingerprint: String,
    profile: HygieneProfile,
    findings: Vec<Value>,
    actions: Vec<Value>,
    unsupported_actions: Vec<Value>,
}

struct FilterClearPlan {
    file_writes: Vec<(PathBuf, Value)>,
    changes: Vec<Value>,
    array_edits: Vec<Value>,
}

struct SlicerClearPlan {
    file_writes: Vec<(PathBuf, Value)>,
    changes: Vec<Value>,
    array_edits: Vec<Value>,
}

pub(crate) fn hygiene_command(command: &str, args: &[String]) -> CliResult<Value> {
    match command {
        "audit" => audit_command(args),
        "sanitize" => sanitize_command(args),
        _ => Err(
            CliError::invalid_args(format!("unknown report hygiene command: {command}"))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report audit\"`.")
                .with_suggested_command("powerbi-cli --json capabilities --for \"report audit\""),
        ),
    }
}

fn audit_command(args: &[String]) -> CliResult<Value> {
    let options = parse_hygiene_args("report audit", args)?;
    let project = required_project(options.project, "report audit")?;
    let profile = options.profile.unwrap_or(HygieneProfile::AgentSafe);
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let plan = build_hygiene_plan(&resolved, &validation, profile, options.include_raw)?;
    let ok = validation.errors.is_empty() && !has_error_findings(&plan.findings);
    let exit_code = if ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    Ok(json!({
        "schema": "powerbi-cli.report.audit.v1",
        "ok": ok,
        "exitCode": exit_code,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "profile": profile.as_str(),
        "rawIncluded": options.include_raw,
        "projectFingerprint": plan.project_fingerprint,
        "counts": counts_json(&plan.findings, &plan.actions, &plan.unsupported_actions),
        "findings": plan.findings,
        "recommendedActions": plan.actions,
        "unsupportedActions": plan.unsupported_actions,
        "next": [
            format!("powerbi-cli report sanitize plan --project {} --profile {} --json", command_arg(&resolved.project_dir), profile.as_str()),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli handoff check {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn sanitize_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(
            CliError::invalid_args("report sanitize requires plan or apply")
                .with_hint("Start with `report sanitize plan`.")
                .with_suggested_command(
                    "powerbi-cli report sanitize plan --project <project-dir-or.pbip> --json",
                ),
        );
    };
    match action.as_str() {
        "plan" => sanitize_plan_command(rest),
        "apply" => sanitize_apply_command(rest),
        other => Err(
            CliError::invalid_args(format!("unknown report sanitize action: {other}"))
                .with_hint("Use `plan` or `apply`.")
                .with_suggested_command(
                    "powerbi-cli report sanitize plan --project <project-dir-or.pbip> --json",
                ),
        ),
    }
}

fn sanitize_plan_command(args: &[String]) -> CliResult<Value> {
    let options = parse_hygiene_args("report sanitize plan", args)?;
    let project = required_project(options.project, "report sanitize plan")?;
    let profile = options.profile.unwrap_or(HygieneProfile::AgentSafe);
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let plan = build_hygiene_plan(&resolved, &validation, profile, false)?;
    Ok(plan_response(
        &resolved,
        &validation,
        &plan,
        "powerbi-cli.report.sanitize.plan.v1",
    ))
}

fn sanitize_apply_command(args: &[String]) -> CliResult<Value> {
    let options = parse_hygiene_args("report sanitize apply", args)?;
    let source_project = required_project(options.project, "report sanitize apply")?;
    let mode = require_mode(options.mode, "report sanitize apply")?;
    let profile = options.profile.unwrap_or(HygieneProfile::AgentSafe);
    let source_resolved = resolve_project(&source_project)?;
    let source_validation = validate_project(&source_resolved)?;
    let source_plan = build_hygiene_plan(&source_resolved, &source_validation, profile, false)?;
    let confirm_token = confirm_token(&source_plan.plan_fingerprint);

    if mode == MutationMode::InPlace && options.confirm.as_deref() != Some(&confirm_token) {
        return Err(CliError::invalid_args(format!(
            "in-place sanitize apply requires --confirm {confirm_token}"
        ))
        .with_hint("Run `report sanitize plan` first and confirm the exact token.")
        .with_suggested_command(format!(
            "powerbi-cli report sanitize apply --project {} --in-place --confirm {} --json",
            command_arg(&source_resolved.project_dir),
            shell_arg(&confirm_token)
        )));
    }

    crate::cli_support::preflight_out_dir(args, sanitize_apply_command)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let _target_validation = validate_project(&target_resolved)?;
    let action_handles = action_handle_sets(&source_plan.actions);
    let (mut filter_plan, filter_records) =
        clear_filter_records(&target_resolved, &action_handles.filter_handles)?;
    let (mut slicer_plan, slicer_records) =
        clear_slicer_records(&target_resolved, &action_handles.slicer_handles)?;

    if mode != MutationMode::DryRun {
        for (path, value) in filter_plan
            .file_writes
            .iter()
            .chain(slicer_plan.file_writes.iter())
        {
            write_json_atomic(path, value)?;
        }
    }

    let post_validation = if mode == MutationMode::DryRun {
        None
    } else {
        Some(validate_project(&target_resolved)?)
    };
    let post_audit = if mode == MutationMode::DryRun {
        Value::Null
    } else {
        let validation = post_validation.as_ref().expect("post validation");
        let plan = build_hygiene_plan(&target_resolved, validation, profile, false)?;
        json!({
            "ok": validation.errors.is_empty() && !has_error_findings(&plan.findings),
            "counts": counts_json(&plan.findings, &plan.actions, &plan.unsupported_actions),
            "projectFingerprint": plan.project_fingerprint,
            "planFingerprint": plan.plan_fingerprint
        })
    };
    let validation_ok = post_validation
        .as_ref()
        .map(|report| report.errors.is_empty())
        .unwrap_or(true);
    let ok = validation_ok;
    let exit_code = if ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let target_arg = command_arg(&target_resolved.project_dir);

    filter_plan.changes.append(&mut slicer_plan.changes);
    filter_plan.array_edits.append(&mut slicer_plan.array_edits);

    Ok(json!({
        "schema": "powerbi-cli.report.sanitize.apply.v1",
        "ok": ok,
        "exitCode": exit_code,
        "dryRun": mode == MutationMode::DryRun,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&source_resolved.project_dir),
        "targetProjectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "profile": profile.as_str(),
        "projectFingerprint": source_plan.project_fingerprint,
        "planFingerprint": source_plan.plan_fingerprint,
        "confirmToken": confirm_token,
        "counts": {
            "plannedActions": source_plan.actions.len(),
            "supportedActions": action_handles.filter_handles.len() + action_handles.slicer_handles.len(),
            "unsupportedActions": source_plan.unsupported_actions.len(),
            "clearedFilters": filter_records.len(),
            "clearedSlicers": slicer_records.len(),
            "changedFiles": filter_plan.file_writes.len() + slicer_plan.file_writes.len(),
            "changes": filter_plan.changes.len()
        },
        "actions": source_plan.actions,
        "unsupportedActions": source_plan.unsupported_actions,
        "changes": filter_plan.changes,
        "arrayEdits": filter_plan.array_edits,
        "validation": post_validation.map(validation_json),
        "postAudit": post_audit,
        "readbackCommand": format!("powerbi-cli report audit --project {target_arg} --profile {} --json", profile.as_str()),
        "validateCommand": format!("powerbi-cli validate --strict {target_arg} --json"),
        "next": [
            format!("powerbi-cli report audit --project {target_arg} --profile {} --json", profile.as_str()),
            format!("powerbi-cli report tree --project {target_arg} --json"),
            format!("powerbi-cli validate --strict {target_arg} --json")
        ]
    }))
}

fn parse_hygiene_args(command: &str, args: &[String]) -> CliResult<HygieneOptions> {
    let mut options = HygieneOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--profile" => {
                options.profile = Some(parse_profile(&take_value(args, &mut i, "--profile")?)?);
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun, command)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace, command)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                options.out_dir = Some(PathBuf::from(take_value(args, &mut i, "--out-dir")?));
                set_mode(&mut options.mode, MutationMode::OutDir, command)?;
            }
            other if !other.starts_with('-') && options.project.is_none() => {
                options.project = Some(PathBuf::from(other));
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!("unknown {command} flag: {other}"))
                    .with_hint(format!(
                        "Run `powerbi-cli --json capabilities --for \"{command}\"` for exact usage."
                    ))
                    .with_suggested_command(format!(
                        "powerbi-cli --json capabilities --for \"{command}\""
                    )));
            }
        }
    }
    Ok(options)
}

fn parse_profile(value: &str) -> CliResult<HygieneProfile> {
    match value {
        "agent-safe" | "agent" => Ok(HygieneProfile::AgentSafe),
        "handoff" | "work-handoff" => Ok(HygieneProfile::Handoff),
        other => Err(CliError::invalid_args(format!(
            "invalid report hygiene profile: {other}"
        ))
        .with_hint("Use --profile agent-safe or --profile handoff.")
        .with_suggested_command(
            "powerbi-cli report audit --project <project-dir-or.pbip> --profile agent-safe --json",
        )),
    }
}

fn build_hygiene_plan(
    resolved: &ResolvedProject,
    validation: &crate::ValidationReport,
    profile: HygieneProfile,
    include_raw: bool,
) -> CliResult<HygienePlan> {
    let mut findings = Vec::new();
    add_lint_findings(resolved, validation, &mut findings)?;
    add_handoff_findings(resolved, profile, &mut findings)?;

    let (filters, _) = list_report_filters(resolved)?;
    let (slicers, _) = list_report_slicers(resolved)?;
    let slicer_visual_handles = slicers
        .iter()
        .map(|record| record.visual_handle.clone())
        .collect::<BTreeSet<_>>();
    for record in &filters {
        if record.may_contain_data_values {
            findings.push(filter_finding(record, include_raw));
        }
    }
    for record in &slicers {
        if record.may_contain_data_values {
            findings.push(slicer_finding(record, include_raw));
        }
    }

    let (bookmarks, _, _) = list_report_bookmarks(resolved)?;
    for record in &bookmarks {
        if record.may_contain_data_values || record.unsupported {
            findings.push(json!({
                "id": format!("bookmark:{}", record.handle),
                "ruleId": "bookmark.possible_persisted_state",
                "severity": "warning",
                "risk": "possible-data-value-or-unsupported-state",
                "surface": "bookmark",
                "handle": record.handle,
                "path": canonical_display(&record.path),
                "jsonPointer": "",
                "fingerprint": record.fingerprint,
                "mayContainDataValues": record.may_contain_data_values,
                "literalCount": record.literal_count,
                "message": "Bookmark state can persist captured report state and may include data values; safe redaction is plan-only until Desktop fixture-proven.",
                "evidence": bookmark_record_json(record, include_raw),
                "recommendedActions": ["review-bookmark-state"]
            }));
        }
    }

    let (interactions, _) = list_report_interactions(resolved)?;
    for record in &interactions {
        if record.unsupported || record.source_visual.is_none() || record.target_visual.is_none() {
            findings.push(json!({
                "id": format!("interaction:{}", record.handle),
                "ruleId": "interaction.unsupported_or_stale",
                "severity": "warning",
                "risk": "stale-or-unsupported-report-behavior",
                "surface": "interaction",
                "handle": record.handle,
                "path": canonical_display(&record.path),
                "jsonPointer": record.json_pointer,
                "fingerprint": record.fingerprint,
                "mayContainDataValues": false,
                "literalCount": 0,
                "message": "Visual interaction override is unsupported or references missing visuals; review before handoff.",
                "evidence": interaction_record_json(record, include_raw),
                "recommendedActions": ["review-interaction"]
            }));
        }
    }

    let mut actions = Vec::new();
    let mut unsupported_actions = Vec::new();
    for record in filters.iter().filter(|record| {
        record.may_contain_data_values && !filter_owned_by_slicer(record, &slicer_visual_handles)
    }) {
        actions.push(json!({
            "actionId": format!("sanitize:filter:{}", record.handle),
            "kind": "clear-filter-values",
            "applySupported": true,
            "status": "ready",
            "handles": [record.handle],
            "paths": [canonical_display(&record.path)],
            "jsonPointers": [record.json_pointer],
            "beforeSummary": filter_record_json(record, false),
            "afterSummary": Value::Null,
            "blockedReason": Value::Null,
            "sourceRuleIds": ["filter.possible_persisted_values"]
        }));
    }
    for record in slicers
        .iter()
        .filter(|record| record.may_contain_data_values)
    {
        actions.push(json!({
            "actionId": format!("sanitize:slicer:{}", record.handle),
            "kind": "clear-slicer-selections",
            "applySupported": true,
            "status": "ready",
            "handles": [record.handle],
            "paths": [record.path.as_ref().map(|path| canonical_display(path))],
            "jsonPointers": SLICER_FILTER_POINTERS,
            "beforeSummary": slicer_record_json(record, false),
            "afterSummary": Value::Null,
            "blockedReason": Value::Null,
            "sourceRuleIds": ["slicer.possible_persisted_values"]
        }));
    }
    for finding in &findings {
        if finding["ruleId"] == "bookmark.possible_persisted_state" {
            unsupported_actions.push(plan_only_action(
                "review-bookmark-state",
                finding,
                "bookmark state redaction requires Desktop-backed golden fixtures before mutation",
            ));
        } else if finding["ruleId"] == "interaction.unsupported_or_stale" {
            unsupported_actions.push(plan_only_action(
                "review-interaction",
                finding,
                "interaction repair requires visual-specific intent; use report interactions commands explicitly",
            ));
        }
    }

    let project_fingerprint = project_fingerprint(resolved)?;
    let plan_fingerprint = plan_fingerprint(
        profile,
        &project_fingerprint,
        &actions,
        &unsupported_actions,
    )?;
    Ok(HygienePlan {
        project_fingerprint,
        plan_fingerprint,
        profile,
        findings,
        actions,
        unsupported_actions,
    })
}

fn add_lint_findings(
    resolved: &ResolvedProject,
    validation: &crate::ValidationReport,
    findings: &mut Vec<Value>,
) -> CliResult<()> {
    let lint = lint_project(resolved, validation)?;
    if let Some(items) = lint["findings"].as_array() {
        for (index, finding) in items.iter().enumerate() {
            findings.push(json!({
                "id": format!("lint:{index}"),
                "ruleId": finding["code"],
                "severity": finding["severity"],
                "risk": "project-quality",
                "surface": "lint",
                "handle": finding["handle"].clone(),
                "path": finding["path"].clone(),
                "jsonPointer": Value::Null,
                "fingerprint": Value::Null,
                "mayContainDataValues": false,
                "literalCount": 0,
                "message": finding["message"],
                "evidence": finding,
                "recommendedActions": ["fix-lint-finding"]
            }));
        }
    }
    Ok(())
}

fn add_handoff_findings(
    resolved: &ResolvedProject,
    profile: HygieneProfile,
    findings: &mut Vec<Value>,
) -> CliResult<()> {
    if profile != HygieneProfile::Handoff {
        return Ok(());
    }
    let handoff = handoff_command(&[
        "check".to_string(),
        canonical_display(&resolved.project_dir),
    ])?;
    if let Some(items) = handoff["findings"].as_array() {
        for (index, finding) in items.iter().enumerate() {
            findings.push(json!({
                "id": format!("handoff:{index}"),
                "ruleId": finding["code"],
                "severity": finding["severity"],
                "risk": "offline-handoff",
                "surface": "handoff",
                "handle": finding["handle"].clone(),
                "path": finding["path"].clone(),
                "jsonPointer": Value::Null,
                "fingerprint": Value::Null,
                "mayContainDataValues": false,
                "literalCount": 0,
                "message": finding["message"],
                "evidence": finding,
                "recommendedActions": ["fix-handoff-finding"]
            }));
        }
    }
    Ok(())
}

fn filter_finding(record: &ReportFilterRecord, include_raw: bool) -> Value {
    json!({
        "id": format!("filter:{}", record.handle),
        "ruleId": "filter.possible_persisted_values",
        "severity": "warning",
        "risk": "possible-persisted-data-values",
        "surface": "filter",
        "handle": record.handle,
        "owner": filter_owner_json(&record.owner),
        "path": canonical_display(&record.path),
        "jsonPointer": record.json_pointer,
        "fingerprint": record.fingerprint,
        "mayContainDataValues": record.may_contain_data_values,
        "literalCount": record.literal_count,
        "message": "Filter metadata can persist selected data values; sanitize can remove this filter entry.",
        "evidence": filter_record_json(record, include_raw),
        "recommendedActions": ["clear-filter-values"]
    })
}

fn slicer_finding(record: &ReportSlicerRecord, include_raw: bool) -> Value {
    json!({
        "id": format!("slicer:{}", record.handle),
        "ruleId": "slicer.possible_persisted_values",
        "severity": "warning",
        "risk": "possible-persisted-data-values",
        "surface": "slicer",
        "handle": record.handle,
        "owner": {
            "visualHandle": record.visual_handle,
            "pageHandle": record.page_handle
        },
        "path": record.path.as_ref().map(|path| canonical_display(path)),
        "jsonPointer": Value::Null,
        "fingerprint": record.fingerprint,
        "mayContainDataValues": record.may_contain_data_values,
        "literalCount": record.literal_count,
        "message": "Slicer metadata can persist selected data values; sanitize can clear matching slicer filter state.",
        "evidence": slicer_record_json(record, include_raw),
        "recommendedActions": ["clear-slicer-selections"]
    })
}

fn filter_owner_json(owner: &FilterOwner) -> Value {
    match owner {
        FilterOwner::Report { .. } => json!({"kind": "report", "handle": "report:main"}),
        FilterOwner::Page {
            handle,
            name,
            display_name,
            ..
        } => json!({"kind": "page", "handle": handle, "name": name, "displayName": display_name}),
        FilterOwner::Visual {
            handle,
            name,
            title,
            visual_type,
            page_handle,
            ..
        } => json!({
            "kind": "visual",
            "handle": handle,
            "name": name,
            "title": title,
            "visualType": visual_type,
            "pageHandle": page_handle
        }),
    }
}

fn filter_owned_by_slicer(
    record: &ReportFilterRecord,
    slicer_visual_handles: &BTreeSet<String>,
) -> bool {
    matches!(
        &record.owner,
        FilterOwner::Visual {
            handle,
            visual_type,
            ..
        } if slicer_visual_handles.contains(handle) || is_slicer_visual_type(visual_type)
    )
}

fn plan_only_action(kind: &str, finding: &Value, blocked_reason: &str) -> Value {
    json!({
        "actionId": format!("sanitize:{}:{}", kind, finding["handle"].as_str().unwrap_or("unknown")),
        "kind": kind,
        "applySupported": false,
        "status": "blocked",
        "handles": [finding["handle"].clone()],
        "paths": [finding["path"].clone()],
        "jsonPointers": [finding["jsonPointer"].clone()],
        "beforeSummary": finding["evidence"].clone(),
        "afterSummary": Value::Null,
        "blockedReason": blocked_reason,
        "sourceRuleIds": [finding["ruleId"].clone()]
    })
}

fn plan_response(
    resolved: &ResolvedProject,
    validation: &crate::ValidationReport,
    plan: &HygienePlan,
    schema: &str,
) -> Value {
    let ok = validation.errors.is_empty() && !has_error_findings(&plan.findings);
    let exit_code = if ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let confirm_token = confirm_token(&plan.plan_fingerprint);
    json!({
        "schema": schema,
        "ok": ok,
        "exitCode": exit_code,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "profile": plan.profile.as_str(),
        "projectFingerprint": plan.project_fingerprint,
        "planFingerprint": plan.plan_fingerprint,
        "confirmToken": confirm_token,
        "counts": counts_json(&plan.findings, &plan.actions, &plan.unsupported_actions),
        "findings": plan.findings,
        "actions": plan.actions,
        "unsupportedActions": plan.unsupported_actions,
        "next": [
            format!("powerbi-cli report sanitize apply --project {} --dry-run --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report sanitize apply --project {} --out-dir <sanitized-project-dir> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report sanitize apply --project {} --in-place --confirm {} --json", command_arg(&resolved.project_dir), shell_arg(&confirm_token))
        ]
    })
}

fn has_error_findings(findings: &[Value]) -> bool {
    findings
        .iter()
        .any(|finding| finding["severity"].as_str() == Some("error"))
}

fn counts_json(findings: &[Value], actions: &[Value], unsupported_actions: &[Value]) -> Value {
    let mut by_rule = Map::new();
    let mut by_severity = Map::new();
    for finding in findings {
        if let Some(rule) = finding["ruleId"].as_str() {
            increment(&mut by_rule, rule);
        }
        if let Some(severity) = finding["severity"].as_str() {
            increment(&mut by_severity, severity);
        }
    }
    json!({
        "findings": findings.len(),
        "actions": actions.len(),
        "supportedActions": actions.iter().filter(|action| action["applySupported"].as_bool() == Some(true)).count(),
        "unsupportedActions": unsupported_actions.len(),
        "byRule": by_rule,
        "bySeverity": by_severity
    })
}

fn increment(map: &mut Map<String, Value>, key: &str) {
    let next = map.get(key).and_then(Value::as_u64).unwrap_or(0) + 1;
    map.insert(key.to_string(), Value::from(next));
}

struct ActionHandleSets {
    filter_handles: BTreeSet<String>,
    slicer_handles: BTreeSet<String>,
}

fn action_handle_sets(actions: &[Value]) -> ActionHandleSets {
    let mut filter_handles = BTreeSet::new();
    let mut slicer_handles = BTreeSet::new();
    for action in actions {
        let Some(kind) = action["kind"].as_str() else {
            continue;
        };
        let handles = action["handles"].as_array().into_iter().flatten();
        for handle in handles.filter_map(Value::as_str) {
            match kind {
                "clear-filter-values" => {
                    filter_handles.insert(handle.to_string());
                }
                "clear-slicer-selections" => {
                    slicer_handles.insert(handle.to_string());
                }
                _ => {}
            }
        }
    }
    ActionHandleSets {
        filter_handles,
        slicer_handles,
    }
}

fn clear_filter_records(
    resolved: &ResolvedProject,
    handles: &BTreeSet<String>,
) -> CliResult<(FilterClearPlan, Vec<ReportFilterRecord>)> {
    let (records, _) = list_report_filters(resolved)?;
    let targets = records
        .into_iter()
        .filter(|record| handles.contains(&record.handle))
        .collect::<Vec<_>>();
    for record in &targets {
        ensure_filter_path_under_report(resolved, record)?;
    }
    let mut by_path: BTreeMap<PathBuf, BTreeMap<String, Vec<ReportFilterRecord>>> = BTreeMap::new();
    for record in &targets {
        let (parent_pointer, _) = filter_array_pointer(&record.json_pointer)?;
        by_path
            .entry(record.path.clone())
            .or_default()
            .entry(parent_pointer)
            .or_default()
            .push(record.clone());
    }

    let mut file_writes = Vec::new();
    let mut changes = Vec::new();
    let mut array_edits = Vec::new();
    for (path, by_pointer) in by_path {
        let mut file_json = read_json_value(&path)?;
        for (parent_pointer, group) in by_pointer {
            let mut ordinals = group
                .iter()
                .map(|record| {
                    filter_array_pointer(&record.json_pointer).map(|(_, ordinal)| ordinal)
                })
                .collect::<CliResult<Vec<_>>>()?;
            ordinals.sort_unstable();
            let unique_ordinals = ordinals.iter().copied().collect::<BTreeSet<_>>();
            let items = file_json
                .pointer_mut(&parent_pointer)
                .and_then(Value::as_array_mut)
                .ok_or_else(|| {
                    CliError::validation_failed(format!(
                        "{} filter array is missing or not an array at {parent_pointer}",
                        path.display()
                    ))
                })?;
            let before_count = items.len();
            for ordinal in unique_ordinals.iter().rev() {
                if *ordinal >= items.len() {
                    return Err(CliError::validation_failed(format!(
                        "{} filter index {ordinal} is outside array {parent_pointer}",
                        path.display()
                    )));
                }
                items.remove(*ordinal);
            }
            let after_count = items.len();
            array_edits.push(json!({
                "kind": "filter-array",
                "path": canonical_display(&path),
                "parentJsonPointer": parent_pointer,
                "ordinals": unique_ordinals.iter().copied().collect::<Vec<_>>(),
                "arrayBeforeCount": before_count,
                "arrayAfterCount": after_count
            }));
            for record in group {
                changes.push(json!({
                    "kind": "pbir.filter",
                    "action": "clear",
                    "path": canonical_display(&record.path),
                    "handle": record.handle,
                    "jsonPointer": record.json_pointer,
                    "parentJsonPointer": parent_pointer,
                    "before": filter_record_json(&record, false),
                    "after": Value::Null
                }));
            }
        }
        file_writes.push((path, file_json));
    }

    Ok((
        FilterClearPlan {
            file_writes,
            changes,
            array_edits,
        },
        targets,
    ))
}

fn clear_slicer_records(
    resolved: &ResolvedProject,
    handles: &BTreeSet<String>,
) -> CliResult<(SlicerClearPlan, Vec<ReportSlicerRecord>)> {
    let (records, _) = list_report_slicers(resolved)?;
    let targets = records
        .into_iter()
        .filter(|record| handles.contains(&record.handle))
        .collect::<Vec<_>>();
    let mut file_writes = Vec::new();
    let mut changes = Vec::new();
    let mut array_edits = Vec::new();
    for record in &targets {
        let path = record.path.as_ref().ok_or_else(|| {
            CliError::validation_failed(format!(
                "slicer has no backing visual path: {}",
                record.handle
            ))
        })?;
        ensure_slicer_path_under_report(resolved, path)?;
        let mut file_json = read_json_value(path)?;
        let before_state = slicer_state_summary_from_bindings(&record.bindings, Some(&file_json));
        for pointer in SLICER_FILTER_POINTERS {
            let Some(array) = file_json.pointer_mut(pointer).and_then(Value::as_array_mut) else {
                continue;
            };
            let before_count = array.len();
            let removed = remove_matching_slicer_filters(array, &record.bindings);
            let after_count = array.len();
            if !removed.is_empty() {
                array_edits.push(json!({
                    "kind": "slicer-filter-array",
                    "path": canonical_display(path),
                    "handle": record.handle,
                    "parentJsonPointer": pointer,
                    "removedOrdinals": removed.iter().map(|(ordinal, _)| *ordinal).collect::<Vec<_>>(),
                    "arrayBeforeCount": before_count,
                    "arrayAfterCount": after_count
                }));
                for (ordinal, removed_value) in removed {
                    changes.push(json!({
                        "kind": "pbir.slicerState",
                        "action": "clear",
                        "path": canonical_display(path),
                        "handle": record.handle,
                        "visualHandle": record.visual_handle,
                        "jsonPointer": format!("{pointer}/{ordinal}"),
                        "parentJsonPointer": pointer,
                        "before": {
                            "ordinal": ordinal,
                            "filterTarget": filter_target(&removed_value)
                        },
                        "after": Value::Null
                    }));
                }
            }
        }
        let after_state = slicer_state_summary_from_bindings(&record.bindings, Some(&file_json));
        changes.push(json!({
            "kind": "pbir.slicerStateSummary",
            "action": "summarize",
            "path": canonical_display(path),
            "handle": record.handle,
            "visualHandle": record.visual_handle,
            "before": before_state,
            "after": after_state
        }));
        file_writes.push((path.clone(), file_json));
    }
    Ok((
        SlicerClearPlan {
            file_writes,
            changes,
            array_edits,
        },
        targets,
    ))
}

fn ensure_slicer_path_under_report(resolved: &ResolvedProject, path: &Path) -> CliResult<()> {
    let file_name = path.file_name().and_then(|value| value.to_str());
    if !matches!(file_name, Some("visual.json")) {
        return Err(CliError::validation_failed(format!(
            "refusing to mutate slicer from unsupported file path: {}",
            path.display()
        )));
    }
    let report_abs = fs::canonicalize(&resolved.report_dir).map_err(|err| {
        CliError::unexpected(format!("resolve {}: {err}", resolved.report_dir.display()))
    })?;
    let path_abs = fs::canonicalize(path)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", path.display())))?;
    if path_abs.starts_with(report_abs) {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "refusing to mutate slicer outside report directory: {}",
        path.display()
    )))
}

fn remove_matching_slicer_filters(
    array: &mut Vec<Value>,
    bindings: &[Value],
) -> Vec<(usize, Value)> {
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for (ordinal, item) in array.drain(..).enumerate() {
        if filter_matches_slicer_binding(&item, bindings) {
            removed.push((ordinal, item));
        } else {
            kept.push(item);
        }
    }
    *array = kept;
    removed
}

fn filter_matches_slicer_binding(filter: &Value, bindings: &[Value]) -> bool {
    let target = filter_target(filter);
    bindings
        .iter()
        .any(|binding| target_matches_binding(&target, binding))
}

fn target_matches_binding(target: &Value, binding: &Value) -> bool {
    let Some(kind) = target["kind"].as_str() else {
        return false;
    };
    if !string_eq(target["table"].as_str(), binding["table"].as_str()) {
        return false;
    }
    match kind {
        "column" => string_eq(
            target["column"]
                .as_str()
                .or_else(|| target["field"].as_str()),
            binding["column"]
                .as_str()
                .or_else(|| binding["field"].as_str()),
        ),
        "measure" => string_eq(
            target["measure"]
                .as_str()
                .or_else(|| target["field"].as_str()),
            binding["measure"]
                .as_str()
                .or_else(|| binding["field"].as_str()),
        ),
        _ => false,
    }
}

fn string_eq(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => false,
    }
}

fn project_fingerprint(resolved: &ResolvedProject) -> CliResult<String> {
    let mut inputs = Vec::new();
    for root in [&resolved.report_dir, &resolved.semantic_model_dir] {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root) {
            let entry =
                crate::walkdir_entry(root, entry, "walk report hygiene fingerprint inputs")?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let extension = path.extension().and_then(|value| value.to_str());
            if !matches!(extension, Some("json" | "tmdl" | "pbir")) {
                continue;
            }
            let relative = path
                .strip_prefix(&resolved.project_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(path)
                .map_err(|err| CliError::unexpected(format!("read {}: {err}", path.display())))?;
            inputs.push((relative, bytes));
        }
    }
    inputs.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hash = 0xcbf29ce484222325u64;
    for (relative, bytes) in inputs {
        fnv_update(&mut hash, relative.as_bytes());
        fnv_update(&mut hash, b"\0");
        fnv_update(&mut hash, &bytes);
        fnv_update(&mut hash, b"\0");
    }
    Ok(format!("fnv64:{hash:016x}"))
}

fn plan_fingerprint(
    profile: HygieneProfile,
    project_fingerprint: &str,
    actions: &[Value],
    unsupported_actions: &[Value],
) -> CliResult<String> {
    let text = serde_json::to_string(&json!({
        "profile": profile.as_str(),
        "projectFingerprint": project_fingerprint,
        "actions": actions,
        "unsupportedActions": unsupported_actions
    }))
    .map_err(|err| CliError::unexpected(format!("serialize sanitize plan fingerprint: {err}")))?;
    let mut hash = 0xcbf29ce484222325u64;
    fnv_update(&mut hash, text.as_bytes());
    Ok(format!("fnv64:{hash:016x}"))
}

fn fnv_update(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn confirm_token(plan_fingerprint: &str) -> String {
    format!("sanitize:{plan_fingerprint}")
}

fn validation_json(report: crate::ValidationReport) -> Value {
    json!({
        "ok": report.errors.is_empty(),
        "warnings": report.warnings,
        "errors": report.errors,
        "counts": {
            "jsonFilesChecked": report.json_files_checked,
            "tables": report.tables,
            "relationships": report.relationships,
            "measures": report.measures,
            "pages": report.pages,
            "visuals": report.visuals,
            "boundVisuals": report.bound_visuals
        }
    })
}
