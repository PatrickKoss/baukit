use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::Manifest;

pub const WEB_PORT: u16 = 5173;
pub const LOCAL_CALLBACK_PORT: u16 = 18890;
pub const WEB_PREVIEW_PORT: u16 = 4173;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortMapping {
    pub host: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PortDeclaration {
    HostList(Vec<u16>),
    Mapping(PortMapping),
}

impl PortDeclaration {
    pub fn as_mapping(&self) -> Option<&PortMapping> {
        match self {
            Self::Mapping(mapping) => Some(mapping),
            Self::HostList(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PortConfiguration {
    pub postgres: u16,
    pub api: u16,
    pub ops: u16,
    pub keycloak: u16,
    pub fake_provider: u16,
    pub redis: u16,
    pub web: Option<u16>,
    pub web_preview: Option<u16>,
    pub web_extra: Vec<u16>,
    pub keycloak_extra: Vec<u16>,
    pub metro: Option<u16>,
    pub local_callback: Option<u16>,
    pub offset: u32,
    pub service_ports_declared: bool,
    pub api_container: u16,
    pub ops_container: u16,
}

pub(crate) fn is_dev_port(name: &str) -> bool {
    matches!(
        name,
        "web" | "web_preview" | "web_extra" | "metro" | "local_callback"
    )
}

impl PortConfiguration {
    pub fn from_manifest(manifest: &Manifest) -> Result<Self> {
        let mut ports = Self::new(manifest.port_offset)?;
        for (name, declaration) in &manifest.ports {
            if let Some(list) = match name.as_str() {
                "web_extra" => Some(&mut ports.web_extra),
                "keycloak_extra" => Some(&mut ports.keycloak_extra),
                _ => None,
            } {
                let PortDeclaration::HostList(hosts) = declaration else {
                    bail!("ports.{name} must be a list of host ports");
                };
                if hosts.is_empty() {
                    bail!("ports.{name} must not be empty");
                }
                if hosts.contains(&0) {
                    bail!("ports.{name} must use nonzero host ports");
                }
                list.clone_from(hosts);
                continue;
            }
            let PortDeclaration::Mapping(mapping) = declaration else {
                bail!("ports.{name} must declare `host` in a port mapping");
            };
            if is_dev_port(name) {
                ports.set_dev_port(name, mapping)?;
                continue;
            }
            let (host, fixed_container) = match name.as_str() {
                "postgres" => (&mut ports.postgres, Some(5432)),
                "api" => (&mut ports.api, None),
                "ops" => (&mut ports.ops, None),
                "keycloak" => (&mut ports.keycloak, Some(8080)),
                "fake_provider" => (&mut ports.fake_provider, None),
                "redis" => (&mut ports.redis, Some(6379)),
                _ => bail!("unknown port declaration `ports.{name}`"),
            };
            let container = mapping.container.with_context(|| {
                format!("ports.{name}.container is required for a container service")
            })?;
            if mapping.host == 0 || container == 0 {
                bail!("ports.{name} must use nonzero host and container ports");
            }
            if let Some(fixed) = fixed_container
                && fixed != container
            {
                bail!("ports.{name}.container must be {fixed} for the generated image");
            }
            *host = mapping.host;
            ports.service_ports_declared = true;
            match name.as_str() {
                "api" => ports.api_container = container,
                "ops" => ports.ops_container = container,
                _ => {}
            }
        }
        if !ports.web_extra.is_empty() && ports.web.is_none() {
            bail!("ports.web_extra requires ports.web");
        }
        ports.validate_unique_hosts()?;
        Ok(ports)
    }

    fn set_dev_port(&mut self, name: &str, mapping: &PortMapping) -> Result<()> {
        if mapping.container.is_some() || mapping.service.is_some() {
            bail!(
                "ports.{name} accepts only `host`; `container` and `service` are not supported for dev servers"
            );
        }
        if mapping.host == 0 {
            bail!("ports.{name}.host must be nonzero");
        }
        let target = match name {
            "web" => &mut self.web,
            "web_preview" => &mut self.web_preview,
            "metro" => &mut self.metro,
            "local_callback" => &mut self.local_callback,
            _ => unreachable!("dev port name was checked"),
        };
        *target = Some(mapping.host);
        Ok(())
    }

    pub fn local_callback_port(&self) -> u16 {
        self.local_callback.unwrap_or(LOCAL_CALLBACK_PORT)
    }

    pub fn new(offset: u32) -> Result<Self> {
        let shifted = |name, base: u32| {
            base.checked_add(offset)
                .and_then(|port| u16::try_from(port).ok())
                .ok_or_else(|| {
                    anyhow!(
                        "--port-offset {offset} puts the {name} port above {}",
                        u16::MAX
                    )
                })
        };
        Ok(Self {
            postgres: shifted("PostgreSQL", 5432)?,
            api: shifted("API", 8080)?,
            ops: shifted("operations", 9090)?,
            keycloak: shifted("Keycloak", 8081)?,
            fake_provider: shifted("fake provider", 18081)?,
            redis: shifted("Redis", 6379)?,
            web: None,
            web_preview: None,
            web_extra: Vec::new(),
            keycloak_extra: Vec::new(),
            metro: None,
            local_callback: None,
            offset,
            service_ports_declared: false,
            api_container: shifted("API", 8080)?,
            ops_container: shifted("operations", 9090)?,
        })
    }

    fn validate_unique_hosts(&self) -> Result<()> {
        let mut seen = BTreeMap::new();
        for (name, port) in [
            ("postgres", Some(self.postgres)),
            ("api", Some(self.api)),
            ("ops", Some(self.ops)),
            ("keycloak", Some(self.keycloak)),
            ("fake_provider", Some(self.fake_provider)),
            ("redis", Some(self.redis)),
            ("web", self.web),
            ("web_preview", self.web_preview),
            ("metro", self.metro),
            ("local_callback", self.local_callback),
        ] {
            if let Some(port) = port
                && let Some(previous) = seen.insert(port, name)
            {
                bail!("ports.{name} and ports.{previous} collide on effective host port {port}");
            }
        }
        for (name, list) in [
            ("web_extra", &self.web_extra),
            ("keycloak_extra", &self.keycloak_extra),
        ] {
            for port in list {
                if let Some(previous) = seen.insert(*port, name) {
                    if previous == name {
                        bail!("ports.{name} contains duplicate host port {port}");
                    }
                    bail!(
                        "ports.{name} and ports.{previous} collide on effective host port {port}"
                    );
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn generation_ports(
    destination: &Path,
    offset: u32,
) -> Result<(PortConfiguration, Option<String>)> {
    let path = destination.join("baukit.toml");
    if !path.is_file() {
        return Ok((PortConfiguration::new(offset)?, None));
    }
    let manifest = crate::read_manifest(destination)?;
    Ok((
        PortConfiguration::from_manifest(&manifest)?,
        Some(fs::read_to_string(path)?),
    ))
}
