use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, IgnoredAny, MapAccess, Visitor},
    ser::SerializeMap,
};

/// A generated product part, selected by the `--<key>` flag of `baukit new`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Component {
    Backend,
    Worker,
    Mobile,
    Web,
    Pwa,
    Mcp,
    Platform,
}

impl Component {
    pub const ALL: [Self; 7] = [
        Self::Backend,
        Self::Worker,
        Self::Mobile,
        Self::Web,
        Self::Pwa,
        Self::Mcp,
        Self::Platform,
    ];

    /// Components that `baukit.toml` must list explicitly under `[capabilities]`.
    const REQUIRED: [Self; 3] = [Self::Backend, Self::Mobile, Self::Web];

    /// The flag, manifest key, and template variable name.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Worker => "worker",
            Self::Mobile => "mobile",
            Self::Web => "web",
            Self::Pwa => "pwa",
            Self::Mcp => "mcp",
            Self::Platform => "platform",
        }
    }

    const fn bit(self) -> u8 {
        match self {
            Self::Backend => 1,
            Self::Worker => 1 << 1,
            Self::Mobile => 1 << 2,
            Self::Web => 1 << 3,
            Self::Pwa => 1 << 4,
            Self::Mcp => 1 << 5,
            Self::Platform => 1 << 6,
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|component| component.key() == key)
    }
}

/// The set of enabled components. Serializes as one boolean per component key.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Components(u8);

impl Components {
    #[must_use]
    pub const fn contains(self, component: Component) -> bool {
        self.0 & component.bit() != 0
    }

    #[must_use]
    pub const fn with(self, component: Component) -> Self {
        Self(self.0 | component.bit())
    }

    pub const fn insert(&mut self, component: Component) {
        self.0 |= component.bit();
    }

    pub const fn remove(&mut self, component: Component) {
        self.0 &= !component.bit();
    }

    pub const fn set(&mut self, component: Component, enabled: bool) {
        if enabled {
            self.insert(component);
        } else {
            self.remove(component);
        }
    }

    #[must_use]
    pub const fn backend(self) -> bool {
        self.contains(Component::Backend)
    }

    #[must_use]
    pub const fn worker(self) -> bool {
        self.contains(Component::Worker)
    }

    #[must_use]
    pub const fn mobile(self) -> bool {
        self.contains(Component::Mobile)
    }

    #[must_use]
    pub const fn web(self) -> bool {
        self.contains(Component::Web)
    }

    #[must_use]
    pub const fn pwa(self) -> bool {
        self.contains(Component::Pwa)
    }

    #[must_use]
    pub const fn mcp(self) -> bool {
        self.contains(Component::Mcp)
    }

    #[must_use]
    pub const fn platform(self) -> bool {
        self.contains(Component::Platform)
    }
}

impl FromIterator<Component> for Components {
    fn from_iter<I: IntoIterator<Item = Component>>(components: I) -> Self {
        components.into_iter().fold(Self::default(), Self::with)
    }
}

impl<const N: usize> From<[Component; N]> for Components {
    fn from(components: [Component; N]) -> Self {
        components.into_iter().collect()
    }
}

impl Serialize for Components {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(Component::ALL.len()))?;
        for component in Component::ALL {
            map.serialize_entry(component.key(), &self.contains(component))?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Components {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(ComponentsVisitor)
    }
}

struct ComponentsVisitor;

impl<'de> Visitor<'de> for ComponentsVisitor {
    type Value = Components;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a table of component booleans")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Components, A::Error> {
        let mut components = Components::default();
        let mut present = Components::default();
        while let Some(key) = map.next_key::<String>()? {
            let Some(component) = Component::from_key(&key) else {
                map.next_value::<IgnoredAny>()?;
                continue;
            };
            if present.contains(component) {
                return Err(de::Error::duplicate_field(component.key()));
            }
            present.insert(component);
            components.set(component, map.next_value()?);
        }
        if let Some(missing) = Component::REQUIRED
            .into_iter()
            .find(|component| !present.contains(*component))
        {
            return Err(de::Error::missing_field(missing.key()));
        }
        Ok(components)
    }
}

#[cfg(test)]
mod tests {
    use super::{Component, Components};

    #[test]
    fn components_round_trip_as_component_booleans() {
        let components = Components::from([Component::Backend, Component::Mcp]);
        let value = serde_json::to_value(components).unwrap();
        assert_eq!(value["backend"], true);
        assert_eq!(value["mcp"], true);
        assert_eq!(value["web"], false);
        assert_eq!(
            serde_json::from_value::<Components>(value).unwrap(),
            components
        );
    }

    #[test]
    fn required_components_must_be_listed() {
        let error = serde_json::from_str::<Components>(r#"{"backend": true, "web": false}"#)
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing field `mobile`"), "{error}");
    }

    #[test]
    fn optional_components_default_to_disabled_and_unknown_keys_are_ignored() {
        let components = serde_json::from_str::<Components>(
            r#"{"backend": false, "mobile": true, "web": false, "analytics": "none"}"#,
        )
        .unwrap();
        assert_eq!(components, Components::from([Component::Mobile]));
    }
}
