use chrono::{DateTime, NaiveDate};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

pub const MAX_PAYLOAD_KEYS: usize = 32;

#[derive(Clone, Debug, Error, PartialEq)]
pub enum PayloadError {
    #[error("unsupported event type: {0}")]
    UnsupportedType(String),
    #[error("payload has more than 32 keys")]
    TooManyKeys,
    #[error("invalid payload key: {0}")]
    InvalidKey(String),
    #[error("null payload value: {0}")]
    NullValue(String),
    #[error("unknown payload field: {0}")]
    UnknownField(String),
    #[error("missing payload field: {0}")]
    MissingField(String),
    #[error("invalid type for payload field: {0}")]
    InvalidType(String),
    #[error("payload field is out of bounds: {0}")]
    OutOfBounds(String),
    #[error("invalid payload value: {0}")]
    InvalidValue(String),
    #[error("invalid typed payload: {0}")]
    InvalidPayload(String),
}

impl PayloadError {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::UnsupportedType(_) => "unsupported_type",
            Self::TooManyKeys => "too_many_keys",
            Self::InvalidKey(_) => "invalid_key",
            Self::NullValue(_) => "null_value",
            Self::UnknownField(_) => "unknown_field",
            Self::MissingField(_) => "missing_field",
            Self::InvalidType(_) => "invalid_type",
            Self::OutOfBounds(_) => "out_of_bounds",
            Self::InvalidValue(_) => "invalid_value",
            Self::InvalidPayload(_) => "invalid_payload",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldKind<'a> {
    Integer { minimum: i64, maximum: i64 },
    Number { minimum: f64, maximum: f64 },
    Enum(&'a [&'a str]),
    Identifier { max_length: usize },
    String { max_length: usize },
    Uuid,
    CivilDate,
    Timestamp,
    Boolean,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldSpec<'a> {
    pub name: &'a str,
    pub required: bool,
    pub kind: FieldKind<'a>,
}

pub fn validate_fields(
    payload: &Map<String, Value>,
    fields: &[FieldSpec<'_>],
) -> Result<(), PayloadError> {
    if payload.len() > MAX_PAYLOAD_KEYS {
        return Err(PayloadError::TooManyKeys);
    }
    for (key, value) in payload {
        let mut bytes = key.bytes();
        if key.len() > 64
            || !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            || !bytes.all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(PayloadError::InvalidKey(key.clone()));
        }
        if value.is_null() {
            return Err(PayloadError::NullValue(key.clone()));
        }
        let field = fields
            .iter()
            .find(|field| field.name == key)
            .ok_or_else(|| PayloadError::UnknownField(key.clone()))?;
        validate_value(key, value, field.kind)?;
    }
    for field in fields {
        if field.required && !payload.contains_key(field.name) {
            return Err(PayloadError::MissingField(field.name.to_owned()));
        }
    }
    Ok(())
}

fn validate_value(key: &str, value: &Value, kind: FieldKind<'_>) -> Result<(), PayloadError> {
    let invalid_type = || PayloadError::InvalidType(key.to_owned());
    let invalid_value = || PayloadError::InvalidValue(key.to_owned());
    let out_of_bounds = || PayloadError::OutOfBounds(key.to_owned());
    match kind {
        FieldKind::Integer { minimum, maximum } => {
            let number = if let Some(number) = value.as_i64() {
                number
            } else if value.as_u64().is_some() {
                return Err(out_of_bounds());
            } else {
                return Err(invalid_type());
            };
            if !(minimum..=maximum).contains(&number) {
                return Err(out_of_bounds());
            }
        }
        FieldKind::Number { minimum, maximum } => {
            let number = value.as_f64().ok_or_else(invalid_type)?;
            if !number.is_finite() || !(minimum..=maximum).contains(&number) {
                return Err(out_of_bounds());
            }
        }
        FieldKind::Boolean => {
            if !value.is_boolean() {
                return Err(invalid_type());
            }
        }
        _ => {
            let text = value.as_str().ok_or_else(invalid_type)?;
            let valid = match kind {
                FieldKind::Enum(values) => values.contains(&text),
                FieldKind::Identifier { max_length } => valid_identifier(text, max_length),
                FieldKind::String { max_length } => {
                    !text.is_empty()
                        && text.chars().count() <= max_length
                        && !text.chars().any(char::is_control)
                }
                FieldKind::Uuid => parse_uuid(text).is_some(),
                FieldKind::CivilDate => parse_civil_date(text).is_some(),
                FieldKind::Timestamp => DateTime::parse_from_rfc3339(text).is_ok(),
                _ => false,
            };
            if !valid {
                return Err(invalid_value());
            }
        }
    }
    Ok(())
}

fn valid_identifier(value: &str, max_length: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-.:".contains(&byte))
}

fn parse_civil_date(value: &str) -> Option<NaiveDate> {
    if value.len() != 10 {
        return None;
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    (date.format("%Y-%m-%d").to_string() == value).then_some(date)
}

pub fn integer<'de, D: Deserializer<'de>, T: TryFrom<i64>, const MIN: i64, const MAX: i64>(
    deserializer: D,
) -> Result<T, D::Error> {
    let value = i64::deserialize(deserializer)?;
    if !(MIN..=MAX).contains(&value) {
        return Err(serde::de::Error::custom("integer out of bounds"));
    }
    T::try_from(value).map_err(|_| serde::de::Error::custom("integer out of range"))
}

fn parse_uuid(value: &str) -> Option<Uuid> {
    let uuid = Uuid::parse_str(value).ok()?;
    (uuid.hyphenated().to_string() == value).then_some(uuid)
}
