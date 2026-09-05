//! Text and structure checks, with explicit deferral for unresolved style policy.

use crate::design::lint::design_finding;
use crate::{CliResult, read_json_value, rules};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const DEFERRED: &[(&str, &str)] = &[
    (
        rules::DESIGN_FONT_BELOW_MINIMUM,
        "Resolved typography tokens and their minimum font-size policy are unavailable.",
    ),
    (
        rules::DESIGN_CONTRAST_BELOW_AA,
        "Resolved foreground/background tokens, including theme inheritance, are unavailable.",
    ),
    (
        rules::DESIGN_PALETTE_DRIFT,
        "Resolved palette and semantic-color tokens are unavailable.",
    ),
    (
        rules::DESIGN_DISPLAY_UNITS_MISSING,
        "The active theme cannot resolve a full display-unit policy; offline metadata cannot establish data magnitude.",
    ),
];

pub(crate) fn is_deferred(id: &str) -> bool {
    DEFERRED.iter().any(|(candidate, _)| *candidate == id)
}

pub(crate) fn deferred_rules() -> Value {
    json!(
        DEFERRED
            .iter()
            .map(|(id, reason)| json!({
                "ruleId": id, "status": "not-evaluated", "reason": reason,
                "owningBead": "pbi-t5-design-system-mlf.3"
            }))
            .collect::<Vec<_>>()
    )
}

fn nonempty(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.trim().is_empty())
}

/// Read only persisted title cards. Inspection's title can be an annotation or
/// container-name fallback and therefore is not evidence of a visible title.
fn title(raw: &Value) -> Option<(String, String)> {
    for base in [
        "/visual/visualContainerObjects/title",
        "/visual/objects/title",
    ] {
        if let Some(cards) = raw.pointer(base).and_then(Value::as_array) {
            for (index, card) in cards.iter().enumerate() {
                let props = &card["properties"];
                if props
                    .pointer("/show/expr/Literal/Value")
                    .and_then(Value::as_str)
                    .is_some_and(|show| show.eq_ignore_ascii_case("false"))
                {
                    continue;
                }
                if let Some(literal) = props
                    .pointer("/text/expr/Literal/Value")
                    .and_then(Value::as_str)
                {
                    let text = literal
                        .strip_prefix('\'')
                        .and_then(|text| text.strip_suffix('\''))
                        .unwrap_or(literal)
                        .replace("''", "'");
                    if !text.trim().is_empty() {
                        return Some((
                            text,
                            format!("{base}/{index}/properties/text/expr/Literal/Value"),
                        ));
                    }
                }
                // Conditional titles are intentionally dynamic; the field
                // expression is sufficient structure, but not caseable text.
                if props.pointer("/text/expr").is_some_and(Value::is_object)
                    && props.pointer("/text/expr/Literal").is_none()
                {
                    return Some((String::new(), format!("{base}/{index}/properties/text")));
                }
            }
            // The canonical container title overrides legacy objects/title.
            return None;
        }
    }
    None
}

fn casing(text: &str) -> Option<&'static str> {
    let words = text
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .filter(|word| {
            !word
                .chars()
                .filter(|ch| ch.is_alphabetic())
                .all(char::is_uppercase)
        })
        .collect::<Vec<_>>();
    if words.len() < 2 {
        return None;
    }
    let capitalized = |word: &str| {
        word.chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(char::is_uppercase)
    };
    if !capitalized(words[0]) {
        return Some("lowercase");
    }
    if words[1..].iter().all(|word| {
        word.chars()
            .filter(|ch| ch.is_alphabetic())
            .all(char::is_lowercase)
    }) {
        return Some("sentence");
    }
    if words[1..].iter().all(|word| {
        capitalized(word)
            || matches!(
                *word,
                "a" | "an"
                    | "and"
                    | "as"
                    | "at"
                    | "by"
                    | "for"
                    | "in"
                    | "of"
                    | "on"
                    | "or"
                    | "the"
                    | "to"
                    | "vs"
                    | "with"
            )
    }) {
        return Some("title");
    }
    None
}

fn emit(
    findings: &mut Vec<Value>,
    id: &str,
    visual: &Value,
    page: usize,
    index: usize,
    pointer: &str,
    evidence: Value,
) {
    findings.push(design_finding(
        id,
        visual["handle"].as_str(),
        visual["path"].as_str().map(Path::new),
        format!("/report/pages/{page}/visuals/{index}{pointer}"),
        rules::find_rule(id)
            .expect("registered rule")
            .summary
            .to_string(),
        evidence,
    ));
}

pub(crate) fn lint_page(
    page: &Value,
    page_index: usize,
    ranking: bool,
    findings: &mut Vec<Value>,
    policy: Option<&Value>,
) -> CliResult<()> {
    let Some(visuals) = page["visuals"].as_array() else {
        return Ok(());
    };
    let mut raw = Vec::new();
    for visual in visuals {
        raw.push(match visual["path"].as_str() {
            Some(path) => read_json_value(Path::new(path))?,
            None => Value::Null,
        });
    }
    lint_visuals(visuals, &raw, page_index, ranking, findings);
    if let Some(policy) = policy {
        for (index, (visual, raw)) in visuals.iter().zip(&raw).enumerate() {
            crate::design_lint_style::lint(raw, visual, page_index, index, policy, findings);
        }
    }
    Ok(())
}

fn lint_visuals(
    visuals: &[Value],
    raw: &[Value],
    page: usize,
    ranking: bool,
    findings: &mut Vec<Value>,
) {
    let mut titles = Vec::new();
    for (index, (visual, raw)) in visuals.iter().zip(raw).enumerate() {
        let kind = visual["visualType"].as_str().unwrap_or_default();
        if matches!(
            kind,
            "textbox" | "shape" | "image" | "actionButton" | "button"
        ) {
            continue;
        }
        match title(raw) {
            Some((text, pointer)) if !text.is_empty() => titles.push((index, text, pointer)),
            Some(_) => {}
            None => emit(
                findings,
                rules::DESIGN_TITLE_MISSING,
                visual,
                page,
                index,
                "/visual/visualContainerObjects/title",
                json!({"visualType": kind}),
            ),
        }
        if ranking
            && (kind.to_ascii_lowercase().contains("bar")
                || kind.to_ascii_lowercase().contains("column"))
        {
            let bindings = visual["bindings"].as_array();
            let measures = bindings
                .into_iter()
                .flatten()
                .filter(|binding| binding["kind"] == "measure")
                .collect::<Vec<_>>();
            if !measures.is_empty()
                && !measures.iter().any(|binding| {
                    binding["sortDirection"]
                        .as_str()
                        .is_some_and(|direction| direction.eq_ignore_ascii_case("Descending"))
                })
            {
                emit(
                    findings,
                    rules::DESIGN_RANKING_NOT_SORTED,
                    visual,
                    page,
                    index,
                    "/visual/query/sortDefinition",
                    json!({"template": "ranking", "requiredDirection": "Descending"}),
                );
            }
        }
    }
    let mut duplicates = BTreeMap::<String, Vec<usize>>::new();
    let mut styles = BTreeMap::<&str, usize>::new();
    for (index, text, _) in &titles {
        duplicates
            .entry(
                text.split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase(),
            )
            .or_default()
            .push(*index);
        if let Some(style) = casing(text) {
            *styles.entry(style).or_default() += 1;
        }
    }
    let majority = styles
        .iter()
        .max_by(|(a, ac), (b, bc)| ac.cmp(bc).then_with(|| b.cmp(a)))
        .map(|(style, _)| *style);
    for (index, text, pointer) in titles {
        let key = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if duplicates[&key].len() > 1 {
            emit(
                findings,
                rules::DESIGN_TITLE_DUPLICATE,
                &visuals[index],
                page,
                index,
                &pointer,
                json!({"title": text, "visualIndexes": duplicates[&key]}),
            );
        }
        if let Some(style) = casing(&text)
            && Some(style) != majority
        {
            emit(
                findings,
                rules::DESIGN_TITLE_CASE_INCONSISTENT,
                &visuals[index],
                page,
                index,
                &pointer,
                json!({"title": text, "style": style, "pageStyle": majority}),
            );
        }
    }
}

pub(crate) fn lint_measure_formats(deep: &Value, findings: &mut Vec<Value>) {
    let mut used = BTreeSet::new();
    for page in deep["report"]["pages"].as_array().into_iter().flatten() {
        for visual in page["visuals"].as_array().into_iter().flatten() {
            for binding in visual["bindings"].as_array().into_iter().flatten() {
                if let (Some(table), Some(measure)) =
                    (binding["table"].as_str(), binding["measure"].as_str())
                {
                    used.insert((table.to_lowercase(), measure.to_lowercase()));
                }
            }
        }
    }
    for (table_index, table) in deep["model"]["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        for (index, measure) in table["measures"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if used.contains(&(
                table["name"].as_str().unwrap_or_default().to_lowercase(),
                measure["name"].as_str().unwrap_or_default().to_lowercase(),
            )) && nonempty(&measure["properties"]["formatString"]).is_none()
                && nonempty(&measure["properties"]["formatStringDefinition"]).is_none()
            {
                findings.push(design_finding(
                    rules::DESIGN_NUMBER_FORMAT_MISSING,
                    measure["handle"].as_str(),
                    table["path"].as_str().map(Path::new),
                    format!("/model/tables/{table_index}/measures/{index}/properties/formatString"),
                    "A report-bound measure has no static or dynamic number format.".to_string(),
                    json!({"table": table["name"], "measure": measure["name"]}),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visual() -> Value {
        json!({"visualType": "barChart", "handle": "visual:P:V", "bindings": [{"kind":"measure", "sortDirection":"Descending"}]})
    }
    fn raw(text: &str) -> Value {
        json!({"visual":{"visualContainerObjects":{"title":[{"properties":{"text":{"expr":{"Literal":{"Value":format!("'{text}'")}}},"show":{"expr":{"Literal":{"Value":"true"}}}}}]}}})
    }
    fn codes(titles: &[Value], ranking: bool, visual: Value) -> Vec<Value> {
        let mut findings = Vec::new();
        lint_visuals(
            &vec![visual; titles.len()],
            titles,
            0,
            ranking,
            &mut findings,
        );
        findings
    }
    fn has(findings: &[Value], id: &str) -> bool {
        findings.iter().any(|f| f["code"] == id)
    }

    #[test]
    fn missing_title_uses_persisted_visibility_not_placeholder_names() {
        for absent in [
            raw(""),
            json!({"annotations":[{"name":"powerbi-cli.placeholderTitle","value":"Named"}]}),
            {
                let mut raw = raw("Hidden");
                raw["visual"]["visualContainerObjects"]["title"][0]["properties"]["show"]["expr"]
                    ["Literal"]["Value"] = json!("false");
                raw
            },
        ] {
            assert!(has(
                &codes(&[absent], false, visual()),
                rules::DESIGN_TITLE_MISSING
            ));
        }
        assert!(!has(
            &codes(&[raw("Visible")], false, visual()),
            rules::DESIGN_TITLE_MISSING
        ));
        let mut dynamic = raw("Visible");
        dynamic["visual"]["visualContainerObjects"]["title"][0]["properties"]["text"] =
            json!({"expr":{"Measure":{"Property":"Title"}}});
        assert!(!has(
            &codes(&[dynamic], false, visual()),
            rules::DESIGN_TITLE_MISSING
        ));
        let mut textbox = visual();
        textbox["visualType"] = json!("textbox");
        assert!(!has(
            &codes(&[json!({})], false, textbox),
            rules::DESIGN_TITLE_MISSING
        ));
    }

    #[test]
    fn duplicate_titles_are_page_local_case_and_whitespace_normalized() {
        assert!(has(
            &codes(
                &[raw("Total revenue"), raw(" total   REVENUE ")],
                false,
                visual()
            ),
            rules::DESIGN_TITLE_DUPLICATE
        ));
        assert!(!has(
            &codes(&[raw("Total revenue"), raw("Total cost")], false, visual()),
            rules::DESIGN_TITLE_DUPLICATE
        ));
    }

    #[test]
    fn casing_ignores_single_words_and_acronyms_and_finds_mixed_conventions() {
        assert!(!has(
            &codes(
                &[raw("Total revenue"), raw("Total cost"), raw("KPI")],
                false,
                visual()
            ),
            rules::DESIGN_TITLE_CASE_INCONSISTENT
        ));
        let found = codes(
            &[raw("Total revenue"), raw("Total cost"), raw("Total Profit")],
            false,
            visual(),
        );
        assert!(has(&found, rules::DESIGN_TITLE_CASE_INCONSISTENT));
        assert!(found.iter().all(|f| {
            f["pointer"]
                .as_str()
                .unwrap()
                .starts_with("/report/pages/0/visuals/")
        }));
    }

    #[test]
    fn ranking_requires_descending_measure_sort_only_on_ranking_pages() {
        assert!(!has(
            &codes(&[raw("Revenue")], true, visual()),
            rules::DESIGN_RANKING_NOT_SORTED
        ));
        let mut missing = visual();
        missing["bindings"][0]["sortDirection"] = Value::Null;
        assert!(has(
            &codes(&[raw("Revenue")], true, missing.clone()),
            rules::DESIGN_RANKING_NOT_SORTED
        ));
        assert!(!has(
            &codes(&[raw("Revenue")], false, missing),
            rules::DESIGN_RANKING_NOT_SORTED
        ));
    }

    #[test]
    fn report_bound_measures_accept_static_and_dynamic_formats_and_use_array_pointers() {
        let mut deep = json!({"report":{"pages":[{"visuals":[{"bindings":[{"table":"T/~", "measure":"M/~"}]}]}]},
            "model":{"tables":[{"name":"T/~", "measures":[{"name":"M/~", "handle":"measure:T%2F~:M%2F~", "properties":{"formatString":"#,##0"}}]}]}});
        let mut findings = Vec::new();
        lint_measure_formats(&deep, &mut findings);
        assert!(findings.is_empty());
        deep["model"]["tables"][0]["measures"][0]["properties"] =
            json!({"formatStringDefinition":"\"0.0%\""});
        lint_measure_formats(&deep, &mut findings);
        assert!(findings.is_empty());
        deep["model"]["tables"][0]["measures"][0]["properties"] = json!({});
        lint_measure_formats(&deep, &mut findings);
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0]["pointer"],
            "/model/tables/0/measures/0/properties/formatString"
        );
    }
}
