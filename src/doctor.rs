use crate::{
    EXIT_SUCCESS, PBIP_SCHEMA, REPORT_DEFINITION_SCHEMA, SEMANTIC_MODEL_DEFINITION_SCHEMA,
    contract, desktop,
};
use serde_json::{Value, json};

pub(crate) fn doctor_json() -> Value {
    let desktop_detection = desktop::detect_power_bi_desktop(None);
    let desktop_status = if desktop_detection.found {
        "pass"
    } else {
        "warn"
    };
    let desktop_severity = if desktop_detection.found {
        "info"
    } else {
        "warning"
    };
    json!({
        "schema": "powerbi-cli.doctor.v1",
        "tool": "powerbi-cli",
        "version": env!("CARGO_PKG_VERSION"),
        "contractVersion": contract::CONTRACT_VERSION,
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "checks": [
            {
                "id": "platform",
                "status": "pass",
                "severity": "info",
                "message": "Offline PBIP/PBIR/TMDL authoring commands are cross-platform.",
                "platform": std::env::consts::OS
            },
            {
                "id": "powerBiDesktop",
                "status": desktop_status,
                "severity": desktop_severity,
                "message": if desktop_detection.found {
                    "Power BI Desktop was detected for optional Desktop oracle checks."
                } else {
                    "Power BI Desktop was not detected; file authoring still works, but Desktop oracle checks are unavailable here."
                },
                "found": desktop_detection.found,
                "path": desktop_detection.path,
                "version": desktop_detection.version,
                "source": desktop_detection.source,
                "checked": desktop_detection.checked,
                "next": if desktop_detection.found {
                    vec![
                        "powerbi-cli desktop open-check <project-dir-or.pbip> --json",
                        "powerbi-cli desktop screenshot <project-dir-or.pbip> --out <evidence.png> --json"
                    ]
                } else {
                    Vec::<&str>::new()
                },
                "instructions": if desktop_detection.found {
                    Vec::<&str>::new()
                } else {
                    vec!["Install Power BI Desktop on Windows, or pass --desktop-path to a Desktop oracle command on an oracle machine."]
                }
            },
            {
                "id": "desktopProofLevel",
                "status": "warn",
                "severity": "warning",
                "message": "Desktop oracle commands can now prove process launch and, when a matching titled window appears, desktop-window observation. Screenshot PNGs are review evidence only; canvas render and refresh compatibility are still not automated.",
                "currentLevel": "desktop-window",
                "observableLevels": ["desktop-launch", "desktop-window"],
                "requiredCompatibilityLevel": "desktop-canvas-refresh",
                "next": [
                    "powerbi-cli desktop open-check <project-dir-or.pbip> --json",
                    "powerbi-cli desktop screenshot <project-dir-or.pbip> --out <evidence.png> --json"
                ],
                "instructions": [
                    "Capture Desktop-saved fixture goldens before broadening PBIR feature claims."
                ]
            },
            {
                "id": "offlineSafety",
                "status": "pass",
                "severity": "info",
                "message": "Generated projects use dummy Power Query rows and do not write credentials or data caches.",
                "dataCacheGenerated": false,
                "credentialsGenerated": false
            }
        ],
        "powerBiDesktop": {
            "found": desktop_detection.found,
            "path": desktop_detection.path,
            "version": desktop_detection.version,
            "checked": desktop_detection.checked,
            "note": "Power BI Desktop is only needed to open/prove PBIP/PBIX compatibility; scaffold and validate are offline file operations."
        },
        "formatAssumptions": {
            "pbipSchema": PBIP_SCHEMA,
            "reportDefinitionSchema": REPORT_DEFINITION_SCHEMA,
            "semanticModelDefinitionSchema": SEMANTIC_MODEL_DEFINITION_SCHEMA,
            "reportFormat": "PBIR",
            "semanticModelFormat": "TMDL"
        },
        "offlineSafety": {
            "generatedDataMode": "Power Query M #table dummy rows",
            "dataCacheGenerated": false,
            "credentialsGenerated": false
        },
        "next": [
            "powerbi-cli --json capabilities",
            "powerbi-cli validate --strict <project-dir-or.pbip> --json",
            "powerbi-cli desktop open-check <project-dir-or.pbip> --json",
            "powerbi-cli desktop screenshot <project-dir-or.pbip> --out <evidence.png> --json"
        ]
    })
}
