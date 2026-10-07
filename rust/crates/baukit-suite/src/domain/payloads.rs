use std::collections::BTreeSet;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

pub use super::validation::PayloadError;
use super::{
    SUITE_CONNECTION_TEST_TYPE, SuiteDataError, SuiteEvent,
    validation::{self, FieldKind, FieldSpec},
};

const JSON_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// A product's versioned payload contract, embedded in its binary.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PayloadCatalog {
    schema_version: u32,
    #[serde(rename = "types")]
    events: Vec<EventDefinition>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventDefinition {
    #[serde(rename = "type")]
    event_type: String,
    natural_key: String,
    fields: Vec<FieldDefinition>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldDefinition {
    name: String,
    required: bool,
    #[serde(flatten)]
    kind: CatalogFieldKind,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum CatalogFieldKind {
    Integer {
        minimum: i64,
        maximum: i64,
    },
    Number {
        minimum: f64,
        maximum: f64,
    },
    Enum {
        values: Vec<String>,
    },
    Identifier {
        #[serde(rename = "maxLength")]
        max_length: usize,
    },
    String {
        #[serde(rename = "maxLength")]
        max_length: usize,
    },
    Uuid,
    CivilDate,
    Timestamp,
    Boolean,
}

/// A map that has passed the catalog's key, type and value checks.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedPayload(Map<String, Value>);

impl ValidatedPayload {
    /// Reads the checked map without changing it.
    pub fn as_map(&self) -> &Map<String, Value> {
        &self.0
    }

    /// Deserializes into a product-owned payload type. Use `deny_unknown_fields`.
    pub fn deserialize<T: DeserializeOwned>(&self) -> Result<T, PayloadError> {
        serde_json::from_value(Value::Object(self.0.clone()))
            .map_err(|error| PayloadError::InvalidPayload(error.to_string()))
    }
}

impl PayloadCatalog {
    /// Loads a catalog and rejects duplicate event types, fields and invalid bounds.
    pub fn from_json(json: &str) -> Result<Self, SuiteDataError> {
        let catalog: Self = serde_json::from_str(json)
            .map_err(|error| SuiteDataError::InvalidRegistry(error.to_string()))?;
        let invalid = || SuiteDataError::InvalidRegistry("invalid payload catalog".into());
        if catalog.schema_version != 1 {
            return Err(invalid());
        }
        let mut event_types = BTreeSet::new();
        for event in &catalog.events {
            if !event_types.insert(&event.event_type)
                || event.fields.len() > validation::MAX_PAYLOAD_KEYS
            {
                return Err(invalid());
            }
            let mut names = BTreeSet::new();
            for field in &event.fields {
                if !names.insert(&field.name) || !valid_key(&field.name) || !field.kind.valid() {
                    return Err(invalid());
                }
            }
        }
        Ok(catalog)
    }

    /// Checks a payload against its event definition. Optional fields must be omitted.
    pub fn validate(
        &self,
        event_type: &str,
        payload: Map<String, Value>,
    ) -> Result<ValidatedPayload, PayloadError> {
        if event_type == SUITE_CONNECTION_TEST_TYPE {
            validation::validate_fields(&payload, &[])?;
            return Ok(ValidatedPayload(payload));
        }
        let event = self
            .events
            .iter()
            .find(|event| event.event_type == event_type)
            .ok_or_else(|| PayloadError::UnsupportedType(event_type.into()))?;
        let enums: Vec<Vec<&str>> = event
            .fields
            .iter()
            .map(|field| match &field.kind {
                CatalogFieldKind::Enum { values } => values.iter().map(String::as_str).collect(),
                _ => Vec::new(),
            })
            .collect();
        let fields: Vec<_> = event
            .fields
            .iter()
            .zip(&enums)
            .map(|(field, values)| FieldSpec {
                name: &field.name,
                required: field.required,
                kind: field.kind.as_kind(values),
            })
            .collect();
        validation::validate_fields(&payload, &fields)?;
        Ok(ValidatedPayload(payload))
    }
}

impl CatalogFieldKind {
    fn valid(&self) -> bool {
        match self {
            Self::Integer { minimum, maximum } => {
                minimum <= maximum
                    && *minimum >= -JSON_SAFE_INTEGER
                    && *maximum <= JSON_SAFE_INTEGER
            }
            Self::Number { minimum, maximum } => {
                minimum.is_finite() && maximum.is_finite() && minimum <= maximum
            }
            Self::Enum { values } => {
                !values.is_empty() && values.iter().collect::<BTreeSet<_>>().len() == values.len()
            }
            Self::Identifier { max_length } | Self::String { max_length } => *max_length > 0,
            _ => true,
        }
    }
    fn as_kind<'a>(&self, values: &'a [&'a str]) -> FieldKind<'a> {
        match self {
            Self::Integer { minimum, maximum } => FieldKind::Integer {
                minimum: *minimum,
                maximum: *maximum,
            },
            Self::Number { minimum, maximum } => FieldKind::Number {
                minimum: *minimum,
                maximum: *maximum,
            },
            Self::Enum { .. } => FieldKind::Enum(values),
            Self::Identifier { max_length } => FieldKind::Identifier {
                max_length: *max_length,
            },
            Self::String { max_length } => FieldKind::String {
                max_length: *max_length,
            },
            Self::Uuid => FieldKind::Uuid,
            Self::CivilDate => FieldKind::CivilDate,
            Self::Timestamp => FieldKind::Timestamp,
            Self::Boolean => FieldKind::Boolean,
        }
    }
}

pub(super) fn valid_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    key.len() <= 64
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_alphanumeric())
}

impl SuiteEvent {
    /// Builds a protocol test. Its payload is always empty and cannot grant a reward.
    pub fn connection_test(
        test_id: uuid::Uuid,
        occurred_at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            natural_key: test_id.to_string(),
            event_type: SUITE_CONNECTION_TEST_TYPE.into(),
            occurred_at,
            payload: Map::new(),
        }
    }
}
