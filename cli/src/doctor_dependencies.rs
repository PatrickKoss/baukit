use std::{fs, path::Path};

use anyhow::{Context, Result};

use crate::doctor_layout;

struct Pin {
    name: String,
    version: String,
    kind: PinKind,
}

enum PinKind {
    Requirement,
    Resolution,
    PnpmResolution,
}

impl Pin {
    fn new(name: &str, version: &str, kind: PinKind) -> Self {
        Self {
            name: name.to_owned(),
            version: version.to_owned(),
            kind,
        }
    }

    fn matches(&self, expected: &str) -> bool {
        match self.kind {
            PinKind::Requirement => {
                self.version.strip_prefix('=').unwrap_or(&self.version) == expected
            }
            PinKind::Resolution => self.version == expected,
            PinKind::PnpmResolution => self.version.split('(').next() == Some(expected),
        }
    }
}

pub(super) fn validate_registry_pins(
    root: &Path,
    expected: &str,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let initial = failures.len();
    let mut checked = 0;
    for extension in ["toml", "lock", "json", "yaml"] {
        for path in doctor_layout::product_files(root, extension)? {
            let filename = path.file_name().and_then(|name| name.to_str());
            if !matches!(
                filename,
                Some("Cargo.toml" | "Cargo.lock" | "package.json" | "pnpm-lock.yaml")
            ) {
                continue;
            }
            let relative = path.strip_prefix(root)?.display().to_string();
            let source = fs::read_to_string(&path)?;
            let pins = read_pins(filename, &source)
                .with_context(|| format!("could not read Baukit pins in `{relative}`"))?;
            for pin in pins {
                checked += 1;
                if !pin.matches(expected) {
                    failures.push(format!(
                        "`{relative}`: {} pin `{}` differs from Baukit registry version {expected}",
                        pin.name, pin.version
                    ));
                }
            }
        }
    }
    if failures.len() == initial {
        successes.push(format!(
            "{checked} Baukit registry pins match version {expected}"
        ));
    }
    Ok(())
}

fn read_pins(filename: Option<&str>, source: &str) -> Result<Vec<Pin>> {
    let mut pins = Vec::new();
    match filename {
        Some("Cargo.toml") => cargo_dependencies(&toml::from_str(source)?, &mut pins),
        Some("Cargo.lock") => {
            let lock: toml::Value = toml::from_str(source)?;
            if let Some(packages) = lock.get("package").and_then(toml::Value::as_array) {
                for package in packages {
                    let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
                        continue;
                    };
                    if name.starts_with("baukit-") {
                        let version = package
                            .get("version")
                            .and_then(toml::Value::as_str)
                            .unwrap_or("<missing version>");
                        pins.push(Pin::new(name, version, PinKind::Resolution));
                    }
                }
            }
        }
        Some("package.json") => npm_dependencies(&serde_json::from_str(source)?, &mut pins),
        Some("pnpm-lock.yaml") => pnpm_pins(&serde_yaml_ng::from_str(source)?, &mut pins),
        _ => {}
    }
    Ok(pins)
}

fn cargo_dependencies(value: &toml::Value, pins: &mut Vec<Pin>) {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dependencies) = value.get(section) {
            cargo_dependency_table(dependencies, pins);
        }
    }
    if let Some(dependencies) = value
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
    {
        cargo_dependency_table(dependencies, pins);
    }
    if let Some(targets) = value.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            cargo_dependencies(target, pins);
        }
    }
}

fn cargo_dependency_table(value: &toml::Value, pins: &mut Vec<Pin>) {
    let Some(dependencies) = value.as_table() else {
        return;
    };
    for (alias, dependency) in dependencies {
        let name = dependency
            .get("package")
            .and_then(toml::Value::as_str)
            .unwrap_or(alias);
        if !name.starts_with("baukit-")
            || dependency.get("workspace").and_then(toml::Value::as_bool) == Some(true)
        {
            continue;
        }
        let version = dependency
            .as_str()
            .or_else(|| dependency.get("version").and_then(toml::Value::as_str))
            .unwrap_or("<missing registry version>");
        pins.push(Pin::new(name, version, PinKind::Requirement));
    }
}

fn npm_dependencies(value: &serde_json::Value, pins: &mut Vec<Pin>) {
    for key in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
        "overrides",
        "resolutions",
    ] {
        if let Some(dependencies) = value.get(key) {
            npm_pins(dependencies, pins);
        }
    }
    if let Some(pnpm) = value.get("pnpm") {
        npm_dependencies(pnpm, pins);
    }
}

fn npm_pins(value: &serde_json::Value, pins: &mut Vec<Pin>) {
    let Some(dependencies) = value.as_object() else {
        return;
    };
    for (name, value) in dependencies {
        if let Some(pin) = value.as_str().and_then(npm_alias_pin) {
            pins.push(pin);
            continue;
        }
        if value.is_object() {
            if let Some(version) = value
                .get(".")
                .and_then(serde_json::Value::as_str)
                .filter(|version| name.starts_with("@baukit/") && npm_alias_pin(version).is_none())
            {
                pins.push(Pin::new(name, version, PinKind::Requirement));
            }
            npm_pins(value, pins);
            continue;
        }
        if name.starts_with("@baukit/") {
            pins.push(Pin::new(
                name,
                value.as_str().unwrap_or("<missing registry version>"),
                PinKind::Requirement,
            ));
        }
    }
}

fn npm_alias_pin(value: &str) -> Option<Pin> {
    let (name, version) = value.strip_prefix("npm:")?.rsplit_once('@')?;
    name.starts_with("@baukit/")
        .then(|| Pin::new(name, version, PinKind::Requirement))
}

fn pnpm_resolution_pin(value: &str) -> Option<Pin> {
    let (name, version) = value.strip_prefix("@baukit/")?.split_once('@')?;
    Some(Pin::new(
        &format!("@baukit/{name}"),
        version,
        PinKind::PnpmResolution,
    ))
}

fn pnpm_pins(value: &serde_yaml_ng::Value, pins: &mut Vec<Pin>) {
    let Some(mapping) = value.as_mapping() else {
        return;
    };
    for (key, value) in mapping {
        let Some(key) = key.as_str() else { continue };
        if let Some(pin) = value
            .as_str()
            .and_then(|value| npm_alias_pin(value).or_else(|| pnpm_resolution_pin(value)))
        {
            pins.push(pin);
            continue;
        }
        let name = key.trim_start_matches('/');
        if !name.starts_with("@baukit/") {
            pnpm_pins(value, pins);
            continue;
        }
        if let Some(pin) = pnpm_resolution_pin(name) {
            pins.push(pin);
        } else if let Some(version) = value.as_str() {
            pins.push(Pin::new(name, version, PinKind::PnpmResolution));
        } else {
            for (key, kind) in [
                ("specifier", PinKind::Requirement),
                ("version", PinKind::PnpmResolution),
            ] {
                if let Some(version) = value.get(key).and_then(serde_yaml_ng::Value::as_str) {
                    pins.push(Pin::new(name, version, kind));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::read_pins;

    #[test]
    fn peer_suffixes_are_only_valid_in_pnpm_resolutions() -> anyhow::Result<()> {
        let npm = read_pins(
            Some("package.json"),
            r#"{"dependencies":{"@baukit/events":"0.10.2(react@19.2.3)"}}"#,
        )?;
        assert_eq!(npm.len(), 1);
        assert!(!npm[0].matches("0.10.2"));

        let pnpm = read_pins(
            Some("pnpm-lock.yaml"),
            "importers:\n  .:\n    dependencies:\n      '@baukit/events':\n        specifier: 0.10.2(react@19.2.3)\n        version: 0.10.2(react@19.2.3)\n",
        )?;
        assert_eq!(pnpm.len(), 2);
        assert!(!pnpm[0].matches("0.10.2"));
        assert!(pnpm[1].matches("0.10.2"));
        Ok(())
    }

    #[test]
    fn cargo_census_checks_aliases_and_target_dependency_tables() -> anyhow::Result<()> {
        let pins = read_pins(
            Some("Cargo.toml"),
            r#"
[package]
name = "fixture"
version = "0.1.0"
[workspace.dependencies]
baukit-core = "0.10.2"
[dependencies]
baukit-core = { workspace = true }
[dev-dependencies]
kit = { package = "baukit-test", version = "=0.10.2" }
[target.'cfg(unix)'.build-dependencies]
baukit-config = { version = "0.9.0" }
serde = "1"
[package.metadata.example.dependencies]
baukit-core = "0.1.0"
"#,
        )?;
        assert_eq!(pins.len(), 3);
        let mismatches = pins
            .iter()
            .filter(|pin| !pin.matches("0.10.2"))
            .map(|pin| (pin.name.as_str(), pin.version.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(mismatches, [("baukit-config", "0.9.0")]);
        Ok(())
    }

    #[test]
    fn npm_census_checks_optional_peer_and_nested_override_pins() -> anyhow::Result<()> {
        let pins = read_pins(
            Some("package.json"),
            r#"{
          "dependencies": {"@baukit/api-runtime": "0.10.2", "react": "19.2.3"},
          "devDependencies": {"@baukit/auth-node": "0.9.0"},
          "optionalDependencies": {"@baukit/analytics-core": "0.10.2"},
          "peerDependencies": {"@baukit/ui-tokens": "0.10.2"},
          "overrides": {"other": {"@baukit/api-runtime": "0.9.0"}},
          "pnpm": {"overrides": {"@baukit/events": "0.10.2"}}
        }"#,
        )?;
        assert_eq!(pins.len(), 6);
        let mismatches = pins
            .iter()
            .filter(|pin| !pin.matches("0.10.2"))
            .map(|pin| pin.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(mismatches, ["@baukit/auth-node", "@baukit/api-runtime"]);
        Ok(())
    }

    #[test]
    fn npm_census_checks_aliases_and_package_override_objects() -> anyhow::Result<()> {
        let source = r#"{
          "dependencies": {"kit": "npm:@baukit/events@0.10.2"},
          "overrides": {
            "@baukit/events": {
              ".": "0.10.2",
              "@baukit/ui-tokens": "0.10.2"
            }
          }
        }"#;
        let pins = read_pins(Some("package.json"), source)?;
        assert_eq!(pins.len(), 3);
        assert!(pins.iter().all(|pin| pin.matches("0.10.2")));
        for (source, count) in [
            (source.replace("\".\": \"0.10.2\",", ""), 2),
            (
                source.replace("\".\": \"0.10.2\"", "\".\": \"npm:@baukit/events@0.10.2\""),
                3,
            ),
        ] {
            let pins = read_pins(Some("package.json"), &source)?;
            assert_eq!(pins.len(), count);
            assert!(pins.iter().all(|pin| pin.matches("0.10.2")));
        }
        let pins = read_pins(Some("package.json"), &source.replace("0.10.2", "0.9.0"))?;
        assert_eq!(pins.len(), 3);
        assert!(pins.iter().all(|pin| !pin.matches("0.10.2")));
        assert_eq!(pins[0].name, "@baukit/events");
        assert_eq!(pins[1].name, "@baukit/events");
        assert_eq!(pins[2].name, "@baukit/ui-tokens");
        Ok(())
    }

    #[test]
    fn pnpm_census_checks_alias_specifiers() -> anyhow::Result<()> {
        let source = "importers:\n  .:\n    dependencies:\n      kit:\n        specifier: npm:@baukit/events@0.10.2\n        version: '@baukit/events@0.10.2'\npackages:\n  '@baukit/events@0.10.2': {}\n";
        let pins = read_pins(Some("pnpm-lock.yaml"), source)?;
        assert_eq!(pins.len(), 3);
        assert!(pins.iter().all(|pin| pin.matches("0.10.2")));
        let pins = read_pins(
            Some("pnpm-lock.yaml"),
            &source.replacen("npm:@baukit/events@0.10.2", "npm:@baukit/events@0.9.0", 1),
        )?;
        assert_eq!(pins.len(), 3);
        assert_eq!(pins[0].name, "@baukit/events");
        assert!(!pins[0].matches("0.10.2"));
        assert!(pins[1].matches("0.10.2"));
        assert!(pins[2].matches("0.10.2"));
        let pins = read_pins(
            Some("pnpm-lock.yaml"),
            &source.replacen(
                "version: '@baukit/events@0.10.2'",
                "version: '@baukit/events@0.9.0'",
                1,
            ),
        )?;
        assert_eq!(pins.len(), 3);
        assert!(pins[0].matches("0.10.2"));
        assert_eq!(pins[1].name, "@baukit/events");
        assert!(!pins[1].matches("0.10.2"));
        assert!(pins[2].matches("0.10.2"));
        Ok(())
    }
}
