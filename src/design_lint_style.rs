//! Checks against a policy recovered from an exact registered token theme.

use crate::design::lint::design_finding;
use crate::design::tokens::{compile_theme, contrast_ratio, token_catalog};
use crate::{CliResult, ResolvedProject, read_json_value, rules};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) fn resolved_policy(project: &ResolvedProject) -> CliResult<Option<Value>> {
    let report = read_json_value(&project.report_dir.join("definition/report.json"))?;
    let Some(name) = report
        .pointer("/themeCollection/customTheme/name")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let themes = crate::pbir_themes::list_report_themes(project)?;
    let Some(active) = themes
        .iter()
        .find(|theme| theme.name == name || theme.relative_path.ends_with(name))
    else {
        return Ok(None);
    };
    for tokens in token_catalog()?["sets"].as_array().into_iter().flatten() {
        let bundle = compile_theme(tokens)?;
        if bundle["registeredThemes"][0]["themeJson"] == active.theme {
            return Ok(Some(tokens.clone()));
        }
    }
    // A filename/preset id alone cannot recover custom overrides safely.
    Ok(None)
}

fn scalar(value: &Value) -> &Value {
    value.pointer("/expr/Literal/Value").unwrap_or(value)
}

fn number(value: &Value) -> Option<f64> {
    let value = scalar(value);
    value.as_f64().or_else(|| {
        value
            .as_str()?
            .trim_matches('\'')
            .trim_end_matches("pt")
            .trim_end_matches(['D', 'd', 'L'])
            .parse()
            .ok()
    })
}

fn color(value: &Value) -> Option<[f64; 3]> {
    let value = value.pointer("/solid/color").unwrap_or(value);
    let text = scalar(value)
        .as_str()?
        .trim_matches('\'')
        .strip_prefix('#')?;
    let hex = if text.len() == 3 {
        text.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        text.to_string()
    };
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let mut rgb = [0.0; 3];
    for (index, channel) in rgb.iter_mut().enumerate() {
        *channel = f64::from(u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?) / 255.0;
    }
    Some(rgb)
}

fn walk<'a>(value: &'a Value, pointer: &str, out: &mut Vec<(String, String, &'a Value)>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let path = format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"));
                out.push((key.clone(), path.clone(), value));
                walk(value, &path, out);
            }
        }
        Value::Array(array) => {
            for (index, value) in array.iter().enumerate() {
                out.push((String::new(), format!("{pointer}/{index}"), value));
                walk(value, &format!("{pointer}/{index}"), out);
            }
        }
        _ => {}
    }
}

fn emit(
    findings: &mut Vec<Value>,
    id: &str,
    visual: &Value,
    prefix: &str,
    pointer: &str,
    evidence: Value,
) {
    findings.push(design_finding(
        id,
        visual["handle"].as_str(),
        visual["path"].as_str().map(Path::new),
        format!("{prefix}{pointer}"),
        rules::find_rule(id).unwrap().summary.to_string(),
        evidence,
    ));
}

pub(crate) fn lint(
    raw: &Value,
    visual: &Value,
    page: usize,
    index: usize,
    tokens: &Value,
    findings: &mut Vec<Value>,
) {
    let prefix = format!("/report/pages/{page}/visuals/{index}");
    let minimum = tokens["typography"]["scale"]["caption"]
        .as_f64()
        .unwrap_or(10.0);
    let mut properties = Vec::new();
    for root in ["/visual/objects", "/visual/visualContainerObjects"] {
        if let Some(value) = raw.pointer(root) {
            walk(value, root, &mut properties);
        }
    }
    let mut token_properties = Vec::new();
    walk(tokens, "", &mut token_properties);
    let allowed = token_properties
        .iter()
        .filter_map(|(_, _, value)| color(value))
        .collect::<Vec<_>>();
    for (key, pointer, value) in &properties {
        if key == "fontSize"
            && let Some(size) = number(value)
            && size < minimum
        {
            emit(
                findings,
                rules::DESIGN_FONT_BELOW_MINIMUM,
                visual,
                &prefix,
                pointer,
                json!({"fontSize": size, "minimum": minimum, "policy": tokens["id"]}),
            );
        }
        if key == "color"
            && let Some(rgb) = color(value)
            && !allowed.contains(&rgb)
        {
            emit(
                findings,
                rules::DESIGN_PALETTE_DRIFT,
                visual,
                &prefix,
                pointer,
                json!({"color": scalar(value), "policy": tokens["id"]}),
            );
        }
        if matches!(key.as_str(), "displayUnits" | "labelDisplayUnits")
            && number(value) == Some(0.0)
            && tokens["numberFormats"]["compactAbove"]
                .as_f64()
                .is_some_and(|value| value > 0.0)
            && visual["bindings"]
                .as_array()
                .is_some_and(|bindings| bindings.iter().any(|binding| binding["kind"] == "measure"))
        {
            emit(
                findings,
                rules::DESIGN_DISPLAY_UNITS_MISSING,
                visual,
                &prefix,
                pointer,
                json!({"displayUnits": 0, "inheritedCompactUnits": tokens["numberFormats"]["compactAbove"], "reason": "explicit automatic units override the resolved compact-unit policy"}),
            );
        }
    }
    // Compare explicit title text to its actual opaque visual background,
    // falling back only to the exactly matched token theme's inherited pair.
    let background_props = raw.pointer("/visual/visualContainerObjects/background/0/properties");
    if background_props.is_some_and(|props| {
        props
            .get("transparency")
            .is_some_and(|value| number(value) != Some(0.0))
    }) {
        return;
    }
    let background = background_props
        .and_then(|props| props.get("color"))
        .unwrap_or(&tokens["surfaces"]["page"]);
    let title_props = raw.pointer("/visual/visualContainerObjects/title/0/properties");
    if title_props.is_some_and(|props| scalar(&props["show"]).as_str() == Some("false")) {
        return;
    }
    let foreground = title_props
        .and_then(|props| props.get("fontColor"))
        .unwrap_or(&tokens["visualDefaults"]["*"]["title"][0]["fontColor"]);
    if let (Some(fg), Some(bg)) = (color(foreground), color(background)) {
        let ratio = contrast_ratio(fg, bg);
        if ratio < 4.5 {
            emit(
                findings,
                rules::DESIGN_CONTRAST_BELOW_AA,
                visual,
                &prefix,
                "/visual/visualContainerObjects/title/0/properties/fontColor",
                json!({"foreground": foreground, "background": background, "ratio": ratio, "minimum": 4.5, "policy": tokens["id"]}),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn findings(properties: Value) -> Vec<Value> {
        let tokens = token_catalog().unwrap()["sets"][0].clone();
        let raw =
            json!({"visual":{"visualContainerObjects":{"title":[{"properties":properties}]}}});
        let visual = json!({"handle":"visual:P:V", "bindings":[{"kind":"measure"}]});
        let mut findings = Vec::new();
        lint(&raw, &visual, 0, 0, &tokens, &mut findings);
        findings
    }
    fn has(props: Value, id: &str) -> bool {
        findings(props).iter().any(|finding| finding["code"] == id)
    }
    #[test]
    fn font_minimum_has_clean_and_planted_fixtures() {
        assert!(!has(
            json!({"fontSize":{"expr":{"Literal":{"Value":"12D"}}}}),
            rules::DESIGN_FONT_BELOW_MINIMUM
        ));
        assert!(has(
            json!({"fontSize":{"expr":{"Literal":{"Value":"6D"}}}}),
            rules::DESIGN_FONT_BELOW_MINIMUM
        ));
    }
    #[test]
    fn contrast_has_clean_and_planted_fixtures() {
        assert!(!has(
            json!({"fontColor":{"solid":{"color":{"expr":{"Literal":{"Value":"'#1F2937'"}}}}}}),
            rules::DESIGN_CONTRAST_BELOW_AA
        ));
        assert!(has(
            json!({"fontColor":{"solid":{"color":{"expr":{"Literal":{"Value":"'#FFFFFF'"}}}}}}),
            rules::DESIGN_CONTRAST_BELOW_AA
        ));
    }
    #[test]
    fn palette_has_clean_and_planted_fixtures_including_short_hex() {
        assert!(!has(
            json!({"color":{"solid":{"color":{"expr":{"Literal":{"Value":"'#fff'"}}}}}}),
            rules::DESIGN_PALETTE_DRIFT
        ));
        assert!(has(
            json!({"color":{"solid":{"color":{"expr":{"Literal":{"Value":"'#123456'"}}}}}}),
            rules::DESIGN_PALETTE_DRIFT
        ));
    }
    #[test]
    fn inherited_or_explicit_compact_units_are_clean_but_auto_override_is_flagged() {
        assert!(!has(json!({}), rules::DESIGN_DISPLAY_UNITS_MISSING));
        assert!(!has(
            json!({"displayUnits":{"expr":{"Literal":{"Value":"1000000D"}}}}),
            rules::DESIGN_DISPLAY_UNITS_MISSING
        ));
        assert!(has(
            json!({"displayUnits":{"expr":{"Literal":{"Value":"0D"}}}}),
            rules::DESIGN_DISPLAY_UNITS_MISSING
        ));
    }
}
