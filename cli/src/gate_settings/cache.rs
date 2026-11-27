use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::Capabilities;

/// Settings `scripts/gate_results.py` takes from `[quality.cache]`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CacheSettings {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub never: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u16>,
}

/// Steps that read data a commit does not change, so a cached pass would go stale.
fn never_cached(capabilities: &Capabilities) -> Vec<&'static str> {
    let mut steps = vec!["secret-scan", "osv"];
    if capabilities.components.backend() {
        steps.extend(["cargo-deny", "migrations"]);
    }
    steps
}

pub(super) fn render_cache(capabilities: &Capabilities) -> String {
    let never = never_cached(capabilities)
        .iter()
        .map(|step| format!("\"{step}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[quality.cache]\nnever = [{never}]\n")
}

pub(super) fn validate_never(
    cache: &CacheSettings,
    steps: &BTreeSet<String>,
    failures: &mut Vec<String>,
) {
    for name in &cache.never {
        if !steps.contains(name) {
            failures.push(format!("quality.cache.never names unknown step `{name}`"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{tests::capabilities, validate};
    use crate::QualityManifest;

    #[test]
    fn cache_never_names_must_be_gate_steps() {
        let caps = capabilities(false, true, false);
        let quality: QualityManifest =
            toml::from_str("[cache]\nnever = [\"osv\", \"cargo-deny\", \"nope\"]\n")
                .expect("quality parses");
        let mut failures = Vec::new();
        validate(&quality, &caps, None, &mut failures);
        assert_eq!(
            failures,
            [
                "quality.cache.never names unknown step `cargo-deny`",
                "quality.cache.never names unknown step `nope`",
            ]
        );
    }
}
