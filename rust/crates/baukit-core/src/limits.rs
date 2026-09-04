//! Runtime-neutral resource-budget measurements.

use serde::Serialize;
use thiserror::Error;

/// A measured amount and the maximum amount allowed by a caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitMeasurement {
    measured: usize,
    allowed: usize,
}

impl LimitMeasurement {
    /// Returns the measured amount.
    #[must_use]
    pub const fn measured(self) -> usize {
        self.measured
    }

    /// Returns the caller-provided maximum.
    #[must_use]
    pub const fn allowed(self) -> usize {
        self.allowed
    }
}

/// A resource-budget check whose measured amount exceeds its allowed amount.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("measured {measured} exceeds allowed {allowed}")]
pub struct LimitExceeded {
    measured: usize,
    allowed: usize,
}

impl LimitExceeded {
    /// Returns the measured amount.
    #[must_use]
    pub const fn measured(self) -> usize {
        self.measured
    }

    /// Returns the caller-provided maximum.
    #[must_use]
    pub const fn allowed(self) -> usize {
        self.allowed
    }
}

/// An error from encoding and checking compact JSON.
#[derive(Debug, Error)]
pub enum CompactJsonLimitError {
    /// Compact JSON encoding failed.
    #[error("compact JSON encoding failed")]
    Encoding(#[from] serde_json::Error),
    /// The encoded JSON exceeds the caller-provided maximum.
    #[error(transparent)]
    Limit(#[from] LimitExceeded),
}

/// Counts Unicode scalar values after trimming Unicode whitespace from both ends.
#[must_use]
pub fn trimmed_unicode_scalar_count(value: &str) -> usize {
    value.trim().chars().count()
}

/// Returns the UTF-8 byte length of the compact JSON encoding.
pub fn compact_json_utf8_bytes(value: &impl Serialize) -> Result<usize, serde_json::Error> {
    serde_json::to_vec(value).map(|encoded| encoded.len())
}

/// Returns the length of a byte slice.
#[must_use]
pub const fn byte_length(value: &[u8]) -> usize {
    value.len()
}

/// Returns the number of elements in a slice.
#[must_use]
pub const fn collection_length<Element>(value: &[Element]) -> usize {
    value.len()
}

/// Checks a measured amount against a caller-provided maximum.
pub const fn check_measurement(
    measured: usize,
    allowed: usize,
) -> Result<LimitMeasurement, LimitExceeded> {
    if measured > allowed {
        return Err(LimitExceeded { measured, allowed });
    }
    Ok(LimitMeasurement { measured, allowed })
}

/// Measures trimmed Unicode scalar values and checks the result.
pub fn check_trimmed_unicode_scalars(
    value: &str,
    allowed: usize,
) -> Result<LimitMeasurement, LimitExceeded> {
    check_measurement(trimmed_unicode_scalar_count(value), allowed)
}

/// Measures compact JSON UTF-8 bytes and checks the result.
pub fn check_compact_json_utf8_bytes(
    value: &impl Serialize,
    allowed: usize,
) -> Result<LimitMeasurement, CompactJsonLimitError> {
    let measured = compact_json_utf8_bytes(value)?;
    check_measurement(measured, allowed).map_err(Into::into)
}

/// Measures a byte slice and checks the result.
pub const fn check_bytes(value: &[u8], allowed: usize) -> Result<LimitMeasurement, LimitExceeded> {
    check_measurement(byte_length(value), allowed)
}

/// Measures a collection slice and checks the result.
pub const fn check_collection<Element>(
    value: &[Element],
    allowed: usize,
) -> Result<LimitMeasurement, LimitExceeded> {
    check_measurement(collection_length(value), allowed)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::{Value, json};

    use super::*;

    #[derive(Debug, Deserialize)]
    struct FixtureCorpus {
        version: u8,
        text: Vec<TextFixture>,
        json: Vec<JsonFixture>,
        bytes: Vec<BytesFixture>,
        collections: Vec<CollectionFixture>,
    }

    #[derive(Debug, Deserialize)]
    struct TextFixture {
        name: String,
        value: String,
        trimmed_unicode_scalars: usize,
    }

    #[derive(Debug, Deserialize)]
    struct JsonFixture {
        name: String,
        value: Value,
        compact: String,
        utf8_bytes: usize,
    }

    #[derive(Debug, Deserialize)]
    struct BytesFixture {
        name: String,
        value: Vec<u8>,
        bytes: usize,
    }

    #[derive(Debug, Deserialize)]
    struct CollectionFixture {
        name: String,
        value: Vec<Value>,
        elements: usize,
    }

    fn fixtures() -> FixtureCorpus {
        serde_json::from_str(include_str!(
            "../../../../fixtures/limits/resource-budget-measurements-v1.json"
        ))
        .expect("resource-budget fixture should parse")
    }

    #[test]
    fn counts_ascii_composed_decomposed_and_emoji_scalars() {
        assert_eq!(trimmed_unicode_scalar_count(" plain "), 5);
        assert_eq!(trimmed_unicode_scalar_count("\u{e9}"), 1);
        assert_eq!(trimmed_unicode_scalar_count("e\u{301}"), 2);
        assert_eq!(
            trimmed_unicode_scalar_count(
                "\u{1f469}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}"
            ),
            7
        );
    }

    #[test]
    fn measures_compact_objects_arrays_escapes_and_multibyte_strings() {
        for fixture in fixtures().json {
            let encoded = serde_json::to_string(&fixture.value).expect("fixture should encode");
            assert_eq!(encoded, fixture.compact, "{}", fixture.name);
            assert_eq!(
                compact_json_utf8_bytes(&fixture.value).expect("fixture should encode"),
                fixture.utf8_bytes,
                "{}",
                fixture.name
            );
        }
    }

    #[test]
    fn byte_and_collection_checks_report_both_sides_of_the_boundary() {
        let bytes = [1_u8, 2, 3];
        let values = ["a", "b", "c"];

        assert_eq!(
            check_bytes(&bytes, 3)
                .expect("boundary should pass")
                .measured(),
            3
        );
        assert_eq!(
            check_collection(&values, 3)
                .expect("boundary should pass")
                .allowed(),
            3
        );
        assert_eq!(
            check_bytes(&bytes, 2),
            Err(LimitExceeded {
                measured: 3,
                allowed: 2,
            })
        );
        assert_eq!(
            check_collection(&values, 2),
            Err(LimitExceeded {
                measured: 3,
                allowed: 2,
            })
        );
    }

    #[test]
    fn specialized_checks_return_measurement_and_allowed_values() {
        assert_eq!(
            check_trimmed_unicode_scalars("  e\u{301}  ", 2),
            Ok(LimitMeasurement {
                measured: 2,
                allowed: 2,
            })
        );
        let error = check_compact_json_utf8_bytes(&json!({"value": "\u{e9}"}), 13)
            .expect_err("fourteen-byte document should fail");
        assert!(matches!(
            error,
            CompactJsonLimitError::Limit(LimitExceeded {
                measured: 14,
                allowed: 13,
            })
        ));
    }

    #[test]
    fn shared_fixture_matches_all_rust_measurements() {
        let fixtures = fixtures();
        assert_eq!(fixtures.version, 1);
        assert!(!fixtures.text.is_empty());
        assert!(!fixtures.json.is_empty());
        assert!(!fixtures.bytes.is_empty());
        assert!(!fixtures.collections.is_empty());

        for fixture in fixtures.text {
            assert_eq!(
                trimmed_unicode_scalar_count(&fixture.value),
                fixture.trimmed_unicode_scalars,
                "{}",
                fixture.name
            );
        }
        for fixture in fixtures.bytes {
            assert_eq!(
                byte_length(&fixture.value),
                fixture.bytes,
                "{}",
                fixture.name
            );
        }
        for fixture in fixtures.collections {
            assert_eq!(
                collection_length(&fixture.value),
                fixture.elements,
                "{}",
                fixture.name
            );
        }
    }
}
