use std::collections::BTreeSet;

use serde_json::Value;

/// Grafana's built-in datasources, valid in every dashboard.
const BUILTIN_DATASOURCE_UIDS: &[&str] = &["-- Grafana --", "-- Mixed --", "-- Dashboard --"];

/// Every string `expr` in a dashboard, with its JSON path in check-metric-names.py's notation.
pub fn expressions(document: &Value, location: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    visit(document, location, &mut |key, child, path| {
        if key == "expr"
            && let Value::String(expression) = child
        {
            found.push((path.to_owned(), expression.clone()));
        }
    });
    found
}

/// Datasource references that name neither a declared datasource variable nor an expected uid.
pub fn datasource_problems(
    document: &Value,
    location: &str,
    expected_uids: &[String],
) -> Vec<(String, String)> {
    let variables = datasource_variables(document);
    let mut problems = Vec::new();
    visit(document, location, &mut |key, child, path| {
        if key != "datasource" || child.is_null() {
            return;
        }
        let uid = match child {
            Value::Object(fields) => fields.get("uid").and_then(Value::as_str),
            other => other.as_str(),
        };
        let problem = match uid {
            None | Some("") => {
                Some("datasource reference needs a variable or a fixed uid".to_owned())
            }
            Some(uid) => uid_problem(uid, &variables, expected_uids),
        };
        if let Some(problem) = problem {
            problems.push((path.to_owned(), problem));
        }
    });
    problems
}

fn uid_problem(
    uid: &str,
    variables: &BTreeSet<String>,
    expected_uids: &[String],
) -> Option<String> {
    if let Some(name) = variable_name(uid) {
        return (!variables.contains(name)).then(|| {
            format!("datasource variable `{name}` is not declared in templating or __inputs")
        });
    }
    let expected =
        BUILTIN_DATASOURCE_UIDS.contains(&uid) || expected_uids.iter().any(|known| known == uid);
    (!expected).then(|| {
        format!("datasource uid `{uid}` is neither a datasource variable nor in observability.datasource-uids")
    })
}

fn variable_name(uid: &str) -> Option<&str> {
    uid.strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
        .or_else(|| uid.strip_prefix('$'))
}

/// Names of `templating.list` entries and `__inputs` of type `datasource`.
fn datasource_variables(document: &Value) -> BTreeSet<String> {
    let templating = document
        .pointer("/templating/list")
        .and_then(Value::as_array);
    let inputs = document.get("__inputs").and_then(Value::as_array);
    templating
        .into_iter()
        .chain(inputs)
        .flatten()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("datasource"))
        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn visit(value: &Value, location: &str, action: &mut impl FnMut(&str, &Value, &str)) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let path = format!("{location}.{key}");
                action(key, child, &path);
                visit(child, &path, action);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                visit(child, &format!("{location}[{index}]"), action);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn expressions_carry_their_json_path() {
        let dashboard = json!({"panels": [{"targets": [{"expr": "up"}]}]});
        assert_eq!(
            expressions(&dashboard, "d.json"),
            vec![(
                "d.json.panels[0].targets[0].expr".to_owned(),
                "up".to_owned()
            )]
        );
    }

    #[test]
    fn datasources_need_a_declared_variable_or_an_expected_uid() {
        let dashboard = json!({
            "templating": {"list": [{"name": "DS_PROMETHEUS", "type": "datasource"}]},
            "panels": [
                {"datasource": {"uid": "${DS_PROMETHEUS}"}},
                {"datasource": {"uid": "$DS_LOKI"}},
                {"datasource": {"uid": "prometheus"}},
                {"datasource": {"type": "prometheus"}},
                {"datasource": "-- Grafana --"},
                {"datasource": null}
            ]
        });
        let problems = datasource_problems(&dashboard, "d", &["prometheus".to_owned()]);
        assert_eq!(
            problems,
            vec![
                (
                    "d.panels[1].datasource".to_owned(),
                    "datasource variable `DS_LOKI` is not declared in templating or __inputs"
                        .to_owned()
                ),
                (
                    "d.panels[3].datasource".to_owned(),
                    "datasource reference needs a variable or a fixed uid".to_owned()
                ),
            ]
        );
    }
}
