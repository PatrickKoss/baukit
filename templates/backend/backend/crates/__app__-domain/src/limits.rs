use std::sync::OnceLock;

use baukit_core::limits::{
    CompactJsonLimitError, LimitExceeded, check_compact_json_utf8_bytes, check_measurement,
    check_trimmed_unicode_scalars,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const SUPPORTED_POLICY_VERSION: u32 = 1;
const LIMITS_JSON: &str = include_str!("../../../../limits.json");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LimitsPolicy {
    #[serde(rename = "$comment")]
    _comment: String,
    pub version: u32,
    pub text: TextLimits,
    pub collection: CollectionLimits,
    pub document: DocumentLimits,
    pub rows: RowLimits,
    pub body: BodyLimits,
    pub batch: BatchLimits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextLimits {
    pub max_characters: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CollectionLimits {
    pub max_elements: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DocumentLimits {
    pub max_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RowLimits {
    pub max_count: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BodyLimits {
    pub max_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BatchLimits {
    pub max_items: usize,
}

#[derive(Debug, Error)]
pub enum LimitsPolicyError {
    #[error("limits policy is invalid: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("unsupported limits policy version {0}")]
    UnsupportedVersion(u32),
    #[error("limits policy value `{0}` must be greater than zero")]
    ZeroValue(&'static str),
}

pub fn parse_limits_policy(source: &str) -> Result<LimitsPolicy, LimitsPolicyError> {
    let policy: LimitsPolicy = serde_json::from_str(source)?;
    policy.validate()?;
    Ok(policy)
}

impl LimitsPolicy {
    fn validate(&self) -> Result<(), LimitsPolicyError> {
        if self.version != SUPPORTED_POLICY_VERSION {
            return Err(LimitsPolicyError::UnsupportedVersion(self.version));
        }
        for (name, value) in [
            ("text.max_characters", self.text.max_characters),
            ("collection.max_elements", self.collection.max_elements),
            ("document.max_bytes", self.document.max_bytes),
            ("rows.max_count", self.rows.max_count),
            ("body.max_bytes", self.body.max_bytes),
            ("batch.max_items", self.batch.max_items),
        ] {
            if value == 0 {
                return Err(LimitsPolicyError::ZeroValue(name));
            }
        }
        Ok(())
    }
}

pub fn limits_policy() -> &'static LimitsPolicy {
    static POLICY: OnceLock<LimitsPolicy> = OnceLock::new();
    POLICY.get_or_init(|| {
        parse_limits_policy(LIMITS_JSON)
            .unwrap_or_else(|error| panic!("embedded limits.json must be valid: {error}"))
    })
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitReason {
    TextTooLong,
    JsonbTooLarge,
    TooManyElements,
    TooManyRows,
    BodyTooLarge,
    BatchTooLarge,
}

impl LimitReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TextTooLong => "text_too_long",
            Self::JsonbTooLarge => "jsonb_too_large",
            Self::TooManyElements => "too_many_elements",
            Self::TooManyRows => "too_many_rows",
            Self::BodyTooLarge => "body_too_large",
            Self::BatchTooLarge => "batch_too_large",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("limit exceeded for {field}: {reason:?}")]
pub struct LimitError {
    pub reason: LimitReason,
    pub field: &'static str,
}

impl LimitError {
    pub const fn code(&self) -> &'static str {
        self.reason.as_str()
    }
}

pub fn check_text(field: &'static str, value: &str) -> Result<(), LimitError> {
    map_limit_exceeded(
        field,
        LimitReason::TextTooLong,
        check_trimmed_unicode_scalars(value, limits_policy().text.max_characters),
    )
}

pub fn check_json_document(field: &'static str, value: &Value) -> Result<(), LimitError> {
    match check_compact_json_utf8_bytes(value, limits_policy().document.max_bytes) {
        Ok(_) => Ok(()),
        Err(CompactJsonLimitError::Limit(_)) => Err(LimitError {
            reason: LimitReason::JsonbTooLarge,
            field,
        }),
        Err(CompactJsonLimitError::Encoding(_)) => {
            unreachable!("serde_json::Value must serialize")
        }
    }
}

pub fn check_collection(field: &'static str, count: usize) -> Result<(), LimitError> {
    check_count(
        field,
        count,
        limits_policy().collection.max_elements,
        LimitReason::TooManyElements,
    )
}

pub fn check_rows(field: &'static str, count: usize) -> Result<(), LimitError> {
    check_count(
        field,
        count,
        limits_policy().rows.max_count,
        LimitReason::TooManyRows,
    )
}

pub fn check_body(field: &'static str, byte_length: usize) -> Result<(), LimitError> {
    check_count(
        field,
        byte_length,
        limits_policy().body.max_bytes,
        LimitReason::BodyTooLarge,
    )
}

pub fn check_batch(field: &'static str, count: usize) -> Result<(), LimitError> {
    check_count(
        field,
        count,
        limits_policy().batch.max_items,
        LimitReason::BatchTooLarge,
    )
}

fn check_count(
    field: &'static str,
    actual: usize,
    maximum: usize,
    reason: LimitReason,
) -> Result<(), LimitError> {
    map_limit_exceeded(field, reason, check_measurement(actual, maximum))
}

fn map_limit_exceeded(
    field: &'static str,
    reason: LimitReason,
    result: Result<baukit_core::limits::LimitMeasurement, LimitExceeded>,
) -> Result<(), LimitError> {
    result.map(|_| ()).map_err(|_| LimitError { reason, field })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn loads_the_shared_product_fixture() {
        let parsed = parse_limits_policy(include_str!("../../../../limits.json"))
            .expect("shared limits fixture should parse");
        assert_eq!(limits_policy(), &parsed);
        assert_eq!(parsed.version, SUPPORTED_POLICY_VERSION);
    }

    #[test]
    fn rejects_an_unknown_version_and_zero_values() {
        let unknown_version = LIMITS_JSON.replace("\"version\": 1", "\"version\": 2");
        assert!(matches!(
            parse_limits_policy(&unknown_version),
            Err(LimitsPolicyError::UnsupportedVersion(2))
        ));

        let zero_text = LIMITS_JSON.replace("\"max_characters\": 200", "\"max_characters\": 0");
        assert!(matches!(
            parse_limits_policy(&zero_text),
            Err(LimitsPolicyError::ZeroValue("text.max_characters"))
        ));
    }

    #[test]
    fn rejects_unknown_policy_fields() {
        let unknown_field = LIMITS_JSON.replace("\"version\": 1", "\"version\": 1, \"extra\": 1");
        assert!(matches!(
            parse_limits_policy(&unknown_field),
            Err(LimitsPolicyError::InvalidJson(_))
        ));
    }

    #[test]
    fn checks_boundaries_and_stable_reason_codes() {
        let policy = limits_policy();
        assert_eq!(
            check_text("title", &"é".repeat(policy.text.max_characters)),
            Ok(())
        );
        assert_eq!(
            check_text("title", &"é".repeat(policy.text.max_characters + 1)),
            Err(LimitError {
                reason: LimitReason::TextTooLong,
                field: "title",
            })
        );
        assert_eq!(
            check_collection("entries", policy.collection.max_elements + 1)
                .expect_err("oversized collection should fail")
                .reason,
            LimitReason::TooManyElements
        );
        assert_eq!(
            check_rows("records", policy.rows.max_count + 1)
                .expect_err("too many rows should fail")
                .reason,
            LimitReason::TooManyRows
        );
        assert_eq!(
            check_body("request", policy.body.max_bytes + 1)
                .expect_err("oversized body should fail")
                .reason,
            LimitReason::BodyTooLarge
        );
        assert_eq!(
            check_batch("changes", policy.batch.max_items + 1)
                .expect_err("oversized batch should fail")
                .reason,
            LimitReason::BatchTooLarge
        );

        let reasons = [
            LimitReason::TextTooLong,
            LimitReason::JsonbTooLarge,
            LimitReason::TooManyElements,
            LimitReason::TooManyRows,
            LimitReason::BodyTooLarge,
            LimitReason::BatchTooLarge,
        ];
        assert_eq!(
            reasons.map(LimitReason::as_str),
            [
                "text_too_long",
                "jsonb_too_large",
                "too_many_elements",
                "too_many_rows",
                "body_too_large",
                "batch_too_large"
            ]
        );
        assert_eq!(
            serde_json::to_value(reasons).expect("reasons should serialize"),
            json!([
                "text_too_long",
                "jsonb_too_large",
                "too_many_elements",
                "too_many_rows",
                "body_too_large",
                "batch_too_large"
            ])
        );
    }

    #[test]
    fn measures_compact_json_in_utf8_bytes() {
        let value = json!({ "value": "é" });
        let byte_length = value.to_string().len();
        let oversized = json!({ "value": "x".repeat(limits_policy().document.max_bytes) });

        assert!(byte_length > value.to_string().chars().count());
        assert_eq!(
            check_json_document("metadata", &oversized)
                .expect_err("oversized JSON should fail")
                .reason,
            LimitReason::JsonbTooLarge
        );
    }
}
