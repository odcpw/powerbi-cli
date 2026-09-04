use crate::desktop_proof::{LoadedDesktopProofRecord, ProofLevel, embedded_desktop_proof_records};
use crate::{CliError, CliResult};
use serde_json::{Value, json};

pub(crate) fn features_command(args: &[String]) -> CliResult<Value> {
    let (action, rest) = match args.split_first() {
        Some((action, rest)) if matches!(action.as_str(), "list" | "ls" | "catalog") => {
            (action.as_str(), rest)
        }
        Some((action, _)) if action.starts_with('-') => ("list", args),
        Some((action, _)) => {
            return Err(
                CliError::invalid_args(format!("unknown features command: {action}"))
                    .with_hint("Run `powerbi-cli features list --json`.")
                    .with_suggested_command("powerbi-cli features list --json"),
            );
        }
        None => ("list", args),
    };

    match action {
        "list" | "ls" | "catalog" => list_features(rest),
        _ => unreachable!("features command action is normalized above"),
    }
}

pub(crate) fn unsupported_feature_error(feature_id: &str) -> CliError {
    let feature = feature_by_id(feature_id);
    let title = feature
        .as_ref()
        .map(|feature| feature.title)
        .unwrap_or(feature_id);
    let reason = feature
        .as_ref()
        .map(|feature| feature.reason)
        .unwrap_or("This feature is not implemented or proven in powerbi-cli.");
    CliError::unsupported_feature(format!("unsupported feature {feature_id}: {title}"))
        .with_hint(format!(
            "{reason} Supported features emit real PBIP/PBIR/TMDL shapes; unproven features fail instead of falling back."
        ))
        .with_suggested_command(format!(
            "powerbi-cli features list --for {feature_id} --json"
        ))
        .with_suggested_command("powerbi-cli --json capabilities")
}

pub(crate) fn unsupported_feature_error_with_message(
    feature_id: &str,
    message: impl Into<String>,
) -> CliError {
    let mut err = unsupported_feature_error(feature_id);
    err.message = message.into();
    err
}

pub(crate) fn feature_policy_json() -> Value {
    json!({
        "noFakeFallbacks": true,
        "supportedMeans": "The CLI emits a typed PBIP/PBIR/TMDL shape covered by local tests. proofLevel is authoritative for compatibility strength; desktop-golden-pending explicitly means Desktop compatibility is not yet claimed.",
        "unsupportedMeans": "The CLI returns error.code=unsupported_feature and does not write partial report/model output.",
        "discoveryCommand": "powerbi-cli features list --json",
        "proofEscalation": "Use fixture normalize/verify for deterministic goldens and Desktop oracle commands only when Desktop compatibility is the claim."
    })
}

pub(crate) fn feature_catalog_schema_fields() -> Vec<&'static str> {
    vec![
        "id",
        "title",
        "category",
        "status",
        "support",
        "proofLevel",
        "runtimeFallback",
        "emitsPbir",
        "commands",
        "refusalCode",
        "reason",
        "nextProof",
        "supportedKinds",
        "referenceSignals",
        "proofRecords",
    ]
}

fn list_features(args: &[String]) -> CliResult<Value> {
    let filter = parse_filter(args)?;
    let proof_records = embedded_desktop_proof_records()?;
    validate_proof_record_feature_ids(&proof_records)?;
    let mut features = FEATURE_CATALOG
        .iter()
        .filter(|feature| {
            filter
                .as_deref()
                .is_none_or(|filter| feature_matches(feature, filter))
        })
        .map(|feature| feature_json(feature, &proof_records))
        .collect::<Vec<_>>();
    features.sort_by_key(|feature| feature["id"].as_str().unwrap_or_default().to_string());
    let supported_count = features
        .iter()
        .filter(|feature| feature["status"] == "supported")
        .count();
    let unsupported_count = features.len().saturating_sub(supported_count);

    Ok(json!({
        "schema": "powerbi-cli.features.v1",
        "policy": feature_policy_json(),
        "filter": filter,
        "matchedFeatures": features.len(),
        "counts": {
            "supported": supported_count,
            "unsupportedOrPlanned": unsupported_count
        },
        "features": features,
        "next": [
            "powerbi-cli --json capabilities",
            "powerbi-cli report visuals catalog --json",
            "powerbi-cli fixture normalize <project-dir-or.pbip> --json"
        ]
    }))
}

fn parse_filter(args: &[String]) -> CliResult<Option<String>> {
    let mut filter = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--for" => {
                filter = Some(
                    args.get(i + 1)
                        .ok_or_else(|| CliError::invalid_args("--for requires a value"))?
                        .to_ascii_lowercase(),
                );
                i += 2;
            }
            other => {
                return Err(
                    CliError::invalid_args(format!("unknown features list flag: {other}"))
                        .with_hint("Run `powerbi-cli features list --json`.")
                        .with_suggested_command("powerbi-cli features list --json"),
                );
            }
        }
    }
    Ok(filter)
}

fn feature_matches(feature: &Feature, filter: &str) -> bool {
    feature.id.contains(filter)
        || feature.title.to_ascii_lowercase().contains(filter)
        || feature.category.contains(filter)
        || feature.status.contains(filter)
        || feature.support.contains(filter)
        || feature.tags.iter().any(|tag| tag.contains(filter))
}

fn feature_by_id(feature_id: &str) -> Option<Feature> {
    FEATURE_CATALOG
        .iter()
        .copied()
        .find(|feature| feature.id == feature_id)
}

fn feature_json(feature: &Feature, proof_records: &[LoadedDesktopProofRecord]) -> Value {
    let linked_records = proof_records
        .iter()
        .filter(|loaded| {
            loaded
                .record
                .signals
                .feature_ids
                .iter()
                .any(|feature_id| feature_id == feature.id)
        })
        .collect::<Vec<_>>();
    let proof_level = linked_records.iter().fold(
        ProofLevel::parse(feature.proof_level).expect("catalog proof levels are closed"),
        |level, loaded| level.max(loaded.record.proof_level),
    );
    let proof_records_json = linked_records
        .iter()
        .map(|loaded| {
            json!({
                "path": loaded.source,
                "fixture": loaded.record.fixture,
                "date": loaded.record.date,
                "desktopVersion": loaded.record.desktop_version,
                "proofLevel": loaded.record.proof_level.as_str()
            })
        })
        .collect::<Vec<_>>();
    json!({
        "id": feature.id,
        "title": feature.title,
        "category": feature.category,
        "status": feature.status,
        "support": feature.support,
        "proofLevel": proof_level.as_str(),
        "runtimeFallback": false,
        "emitsPbir": feature.emits_pbir,
        "commands": feature.commands,
        "refusalCode": feature.refusal_code,
        "reason": feature.reason,
        "nextProof": feature.next_proof,
        "supportedKinds": supported_kinds(feature),
        "referenceSignals": feature.reference_signals,
        "proofRecords": proof_records_json,
        "tags": feature.tags
    })
}

fn validate_proof_record_feature_ids(proof_records: &[LoadedDesktopProofRecord]) -> CliResult<()> {
    for loaded in proof_records {
        for feature_id in &loaded.record.signals.feature_ids {
            if feature_by_id(feature_id).is_none() {
                return Err(CliError::validation_failed(format!(
                    "invalid embedded Desktop proof record {}: signals.featureIds contains unknown feature `{feature_id}`",
                    loaded.source
                ))
                .with_hint(
                    "Use an exact features[].id from `powerbi-cli features list --json`; proof records never create feature catalog entries.",
                )
                .with_suggested_command("powerbi-cli features list --json"));
            }
        }
    }
    Ok(())
}

fn supported_kinds(feature: &Feature) -> &'static [&'static str] {
    match feature.id {
        "model.source-templates" => &[
            "sql",
            "postgres",
            "odbc",
            "excel",
            "csv",
            "folder",
            "sharepoint",
        ],
        _ => &[],
    }
}

#[derive(Debug, Clone, Copy)]
struct Feature {
    id: &'static str,
    title: &'static str,
    category: &'static str,
    status: &'static str,
    support: &'static str,
    proof_level: &'static str,
    emits_pbir: bool,
    commands: &'static [&'static str],
    refusal_code: Option<&'static str>,
    reason: &'static str,
    next_proof: &'static [&'static str],
    reference_signals: &'static [&'static str],
    tags: &'static [&'static str],
}

const FEATURE_CATALOG: &[Feature] = &[
    Feature {
        id: "quality.lint-rule-registry",
        title: "Discoverable lint and audit rule registry",
        category: "validation",
        status: "supported",
        support: "read-only-contract-catalog",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &["lint", "model dax lint", "report audit"],
        refusal_code: None,
        reason: "Every lint and report-audit finding id is registered with family, severity, summary, remediation, optional sanitize action, and version metadata; agents can list or explain rules without opening a project. M lint includes the error-level m.duplicate_step_name check for duplicate let identifiers.",
        next_proof: &[
            "Extend the typed design family when design lint ships without adding ad-hoc ids",
        ],
        reference_signals: &[],
        tags: &["lint", "audit", "rules", "diagnostics", "agent"],
    },
    Feature {
        id: "validation.microsoft-report",
        title: "Official Microsoft report validation backend",
        category: "validation",
        status: "supported",
        support: "explicit-exact-official-validator",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &["validate"],
        refusal_code: None,
        reason: "The explicit microsoft-report and all backends run the exact installed Microsoft report-authoring validator without replacing or silently falling back to native validation.",
        next_proof: &[
            "Keep exact-package valid and invalid consumed-surface fixtures in normal CI",
            "Record validator disagreements without merging diagnostic provenance",
        ],
        reference_signals: &[
            "integrations/microsoft/package-lock.json: @microsoft/powerbi-report-authoring-cli 0.1.4",
            "testdata/conformance/microsoft/DISCREPANCIES.md",
        ],
        tags: &["microsoft", "report", "validation", "no-fallback"],
    },
    Feature {
        id: "integrations.microsoft-toolchain",
        title: "Exact optional Microsoft Power BI toolchain",
        category: "integration",
        status: "supported",
        support: "explicit-install-immutable-cache",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &["integrations status", "integrations install"],
        refusal_code: None,
        reason: "The CLI pins the complete Microsoft npm graph, launches no child for shallow readiness, and activates verified installs atomically only after explicit network consent.",
        next_proof: &[
            "Run exact-package conformance on every supported Node 20+ CI platform",
            "Escalate Modeling MCP protocol and Desktop Bridge live proof in their dedicated integration features",
        ],
        reference_signals: &[
            "integrations/microsoft/integration-lock.json",
            "integrations/microsoft/package-lock.json",
        ],
        tags: &["microsoft", "supply-chain", "offline", "immutable-cache"],
    },
    Feature {
        id: "desktop.window-evidence",
        title: "Managed Desktop sessions, window observation, and screenshot evidence",
        category: "desktop",
        status: "supported",
        support: "opt-in-window-observation-and-primary-display-capture",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[
            "desktop open",
            "desktop close",
            "desktop open-check",
            "desktop screenshot",
        ],
        refusal_code: None,
        reason: "On an opted-in Windows oracle machine, the CLI can open a PBIP project or PBIX document as exactly one interactive Desktop session, own it by exact PID and creation time, close it idempotently, enforce a launch/observation watchdog, observe a document-matched window title, and capture the primary display. PBIP retains strict source preflight; PBIX gets bounded archive/report/DataModel preflight. These signals are evidence only and do not inspect the report canvas or refresh state.",
        next_proof: &[
            "Detect unresolved Desktop issue dialogs and banners",
            "Detect a rendered non-blank report canvas",
            "Automate dummy-partition refresh before claiming desktop-canvas-refresh compatibility",
        ],
        reference_signals: &[
            "docs/pbir-desktop-oracle.md: proof levels and Desktop signal semantics",
        ],
        tags: &[
            "desktop",
            "oracle",
            "window",
            "title",
            "screenshot",
            "evidence",
        ],
    },
    Feature {
        id: "agent.codex-skill-distribution",
        title: "Self-contained Codex skill installation and verification",
        category: "agent",
        status: "supported",
        support: "embedded-install-and-hash-verification",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &["skill install", "skill status"],
        refusal_code: None,
        reason: "The canonical powerbi-cli skill and its runtime regression reference are embedded in the Rust binary. The CLI can install or repair only those owned files in the Codex global skill directory and verify their SHA-256 identities without Python, network access, or an external script.",
        next_proof: &[
            "Publish an installer smoke test for packaged release binaries",
            "Keep repository and embedded skill hashes pinned by unit tests",
        ],
        reference_signals: &["skills/powerbi-cli/SKILL.md"],
        tags: &["agent", "skill", "codex", "install", "no-python"],
    },
    Feature {
        id: "package.pbix-pbit-boundary",
        title: "PBIX/PBIT package boundary",
        category: "package",
        status: "supported",
        support: "inspect-safe-metadata-source-pack-work-pack-export-plan",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[
            "package inspect",
            "package extract",
            "package import",
            "package source-pack",
            "package work-pack",
            "package export-plan",
        ],
        refusal_code: None,
        reason: "The CLI can inspect package archives, extract/import actual source-like metadata entries, package dummy source projects, and package credential-free materialized live-source work variants; opaque binary PBIX/PBIT writing is refused with a Desktop handoff plan.",
        next_proof: &[
            "Add recent Microsoft sample package fixtures containing source entries where licensing permits",
            "Keep binary package export behind Desktop handoff until a documented writable format exists",
        ],
        reference_signals: &[
            "https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-templates: PBIT templates contain report pages, visuals, model, and query definitions without data",
        ],
        tags: &["pbix", "pbit", "package", "metadata", "desktop"],
    },
    Feature {
        id: "model.static-control-tables",
        title: "Small static selector and lookup tables",
        category: "model",
        status: "supported",
        support: "add-bounded-string-table",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &["model tables add-static"],
        refusal_code: None,
        reason: "The CLI can add a new small TMDL table backed by an inline generated #table partition: either a disconnected one-column selector or a 1-10 column lookup dimension with a unique first-column key. Relationships remain a separate reviewed mutation.",
        next_proof: &[
            "Desktop refresh and selector-slicer interaction fixture",
            "Separate typed table/column CRUD only after broader TMDL fixtures exist",
        ],
        reference_signals: &[],
        tags: &[
            "table",
            "static-table",
            "selector",
            "parameter",
            "semantic-model",
            "tmdl",
        ],
    },
    Feature {
        id: "desktop.dax-query-execution",
        title: "Bounded DAX query execution against an open Desktop model",
        category: "desktop",
        status: "supported",
        support: "explicit-opt-in-exact-document-read-only-query",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &["model dax execute"],
        refusal_code: None,
        reason: "On an explicitly opted-in Windows oracle machine, the CLI locates the exact already-open PBIP or PBIX document process and its child semantic-model engine, loads Desktop's bundled ADOMD client from a private temporary copy, and executes only EVALUATE or DEFINE ... EVALUATE query forms. Rows, cell text, query size, and runtime are bounded; the command never launches Desktop, writes the model, or returns the query text.",
        next_proof: &[
            "Keep a live synthetic Desktop smoke test in the release checklist across supported Desktop versions",
            "Add a documented remote XMLA engine only with credential-isolated real-service integration tests",
        ],
        reference_signals: &[
            "docs/pbir-desktop-oracle.md: bounded Desktop DAX query execution",
            "https://learn.microsoft.com/en-us/power-bi/transform-model/desktop-external-tools: Desktop hosts a local Analysis Services model and client libraries can execute DAX queries",
            "https://learn.microsoft.com/en-us/dax/dax-queries: DAX query syntax uses EVALUATE and optional DEFINE declarations",
            "https://learn.microsoft.com/en-us/dotnet/api/microsoft.analysisservices.adomdclient.adomdcommand.executereader: ADOMD ExecuteReader query API",
            "https://learn.microsoft.com/en-us/power-bi/developer/agentic/power-bi-desktop-bridge-overview: the preview IPC manifest currently covers state, screenshot, and reload rather than DAX execution",
        ],
        tags: &[
            "desktop",
            "oracle",
            "dax",
            "query",
            "adomd",
            "read-only",
            "data-read",
            "pbix",
        ],
    },
    Feature {
        id: "desktop.live-tmdl-export",
        title: "Read-only semantic-model TMDL export from an open Desktop document",
        category: "desktop",
        status: "supported",
        support: "explicit-opt-in-exact-document-read-only-mcp-export",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &["model live export-tmdl"],
        refusal_code: None,
        reason: "On an explicitly opted-in Windows oracle machine, the CLI reuses and revalidates the exact PBIP/PBIX document-to-process-to-engine creation/workspace/port identity, sends the pinned local Microsoft Modeling MCP one closed canonical localhost request in read-only mode, and exports TMDL into a fresh sibling quarantine. The MCP returns an opaque connection name rather than endpoint readback. Output is published only after bounded TMDL shape, UTF-8, link/reparse, and credential-like-text validation; report pages are not exported.",
        next_proof: &[
            "Keep a live synthetic PBIX export smoke test in the Windows release checklist",
            "Add full PBIP materialization only when report-definition extraction has an equally closed and validated source contract",
        ],
        reference_signals: &[
            "skills/powerbi-cli/references/desktop-runtime-regression.md: exact Desktop lifecycle and live-model regression loop",
            "https://github.com/microsoft/powerbi-modeling-mcp: Desktop/PBIP connections and TMDL modeling surface",
        ],
        tags: &[
            "desktop",
            "oracle",
            "modeling-mcp",
            "tmdl",
            "read-only",
            "model-read",
            "pbix",
            "pbip",
        ],
    },
    Feature {
        id: "model.measures",
        title: "DAX measures",
        category: "model",
        status: "supported",
        support: "read-write",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[
            "model measures list",
            "model measures show",
            "model measures add",
            "model measures update",
            "model measures delete",
        ],
        refusal_code: None,
        reason: "TMDL measure blocks are parsed and rewritten with guarded output modes; DAX engine semantics remain a Desktop concern.",
        next_proof: &["Desktop open/save fixture for richer Desktop-authored TMDL metadata"],
        reference_signals: &[],
        tags: &["dax", "semantic-model", "tmdl"],
    },
    Feature {
        id: "model.calculated-columns",
        title: "DAX calculated columns",
        category: "model",
        status: "supported",
        support: "read-write",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[
            "model calculated-columns list",
            "model calculated-columns show",
            "model calculated-columns add",
            "model calculated-columns update",
            "model calculated-columns delete",
        ],
        refusal_code: None,
        reason: "TMDL calculated-column blocks are parsed and rewritten with guarded output modes.",
        next_proof: &["Desktop refresh/open fixture for calculated column recomputation behavior"],
        reference_signals: &[],
        tags: &["dax", "semantic-model", "tmdl"],
    },
    Feature {
        id: "model.dax-static-analysis",
        title: "DAX dependency inventory and static lint",
        category: "model",
        status: "supported",
        support: "read-only-static-analysis",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[
            "model dax dependencies",
            "model dax lint",
            "model dax bridge-plan",
        ],
        refusal_code: None,
        reason: "The CLI extracts common DAX references and obvious dependency defects while explicitly reporting that Desktop/XMLA remains the DAX engine validation boundary.",
        next_proof: &[
            "Add external DAX engine bridge integration only with real parser/service proof",
        ],
        reference_signals: &[
            "https://learn.microsoft.com/en-us/power-bi/developer/agentic/semantic-model-authoring-skill-overview: semantic model authoring includes DAX and BPA checks, with MCP recommended for modeling operations",
        ],
        tags: &[
            "dax",
            "semantic-model",
            "lint",
            "dependencies",
            "no-fallback",
        ],
    },
    Feature {
        id: "model.advanced-readback",
        title: "Advanced semantic model TMDL readback",
        category: "model",
        status: "supported",
        support: "read-only",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[
            "model advanced inventory",
            "model roles list",
            "model roles show",
            "model perspectives list",
            "model perspectives show",
            "model cultures list",
            "model cultures show",
            "model expressions list",
            "model expressions show",
        ],
        refusal_code: None,
        reason: "Roles/RLS, perspectives, cultures/translations, and named expressions can be inventoried from TMDL source files; mutation remains fixture-gated.",
        next_proof: &[
            "Desktop-authored TMDL fixtures for role, perspective, culture, and named-expression mutations",
        ],
        reference_signals: &[
            "https://learn.microsoft.com/en-us/power-bi/developer/projects/projects-dataset: TMDL folders include roles, perspectives, cultures, and definition files",
        ],
        tags: &[
            "tmdl",
            "semantic-model",
            "roles",
            "rls",
            "perspectives",
            "cultures",
        ],
    },
    Feature {
        id: "model.relationships",
        title: "Model relationships",
        category: "model",
        status: "supported",
        support: "read-write",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[
            "model relationships list",
            "model relationships show",
            "model relationships add",
            "model relationships update",
            "model relationships delete",
        ],
        refusal_code: None,
        reason: "Relationship endpoint and metadata edits are validated against local TMDL tables and columns.",
        next_proof: &["Desktop open/save fixture for relationship edge cases"],
        reference_signals: &[],
        tags: &["semantic-model", "tmdl"],
    },
    Feature {
        id: "model.source-templates",
        title: "Credential-free source templates and rebind runbooks",
        category: "model",
        status: "supported",
        support: "sidecar-sql-postgres-odbc-excel-csv-folder-sharepoint",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[
            "source-template list",
            "source-template show",
            "source-template add",
            "source-template apply",
            "handoff rebind-plan",
        ],
        refusal_code: None,
        reason: "Credential-free SQL Server, PostgreSQL, ODBC, Excel, CSV, folder, and SharePoint/OneDrive M templates are stored in sidecar metadata and can replace safe generated dummy partitions. File-family templates emit explicit TMDL-derived type conversions. An exact-handle confirmation gate also permits intentional retargeting of recognized credential-free existing sources without embedding credentials.",
        next_proof: &[
            "Manually rebind and refresh representative SQL Server, PostgreSQL/Npgsql, ODBC/DSN, Excel, CSV, folder, and SharePoint/OneDrive projects in Power BI Desktop",
        ],
        reference_signals: &[],
        tags: &[
            "semantic-model",
            "source-template",
            "postgres",
            "odbc",
            "excel",
            "csv",
            "folder",
            "sharepoint",
            "handoff",
            "rebind",
        ],
    },
    Feature {
        id: "model.partition-grouped-rank",
        title: "Refresh-time grouped rank partition generator",
        category: "model",
        status: "supported",
        support: "safe-generated-partition-mutation",
        proof_level: "schema-golden",
        emits_pbir: false,
        commands: &["model partitions add-grouped-rank"],
        refusal_code: None,
        reason: "A guarded generator appends sort, buffered per-group eligibility splitting, 1-based indexing, zero-ranked ineligible rows, recombination, and an explicit Int64 retype to one safe generated partition.",
        next_proof: &[
            "Refresh a grouped-rank analytics table in Power BI Desktop and verify ranks against bounded DAX queries",
        ],
        reference_signals: &[],
        tags: &["semantic-model", "partition", "m", "rank", "performance"],
    },
    Feature {
        id: "workflow.source-profile",
        title: "Deterministic staged source-profile workflow",
        category: "workflow",
        status: "supported",
        support: "plan-run-verify",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &["workflow plan", "workflow run", "workflow verify"],
        refusal_code: None,
        reason: "A fingerprinted profile and narrow PBIP closure drive exact partition source changes in a fresh output through the pinned local MCP process, followed by native and official validation. Default tests cover the offline contract and CI runs plan, run, and verify against the exact installed sidecars.",
        next_proof: &[],
        reference_signals: &[],
        tags: &["workflow", "source-profile", "mcp", "pbip", "validation"],
    },
    Feature {
        id: "report.dashboard-spec-v2",
        title: "Strict dashboard spec v2 shape and compilation boundary",
        category: "report",
        status: "supported",
        support: "strict-shape-partial-compile",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &["report spec fields", "report spec validate", "report build"],
        refusal_code: None,
        reason: "powerbi-cli.dashboard.v2 is a strict superset of v1 with versioned allowed-key tables and deny-unknown-fields models. The currently compiled subset is artifact-identical to v1; every recognized future section stops with unsupported_feature and its owning T3 bead id.",
        next_proof: &[
            "Land the named T3 compiler bead for each currently refused v2 section",
            "Promote generated v2 archetypes through the existing Desktop proof ladder",
        ],
        reference_signals: &[
            "examples/sales.dashboard.v2.json: minimal compiled v2 parity fixture",
        ],
        tags: &["report", "dashboard", "spec", "v2", "compiler", "agent"],
    },
    Feature {
        id: "report.intent-parser",
        title: "Structured report intent parsing",
        category: "report",
        status: "supported",
        support: "deterministic-json-markdown-normalization",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &["report plan"],
        refusal_code: None,
        reason: "report plan reads a bounded intent.v1 JSON document or lightly structured Markdown through the input-safety contract, normalizes audience/questions/KPIs and planning constraints, resolves KPI names to exact model measures, and returns pointer-rich diagnostics instead of guessing.",
        next_proof: &[
            "Compile comparisons, periods, drill paths, alerts, filters, archetypes, page flow, and handoff fields through their owning planner/compiler beads",
        ],
        reference_signals: &[
            "examples/intents/sales.intent.json",
            "examples/intents/sales.intent.md",
        ],
        tags: &["report", "intent", "planner", "markdown", "json", "agent"],
    },
    Feature {
        id: "workflow.synthetic-source",
        title: "Offline deterministic synthetic source swap",
        category: "workflow",
        status: "supported",
        support: "shared-m-expressions-with-scale-and-seed",
        proof_level: "schema-golden",
        emits_pbir: false,
        commands: &["workflow synthesize"],
        refusal_code: None,
        reason: "A fresh project copy replaces recognized Database connector roots with shared M expressions; optional exact integer row-scale and seed values invoke deterministic generator functions.",
        next_proof: &[
            "Refresh multiple row scales in Power BI Desktop and record wall-time/canvas evidence",
        ],
        reference_signals: &[],
        tags: &["workflow", "synthetic", "m", "scale", "seed", "offline"],
    },
    Feature {
        id: "report.pages",
        title: "Report pages and layout metadata",
        category: "report",
        status: "supported",
        support: "read-write",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &[
            "report pages list",
            "report pages show",
            "report pages add",
            "report pages update",
            "report pages reorder",
            "report pages set-active",
            "report pages delete-empty",
        ],
        refusal_code: None,
        reason: "PBIR page metadata and pages.json edits are scoped and read back through report commands.",
        next_proof: &["Desktop open/save fixture for page order and active-page semantics"],
        reference_signals: &[],
        tags: &["pbir", "layout"],
    },
    Feature {
        id: "report.design-layout",
        title: "Report design planning and automatic layout",
        category: "report",
        status: "supported",
        support: "read-write-layout",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &["report design-plan", "report layout auto"],
        refusal_code: None,
        reason: "The design planner profiles local TMDL/PBIR metadata and auto-layout rewrites only visual position blocks with guarded mutation modes.",
        next_proof: &[
            "Desktop screenshot fixture to assert generated layouts are visually readable across page sizes",
        ],
        reference_signals: &[],
        tags: &["pbir", "layout", "design", "agent"],
    },
    Feature {
        id: "report.visuals.generated",
        title: "Generated core visuals",
        category: "report",
        status: "supported",
        support: "read-write-small-catalog",
        proof_level: "schema-golden",
        emits_pbir: true,
        commands: &[
            "report visuals catalog",
            "report visuals add",
            "report visuals set-position",
            "report visuals set-bindings",
            "report visuals set-object",
            "report visuals set-display-name",
            "report visuals delete",
        ],
        refusal_code: None,
        reason: "Only cataloged core visual families emit generated PBIR; other visual families are refused until fixture-proven.",
        next_proof: &[
            "Add Desktop-authored golden fixtures before widening visual families or field wells",
        ],
        reference_signals: &[
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: PBIR visual examples and visual catalog references",
        ],
        tags: &["pbir", "visuals", "bindings"],
    },
    Feature {
        id: "report.visuals.role-maps",
        title: "Fixture-backed visual role maps and runtime-parity repair",
        category: "report",
        status: "supported",
        support: "validated-catalog-and-dry-run-repair",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &[
            "report visuals catalog",
            "report visuals add",
            "report visuals set-bindings",
            "report visuals repair-bindings",
            "report spec validate",
        ],
        refusal_code: None,
        reason: "Every generated visual type publishes required and optional roles, measure-only roles, projection limits, mutually exclusive roles, runtime-parity rules, and per-row provenance. Spec validation, generated add, and set-bindings enforce the same closed role surface. The dry-run repair planner can canonicalize proven role aliases and wrap proven Sum aggregations, but never invents or drops fields. Pie, donut, pivotTable, and slicer rows cite real Desktop-authored references; all other rows identify their repository-generated proof honestly.",
        next_proof: &[
            "Harvest independent Desktop-authored references for repository-generated visual families",
            "Run validate --strict --backend all and Desktop refresh/save proof before promoting each unit-smoke row",
        ],
        reference_signals: &[
            "docs/reference/desktop-authored-visuals/pieChart.visual.json",
            "docs/reference/desktop-authored-visuals/donutChart.visual.json",
            "docs/reference/desktop-authored-visuals/matrix.visual.json",
            "docs/reference/desktop-authored-visuals/slicer.visual.json",
            "docs/pilot-lessons.md: scatter runtime-parity findings",
        ],
        tags: &["pbir", "visuals", "bindings", "catalog", "repair"],
    },
    Feature {
        id: "report.visuals.category-share",
        title: "Generated pie and donut visuals",
        category: "report",
        status: "supported",
        support: "generated-desktop-golden-pending",
        proof_level: "desktop-golden-pending",
        emits_pbir: true,
        commands: &[
            "report visuals catalog",
            "report visuals add",
            "report visuals set-bindings",
            "report build",
        ],
        refusal_code: None,
        reason: "The CLI generates pieChart and donutChart from Desktop-authored reference shapes with exactly one Category column, one or more Y values, and the Desktop default descending Y sort. Local golden and round-trip coverage is complete, and testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json proves the binding/canvas baseline. Current generated title container bytes await Desktop re-verification; validator-rejected general.altText is omitted.",
        next_proof: &[
            "Automate the manual pie/donut canvas and refresh assertions as the desktop-canvas-refresh proof level",
            "Widen typed pie/donut formatting coverage with Desktop-authored fixtures and PBIR readback",
        ],
        reference_signals: &[
            "wo-refs/pieChart.visual.json and wo-refs/donutChart.visual.json: Microsoft BCApps MIT-licensed Desktop-authored PBIR references",
            "testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json: refreshed pie/donut canvas values and sort manually verified in Desktop Store 2.155.756.0",
        ],
        tags: &["pbir", "visuals", "pie", "donut", "bindings"],
    },
    Feature {
        id: "report.visuals.combo-pareto",
        title: "Generated line and clustered-column combo visual",
        category: "report",
        status: "supported",
        support: "generated-manual-desktop-canvas-refresh",
        proof_level: "manual-desktop-canvas-refresh",
        emits_pbir: true,
        commands: &[
            "report visuals catalog",
            "report visuals add",
            "report visuals set-bindings",
            "report build",
        ],
        refusal_code: None,
        reason: "The CLI emits lineClusteredColumnComboChart with Category, Y column measures, Y2 line measures, optional Tooltips, and one optional explicit descending measure sort. Local schema goldens and lifecycle readback cover the generated PBIR. Power BI Desktop Store 2.156.951.0 refreshed, rendered, sorted, and saved the deterministic combo fixture on 2026-07-23.",
        next_proof: &[
            "Automate rendered-value and blank-canvas assertions beyond the recorded manual/screen-agent proof",
            "Keep ascending and multi-key sort refused until separate Desktop-authored fixtures exist",
        ],
        reference_signals: &[
            "tests/report.rs: combo add, sort, readback, rebind, clone, and validation lifecycle",
            "examples/archetypes/catalog-proof.dashboard.json: deterministic Pareto-style combo proof",
            "docs/reference/desktop-authored-visuals/lineClusteredColumnComboChart.visual.json: Desktop-saved canonical PBIR",
            "testdata/desktop-proof/combo-pareto.2026-07-23.refresh-session.json: managed refresh/render/save evidence",
        ],
        tags: &["pbir", "visuals", "combo", "pareto", "sort", "bindings"],
    },
    Feature {
        id: "report.visuals.matrix",
        title: "Generated matrix visual",
        category: "report",
        status: "supported",
        support: "generated-desktop-golden-pending",
        proof_level: "desktop-golden-pending",
        emits_pbir: true,
        commands: &[
            "report visuals catalog",
            "report visuals add",
            "report visuals set-bindings",
            "report build",
        ],
        refusal_code: None,
        reason: "The CLI resolves matrix to the PBIR visualType pivotTable and generates ordered Rows, optional Columns, and Values projections from a Desktop-authored reference shape. Matrices with multiple Rows bindings also enable Desktop's native per-row +/- expand/collapse controls. Local golden and round-trip coverage is complete, and testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json proves the binding/canvas baseline. Current generated title container bytes await Desktop re-verification; validator-rejected general.altText is omitted.",
        next_proof: &[
            "Automate the manual matrix canvas and refresh assertions as the desktop-canvas-refresh proof level",
            "Widen typed matrix formatting and hierarchy coverage with Desktop-authored fixtures and PBIR readback",
        ],
        reference_signals: &[
            "wo-refs/matrix.visual.json: Microsoft BCApps MIT-licensed Desktop-authored PBIR reference",
            "testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json: refreshed pivotTable rows, columns, totals, and exact values manually verified in Desktop Store 2.155.756.0",
        ],
        tags: &["pbir", "visuals", "matrix", "pivotTable", "bindings"],
    },
    Feature {
        id: "report.visuals.template-clone",
        title: "Template visual clone",
        category: "report",
        status: "supported",
        support: "guarded-copy",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &["report visuals clone"],
        refusal_code: None,
        reason: "Simple visual containers can be cloned without inventing their PBIR internals.",
        next_proof: &[
            "Desktop open/save fixture for complex template visuals with sidecar resources",
        ],
        reference_signals: &[],
        tags: &["pbir", "visuals", "template"],
    },
    Feature {
        id: "report.filters.categorical",
        title: "Categorical report/page/visual filters",
        category: "report",
        status: "supported",
        support: "read-write-categorical",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &[
            "report filters list",
            "report filters show",
            "report filters add",
            "report filters update",
            "report filters delete",
            "report filters clear",
        ],
        refusal_code: None,
        reason: "The CLI writes scoped PBIR filterConfig.filters categorical shapes, replaces values by stable handle without changing type, and warns about persisted dummy values.",
        next_proof: &[
            "Repeat the existing Desktop canvas/open-save proof after categorical update-by-handle",
        ],
        reference_signals: &[],
        tags: &["pbir", "filters"],
    },
    Feature {
        id: "report.filters.numeric-range",
        title: "Numeric range report/page/visual filters",
        category: "report",
        status: "supported",
        support: "read-write-advanced-comparison",
        proof_level: "schema-golden",
        emits_pbir: true,
        commands: &[
            "report filters list",
            "report filters show",
            "report filters add",
            "report filters update",
            "report filters delete",
        ],
        refusal_code: None,
        reason: "The CLI type-checks numeric TMDL columns and writes Version 2 Advanced filters using >= and/or <= Comparison conditions with Source aliases. Update may change the display name but refuses type or bound changes.",
        next_proof: &[
            "Open, render, save, and reopen report/page/visual open-ended and closed numeric ranges in Power BI Desktop",
        ],
        reference_signals: &[
            "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/semanticQuery/1.2.0/schema.json: FilterDefinition and ComparisonCondition",
            "https://github.com/microsoft/skills-for-fabric/blob/main/skills/powerbi-report-authoring/references/filters.md: Advanced filter reference",
        ],
        tags: &["pbir", "filters", "advanced", "numeric-range"],
    },
    Feature {
        id: "report.filters.topn",
        title: "TopN visual filters ordered by a measure",
        category: "report",
        status: "supported",
        support: "read-write-visual-subquery",
        proof_level: "schema-golden",
        emits_pbir: true,
        commands: &[
            "report filters list",
            "report filters show",
            "report filters add",
            "report filters update",
            "report filters delete",
            "report visuals set-topn-guard",
        ],
        refusal_code: None,
        reason: "The CLI writes the schema-defined visual-only Type 2 subquery/In.Table TopN shape, with a TMDL-resolved measure OrderBy. Report/page TopN and type-changing updates are refused. `report visuals set-topn-guard` creates or updates the same shape by field.",
        next_proof: &[
            "Capture Desktop-authored top and bottom measure-ranked visual filters and compare the OrderBy expression byte-for-byte",
            "Open, render, save, and reopen generated TopN visual filters in Power BI Desktop",
        ],
        reference_signals: &[
            "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/semanticQuery/1.2.0/schema.json: SubqueryExpression, QuerySortClause, and Top",
            "https://github.com/microsoft/skills-for-fabric/blob/main/skills/powerbi-report-authoring/references/filters.md: TopN subquery/In.Table reference and Desktop caveat",
        ],
        tags: &["pbir", "filters", "topn", "visual", "measure"],
    },
    Feature {
        id: "report.filters.relative-date",
        title: "Relative-date report/page/visual filters",
        category: "report",
        status: "supported",
        support: "read-write-between-date-expressions",
        proof_level: "schema-golden",
        emits_pbir: true,
        commands: &[
            "report filters list",
            "report filters show",
            "report filters add",
            "report filters update",
            "report filters delete",
        ],
        refusal_code: None,
        reason: "The CLI type-checks date TMDL columns and writes Version 2 RelativeDate Between filters using DateSpan, DateAdd, and Now expressions. Update may change the display name but refuses type or window changes.",
        next_proof: &[
            "Capture Desktop-authored rolling and calendar last/next/this filters for every supported unit",
            "Open, render, save, and reopen generated relative-date filters in Power BI Desktop",
        ],
        reference_signals: &[
            "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/semanticQuery/1.2.0/schema.json: BetweenCondition, DateSpanExpression, DateAddExpression, and NowExpression",
            "https://github.com/microsoft/skills-for-fabric/blob/main/skills/powerbi-report-authoring/references/filters.md: RelativeDate expression reference",
        ],
        tags: &["pbir", "filters", "relative-date", "calendar"],
    },
    Feature {
        id: "report.slicer-clear",
        title: "Slicer inventory and persisted-selection clear",
        category: "report",
        status: "supported",
        support: "read-write-clear-only",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &[
            "report slicers list",
            "report slicers show",
            "report slicers clear",
        ],
        refusal_code: None,
        reason: "Slicer state clearing is guarded and preserves slicer bindings/layout/formatting.",
        next_proof: &[
            "Desktop-authored fixtures for selection defaults, selection updates, additional modes, and sync groups",
        ],
        reference_signals: &[
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: PBIR slicer syncGroup examples",
        ],
        tags: &["pbir", "slicers"],
    },
    Feature {
        id: "report.interactions.overrides",
        title: "Explicit visual interaction overrides",
        category: "report",
        status: "supported",
        support: "read-write-explicit-overrides",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &[
            "report build",
            "report interactions list",
            "report interactions show",
            "report interactions set",
            "report interactions disable",
        ],
        refusal_code: None,
        reason: "DataFilter, HighlightFilter, and NoFilter rows can be declared by page-local visual ID during report build, then inspected and upserted in page visualInteractions.",
        next_proof: &["Desktop fixture for Default/reset semantics and interaction UI state"],
        reference_signals: &[
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: PBIR visualInteractions references and examples",
        ],
        tags: &["pbir", "interactions"],
    },
    Feature {
        id: "report.bookmarks.readback",
        title: "Bookmark inventory/readback and metadata edits",
        category: "report",
        status: "supported",
        support: "read-write-metadata-only",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &[
            "report bookmarks list",
            "report bookmarks show",
            "report bookmarks set-display-name",
            "report bookmarks reorder",
            "report bookmarks delete",
        ],
        refusal_code: None,
        reason: "Bookmarks are read with state/safety summaries; metadata-only display name, flat order, and delete edits are guarded. Capturing or creating bookmark state remains gated because bookmarks snapshot broad report state.",
        next_proof: &["Desktop-authored bookmark state creation/update fixtures"],
        reference_signals: &[
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: PBIR bookmark examples",
        ],
        tags: &["pbir", "bookmarks"],
    },
    Feature {
        id: "report.themes",
        title: "Theme, visual formatting, and master style bundles",
        category: "report",
        status: "supported",
        support: "guarded-bundle-copy",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &[
            "report themes show",
            "report themes extract",
            "report themes apply",
            "report themes presets",
            "report themes apply-preset",
            "report visuals formatting list",
            "report visuals formatting show",
            "report visuals formatting extract",
            "report visuals formatting apply",
            "report visuals formatting set-text",
            "report visuals formatting set-color",
            "report style inspect",
            "report style extract",
            "report style apply",
            "report style diff",
        ],
        refusal_code: None,
        reason: "Theme, visual formatting, and master-style bundle operations preserve raw PBIR cards while guarding literal text and typed color/text patches.",
        next_proof: &["Desktop-authored conditional formatting fixture before CF authoring"],
        reference_signals: &[],
        tags: &["pbir", "themes", "formatting"],
    },
    Feature {
        id: "report.drillthrough",
        title: "Same-report drillthrough page bindings",
        category: "report",
        status: "supported",
        support: "read-write-page-binding",
        proof_level: "schema-golden",
        emits_pbir: true,
        commands: &[
            "report drillthrough set",
            "report drillthrough show",
            "report drillthrough clear",
        ],
        refusal_code: None,
        reason: "The CLI writes a one-column same-report pageBinding parameter with boundFilter + fieldExpr and a paired bodyless Categorical Drillthrough filter, matching the public Desktop-authored microsoft/BCApps reference shape and schema. Cross-report drillthrough, visual action links, measures, and field parameters remain explicitly refused.",
        next_proof: &[
            "Commit a reproducible Desktop proof for drillthrough-well registration, source context-menu availability, navigation, and carried filters",
            "Automate drillthrough-well registration, source context-menu availability, navigation, and carried-filter assertions",
            "Capture a Desktop-authored visual drillthrough action fixture before visual link authoring",
            "Add multi-field and cross-report goldens before expanding command scope",
        ],
        reference_signals: &[
            "microsoft/BCApps Sales app page 429930d2d08538d4d2bb: linked boundFilter + fieldExpr + bodyless Drillthrough filter",
            "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/2.0.0/schema.json: pageBinding.type=Drillthrough and parameters[].fieldExpr",
            "https://learn.microsoft.com/en-us/power-bi/create-reports/desktop-drillthrough: Desktop drillthrough UX behavior",
        ],
        tags: &["pbir", "drillthrough", "page-binding"],
    },
    Feature {
        id: "report.drilldown",
        title: "Hierarchy drilldown authoring",
        category: "report",
        status: "supported",
        support: "read-write-category-hierarchy",
        proof_level: "unit-smoke",
        emits_pbir: true,
        commands: &["report drilldown set-hierarchy"],
        refusal_code: None,
        reason: "The CLI writes multiple resolved model-column projections under visual.query.queryState.Category for existing line, area, bar, column, and combo charts, marks the first field active as the initial level, then explicitly enables their visual-header drill controls. Scatter is refused because Microsoft Report Authoring permits only one Category projection. Later end-user drill position and expanded data state remain transient.",
        next_proof: &[
            "Capture Desktop-authored chart hierarchy drilldown fixtures for every supported chart family",
            "Add Desktop oracle screenshot/readback proof for hierarchy buttons on generated samples",
        ],
        reference_signals: &[
            "PBIR visual queryState role projections already support multiple fields in one field well",
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: PBIR visual examples include drillFilterOtherVisuals",
        ],
        tags: &["pbir", "drilldown", "hierarchy"],
    },
    Feature {
        id: "report.tooltip-pages",
        title: "Report tooltip pages",
        category: "report",
        status: "planned",
        support: "unsupported",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[],
        refusal_code: Some("unsupported_feature"),
        reason: "Tooltip page metadata and visualTooltip opt-in are known PBIR surfaces, but not yet pinned in this CLI's goldens.",
        next_proof: &[
            "Capture Desktop-authored tooltip page fixture",
            "Implement report tooltips command family",
        ],
        reference_signals: &[
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: PBIR format references mention tooltip pages and visualTooltip",
        ],
        tags: &["pbir", "tooltips"],
    },
    Feature {
        id: "report.bookmark-mutations",
        title: "Bookmark state capture/create/update",
        category: "report",
        status: "planned",
        support: "unsupported",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[],
        refusal_code: Some("unsupported_feature"),
        reason: "Metadata-only bookmark edits are supported, but capturing or authoring bookmark explorationState snapshots needs Desktop-authored goldens.",
        next_proof: &[
            "Capture Desktop-authored bookmark create/update fixtures",
            "Add bookmark state diff/normalize coverage",
        ],
        reference_signals: &[
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: PBIR bookmark examples",
        ],
        tags: &["pbir", "bookmarks"],
    },
    Feature {
        id: "report.slicer-authoring",
        title: "Generated basic, dropdown, and between slicers",
        category: "report",
        status: "supported",
        support: "generated-clean-state-desktop-golden-pending",
        proof_level: "desktop-golden-pending",
        emits_pbir: true,
        commands: &[
            "report visuals catalog",
            "report visuals add",
            "report visuals set-bindings",
            "report build",
            "report slicers list",
            "report slicers show",
            "report slicers clear",
        ],
        refusal_code: None,
        reason: "The CLI generates a slicer with exactly one Values column and a Basic, Dropdown, or Between mode under /visual/objects/data. Between also writes /visual/objects/slider.show=true so the numeric/date range is an explicit draggable band. Generated slicers deliberately contain no general.filter or other persisted selection state and omit validator-rejected general.altText. Local golden, hygiene, and round-trip coverage is complete, and testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json proves the clean Basic binding/canvas baseline. Current generated title container bytes await Desktop re-verification.",
        next_proof: &[
            "Automate the manual slicer canvas, refresh, and interaction assertions as the desktop-canvas-refresh proof level",
            "Widen typed slicer formatting and mode coverage with Desktop-authored fixtures and PBIR readback",
        ],
        reference_signals: &[
            "wo-refs/slicer.visual.json: Microsoft BCApps MIT-licensed Desktop-authored PBIR reference; persisted general.filter was intentionally excluded",
            "testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json: clean Basic slicer render and live selection manually verified in Desktop Store 2.155.756.0",
        ],
        tags: &["pbir", "slicers"],
    },
    Feature {
        id: "report.slicer-sync-authoring",
        title: "Slicer sync groups",
        category: "report",
        status: "planned",
        support: "unsupported",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[],
        refusal_code: Some("unsupported_feature"),
        reason: "syncGroup appears in PBIR examples, but the CLI does not yet author or mutate slicer sync state.",
        next_proof: &[
            "Capture synced slicer Desktop fixture",
            "Add syncGroup summary to fixture normalize",
        ],
        reference_signals: &[
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: K201-MonthSlicer PBIR examples include syncGroup",
        ],
        tags: &["pbir", "slicers", "sync"],
    },
    Feature {
        id: "report.interaction-default-reset",
        title: "Interaction Default/reset semantics",
        category: "report",
        status: "planned",
        support: "unsupported",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[],
        refusal_code: Some("unsupported_feature"),
        reason: "Explicit interaction overrides are supported, but Desktop's absent-row/default/reset semantics are not yet pinned.",
        next_proof: &[
            "Capture Desktop reset/default interaction fixtures",
            "Define command behavior for row deletion vs explicit Default",
        ],
        reference_signals: &[
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: PBIR visualInteractions references",
        ],
        tags: &["pbir", "interactions"],
    },
    Feature {
        id: "report.visuals.planned-types",
        title: "Generated PBIR for non-catalog visual types",
        category: "report",
        status: "planned",
        support: "unsupported",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[],
        refusal_code: Some("unsupported_feature"),
        reason: "Non-catalog visual families are intentionally refused until their field wells and formatting shapes are fixture-proven.",
        next_proof: &[
            "Add Desktop-authored fixtures for maps, navigators, gauges, decomposition trees, and non-catalog custom visuals",
        ],
        reference_signals: &[
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: PBIR examples include broader visual families",
        ],
        tags: &["pbir", "visuals", "catalog"],
    },
    Feature {
        id: "report.conditional-formatting",
        title: "Conditional formatting readback",
        category: "report",
        status: "supported",
        support: "read-only-static-scan",
        proof_level: "unit-smoke",
        emits_pbir: false,
        commands: &[
            "report visuals formatting conditional-formatting list",
            "report visuals formatting conditional-formatting show",
        ],
        refusal_code: None,
        reason: "Static formatting patches and raw formatting bundles are supported; conditional formatting signals can be inventoried, while data-bound/wildcard CF authoring still needs fixture proof.",
        next_proof: &[
            "Capture Desktop-authored conditional formatting fixtures",
            "Implement typed CF authoring commands with Desktop fixture readback",
        ],
        reference_signals: &[
            "https://github.com/data-goblin/power-bi-agentic-development @ 9704f1d: formatted visual examples include conditional formatting selectors",
        ],
        tags: &["pbir", "formatting", "conditional-formatting"],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_records_preserve_every_existing_feature_proof_level() {
        let records = embedded_desktop_proof_records().expect("embedded proof records");
        validate_proof_record_feature_ids(&records).expect("known feature ids");
        for feature in FEATURE_CATALOG {
            assert_eq!(
                feature_json(feature, &records)["proofLevel"],
                feature.proof_level,
                "record unexpectedly changed {}",
                feature.id
            );
        }
    }

    #[test]
    fn feature_level_uses_maximum_linked_proven_record() {
        let placeholder = crate::desktop_proof::parse_desktop_proof_record(
            "placeholder.json",
            r#"{
                "schema":"powerbi-cli.desktop-proof.v1",
                "fixture":"archetypes/placeholder",
                "date":"2026-09-03",
                "desktopVersion":null,
                "commands":["powerbi-cli validate --strict build/placeholder --json"],
                "signals":{"featureIds":["report.visuals.generated"],"placeholder":true},
                "proofLevel":"desktop-golden-pending"
            }"#,
        )
        .expect("placeholder record");
        let proven = crate::desktop_proof::parse_desktop_proof_record(
            "proven.json",
            r#"{
                "schema":"powerbi-cli.desktop-proof.v1",
                "fixture":"archetypes/proven",
                "date":"2026-09-04",
                "desktopVersion":"2.200.1.0",
                "commands":["powerbi-cli desktop open-check build/proven --json"],
                "signals":{"featureIds":["report.visuals.generated"],"schemaGolden":true,"desktopReferencePresent":true,"currentArtifact":true,"desktopOpened":true,"canvasRendered":true,"refreshCompleted":true,"issueDialogsAbsent":true,"expectedValuesMatched":true,"automated":true},
                "proofLevel":"desktop-canvas-refresh"
            }"#,
        )
        .expect("proven record");
        let feature = feature_by_id("report.visuals.generated").expect("feature");
        let mut records = vec![LoadedDesktopProofRecord {
            source: "placeholder.json",
            record: placeholder,
        }];
        assert_eq!(
            feature_json(&feature, &records)["proofLevel"],
            "desktop-golden-pending"
        );
        records.push(LoadedDesktopProofRecord {
            source: "proven.json",
            record: proven,
        });
        let value = feature_json(&feature, &records);
        assert_eq!(value["proofLevel"], "desktop-canvas-refresh");
        assert_eq!(value["proofRecords"][0]["path"], "placeholder.json");
        assert_eq!(value["proofRecords"][1]["path"], "proven.json");
    }

    #[test]
    fn features_list_is_deterministic_with_embedded_records() {
        let args = ["list".to_string()];
        let first = serde_json::to_vec(&features_command(&args).expect("first feature list"))
            .expect("serialize first feature list");
        let second = serde_json::to_vec(&features_command(&args).expect("second feature list"))
            .expect("serialize second feature list");
        assert_eq!(first, second);
    }

    #[test]
    fn record_with_unknown_feature_id_is_rejected() {
        let record = crate::desktop_proof::parse_desktop_proof_record(
            "unknown.json",
            r#"{
                "schema":"powerbi-cli.desktop-proof.v1",
                "fixture":"archetypes/unknown",
                "date":"2026-09-04",
                "desktopVersion":null,
                "commands":["powerbi-cli validate --strict build/unknown --json"],
                "signals":{"featureIds":["report.does-not-exist"]},
                "proofLevel":"unit-smoke"
            }"#,
        )
        .expect("record shape");
        let error = validate_proof_record_feature_ids(&[LoadedDesktopProofRecord {
            source: "unknown.json",
            record,
        }])
        .expect_err("unknown feature id");
        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("report.does-not-exist"));
    }
}
