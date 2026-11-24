use super::{
    DEPLOY_VALUES_PROBE, DEPLOY_VALUES_STEP, GATE_SCRIPT, GateSteps, STRICT_GATE_TEMPLATE,
    glob_matches,
};
const LOCAL_GATE_SCRIPT: &str = "scripts/quality-gate.local.sh";

use crate::{Capabilities, QualityManifest, QualityProfile};

pub(super) fn validate_deploy_glob(
    quality: &QualityManifest,
    capabilities: &Capabilities,
    gate: Option<&GateSteps>,
    failures: &mut Vec<String>,
) {
    if !capabilities.components.backend() {
        return;
    }
    match gate {
        Some(gate) if !gate.steps.contains(DEPLOY_VALUES_STEP) => {
            failures.push(format!(
                "{GATE_SCRIPT} has no `{DEPLOY_VALUES_STEP}` step; copy `step_deploy_values` and its \
                 `{DEPLOY_VALUES_STEP} fast - selected -` step line from baukit's \
                 {STRICT_GATE_TEMPLATE}; a product that overrides `list_steps` in \
                 {LOCAL_GATE_SCRIPT} adds the step line there"
            ));
            return;
        }
        None if quality.profile != QualityProfile::Strict => return,
        _ => {}
    }
    let matching: Vec<(&String, &Vec<String>)> = quality
        .affected
        .iter()
        .filter(|(glob, _)| glob_matches(glob, DEPLOY_VALUES_PROBE))
        .collect();
    let selects_step = matching.iter().any(|(_, tokens)| {
        tokens
            .iter()
            .any(|token| token == DEPLOY_VALUES_STEP || token == "fast")
    });
    if matching.is_empty() || selects_step {
        return;
    }
    let globs: Vec<String> = matching
        .iter()
        .map(|(glob, _)| format!("`{glob}`"))
        .collect();
    failures.push(format!(
        "quality.affected {} matches {DEPLOY_VALUES_PROBE} without selecting `{DEPLOY_VALUES_STEP}`; add it so deploy changes render the chart",
        globs.join(", ")
    ));
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::{tests::capabilities, validate};
    use super::*;

    #[test]
    fn deploy_glob_rule_follows_the_gate_that_runs_it() {
        let deploy_failures = |text: &str, gate: Option<&GateSteps>| {
            let quality: QualityManifest = toml::from_str(text).expect("quality parses");
            let mut failures = Vec::new();
            validate_deploy_glob(
                &quality,
                &capabilities(true, false, false),
                gate,
                &mut failures,
            );
            failures
        };
        let stale = "[affected]\n\"deploy/**\" = []\n";
        assert!(deploy_failures(stale, None).is_empty());
        let without_step = GateSteps {
            steps: BTreeSet::from(["rust-tests".to_owned()]),
            ..GateSteps::default()
        };
        assert_eq!(
            deploy_failures(stale, Some(&without_step)),
            [
                "scripts/quality-gate.sh has no `deploy-values` step; copy `step_deploy_values` and its `deploy-values fast - selected -` step line from baukit's templates/common/__strict__/scripts/quality-gate.sh; a product that overrides `list_steps` in scripts/quality-gate.local.sh adds the step line there"
            ]
        );
        let with_step = GateSteps {
            steps: BTreeSet::from([DEPLOY_VALUES_STEP.to_owned()]),
            ..GateSteps::default()
        };
        assert_eq!(deploy_failures(stale, Some(&with_step)).len(), 1);
        assert!(
            deploy_failures(
                "[affected]\n\"deploy/**\" = [\"deploy-values\"]\n",
                Some(&with_step)
            )
            .is_empty()
        );
    }

    #[test]
    fn deploy_globs_must_select_deploy_values() {
        let deploy_failures = |text: &str, caps: &Capabilities| {
            let quality: QualityManifest =
                toml::from_str(&format!("profile = \"strict\"\n{text}")).expect("quality parses");
            let mut failures = Vec::new();
            validate(&quality, caps, None, &mut failures);
            failures
                .into_iter()
                .filter(|failure| failure.contains("deploy-values"))
                .collect::<Vec<_>>()
        };
        let backend = capabilities(true, false, false);
        assert_eq!(
            deploy_failures(
                "[affected]\n\"deploy/**\" = []\n\"docs/**\" = []\n",
                &backend
            ),
            [
                "quality.affected `deploy/**` matches deploy/gitops/base/helmrelease.yaml without selecting `deploy-values`; add it so deploy changes render the chart"
            ]
        );
        for text in [
            "[affected]\n\"deploy/**\" = [\"deploy-values\"]\n",
            "[affected]\n\"deploy/**\" = [\"images\"]\n\"deploy/gitops/**\" = [\"fast\"]\n",
            "[affected]\n\"docs/**\" = []\n",
        ] {
            assert!(deploy_failures(text, &backend).is_empty(), "{text}");
        }
        assert!(
            deploy_failures(
                "[affected]\n\"deploy/**\" = []\n",
                &capabilities(false, true, false)
            )
            .is_empty()
        );
    }
}
