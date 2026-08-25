use std::{collections::BTreeSet, fs, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/product-experience")
        .join(name)
}

fn load_fixture<T>(name: &str) -> T
where
    T: serde::de::DeserializeOwned,
{
    let path = fixture_path(name);
    let json = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&json)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn deserialize_present_optional<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OptionalWireFixture {
    name: String,
    key: String,
    wire: Value,
    expected_state: String,
    #[serde(default)]
    expected_value: Option<Value>,
    round_trip: Value,
}

macro_rules! optional_payload {
    ($name:ident, $field:ident, $type:ty) => {
        #[derive(Debug, Deserialize, Serialize)]
        struct $name {
            #[serde(
                default,
                skip_serializing_if = "Option::is_none",
                deserialize_with = "deserialize_present_optional"
            )]
            $field: Option<Option<$type>>,
        }
    };
}

optional_payload!(LanguagePayload, language, String);
optional_payload!(ThemeModePayload, theme_mode, String);
optional_payload!(GameLayerPayload, game_layer_enabled, bool);
optional_payload!(CustomColorPayload, custom_color, String);

fn represented_wire(wire: &Value) -> Value {
    if wire == "absent" {
        Value::Object(serde_json::Map::new())
    } else {
        wire.clone()
    }
}

fn represented_round_trip(key: &str, encoded: Value) -> Value {
    if encoded.get(key).is_some() {
        encoded
    } else {
        Value::String("absent".to_owned())
    }
}

fn state_and_value<T>(field: &Option<Option<T>>) -> (&'static str, Option<Value>)
where
    T: Serialize,
{
    match field {
        None => ("absent", None),
        Some(None) => ("null", None),
        Some(Some(value)) => (
            "value",
            Some(serde_json::to_value(value).expect("fixture scalar should serialize")),
        ),
    }
}

#[test]
fn optional_wire_values_round_trip_through_double_option() {
    let fixtures: Vec<OptionalWireFixture> = load_fixture("optional-wire-values.json");
    assert!(!fixtures.is_empty());

    for fixture in fixtures {
        let input = represented_wire(&fixture.wire);
        let (state, value, encoded) = match fixture.key.as_str() {
            "language" => {
                let payload: LanguagePayload =
                    serde_json::from_value(input).expect("language fixture should deserialize");
                let (state, value) = state_and_value(&payload.language);
                (
                    state,
                    value,
                    serde_json::to_value(payload).expect("language fixture should serialize"),
                )
            }
            "theme_mode" => {
                let payload: ThemeModePayload =
                    serde_json::from_value(input).expect("theme fixture should deserialize");
                let (state, value) = state_and_value(&payload.theme_mode);
                (
                    state,
                    value,
                    serde_json::to_value(payload).expect("theme fixture should serialize"),
                )
            }
            "game_layer_enabled" => {
                let payload: GameLayerPayload =
                    serde_json::from_value(input).expect("game-layer fixture should deserialize");
                let (state, value) = state_and_value(&payload.game_layer_enabled);
                (
                    state,
                    value,
                    serde_json::to_value(payload).expect("game-layer fixture should serialize"),
                )
            }
            "custom_color" => {
                let payload: CustomColorPayload =
                    serde_json::from_value(input).expect("custom-color fixture should deserialize");
                let (state, value) = state_and_value(&payload.custom_color);
                (
                    state,
                    value,
                    serde_json::to_value(payload).expect("custom-color fixture should serialize"),
                )
            }
            key => panic!("unsupported fixture key {key:?}"),
        };

        assert_eq!(state, fixture.expected_state, "{}", fixture.name);
        assert_eq!(value, fixture.expected_value, "{}", fixture.name);
        assert_eq!(
            represented_round_trip(&fixture.key, encoded),
            fixture.round_trip,
            "{}",
            fixture.name
        );
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeletionOutcomeFixture {
    name: String,
    server_retry: ServerRetry,
    session_retained: bool,
    result: DeletionResult,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum ServerRetry {
    NotRequired,
    Retry,
    Reconcile,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
enum DeletionResult {
    Erased {
        receipt: ErasureReceipt,
        warnings: Vec<ErasureIssue>,
    },
    ServerFailure {
        error: ErasureIssue,
        warnings: Vec<ErasureIssue>,
    },
    Ambiguous {
        error: ErasureIssue,
        warnings: Vec<ErasureIssue>,
    },
    LocalFailure {
        receipt: ErasureReceipt,
        error: ErasureIssue,
        sign_out_error: Option<ErasureIssue>,
        warnings: Vec<ErasureIssue>,
    },
    SignoutFailure {
        receipt: ErasureReceipt,
        error: ErasureIssue,
        warnings: Vec<ErasureIssue>,
    },
}

impl DeletionResult {
    fn status(&self) -> &'static str {
        match self {
            Self::Erased { .. } => "erased",
            Self::ServerFailure { .. } => "server-failure",
            Self::Ambiguous { .. } => "ambiguous",
            Self::LocalFailure { .. } => "local-failure",
            Self::SignoutFailure { .. } => "signout-failure",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErasureReceipt {
    operation_id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct ErasureIssue {
    stage: String,
    cause: String,
}

#[test]
fn deletion_outcomes_match_the_shared_contract() {
    let fixtures: Vec<DeletionOutcomeFixture> = load_fixture("deletion-outcomes.json");
    let names = fixtures
        .iter()
        .map(|fixture| fixture.name.as_str())
        .collect::<BTreeSet<_>>();
    let statuses = fixtures
        .iter()
        .map(|fixture| fixture.result.status())
        .collect::<BTreeSet<_>>();
    let outcomes = fixtures
        .iter()
        .map(|fixture| (fixture.name.as_str(), fixture.result.status()))
        .collect::<BTreeSet<_>>();

    assert_eq!(
        names,
        BTreeSet::from([
            "ambiguous",
            "local-failure",
            "server-failure",
            "signout-failure",
            "success",
            "warnings-only",
        ])
    );
    assert_eq!(
        statuses,
        BTreeSet::from([
            "ambiguous",
            "erased",
            "local-failure",
            "server-failure",
            "signout-failure",
        ])
    );
    assert_eq!(
        outcomes,
        BTreeSet::from([
            ("ambiguous", "ambiguous"),
            ("local-failure", "local-failure"),
            ("server-failure", "server-failure"),
            ("signout-failure", "signout-failure"),
            ("success", "erased"),
            ("warnings-only", "erased"),
        ])
    );

    for fixture in fixtures {
        let expected_retry = match &fixture.result {
            DeletionResult::ServerFailure { .. } => ServerRetry::Retry,
            DeletionResult::Ambiguous { .. } => ServerRetry::Reconcile,
            _ => ServerRetry::NotRequired,
        };
        let expected_session_retained = matches!(
            &fixture.result,
            DeletionResult::ServerFailure { .. }
                | DeletionResult::Ambiguous { .. }
                | DeletionResult::SignoutFailure { .. }
        );

        assert_eq!(fixture.server_retry, expected_retry, "{}", fixture.name);
        assert_eq!(
            fixture.session_retained, expected_session_retained,
            "{}",
            fixture.name
        );
        assert_deletion_result_shape(&fixture.result, &fixture.name);
    }
}

fn assert_deletion_result_shape(result: &DeletionResult, name: &str) {
    let assert_receipt = |receipt: &ErasureReceipt| {
        assert!(!receipt.operation_id.is_empty(), "{name}");
        assert!(
            matches!(receipt.status.as_str(), "completed" | "pending"),
            "{name}"
        );
    };
    let assert_issue = |issue: &ErasureIssue| {
        assert!(
            matches!(
                issue.stage.as_str(),
                "before-server" | "server" | "local" | "sign-out"
            ),
            "{name}"
        );
        assert!(!issue.cause.is_empty(), "{name}");
    };

    match result {
        DeletionResult::Erased { receipt, warnings } => {
            assert_receipt(receipt);
            warnings.iter().for_each(assert_issue);
        }
        DeletionResult::ServerFailure { error, warnings }
        | DeletionResult::Ambiguous { error, warnings } => {
            assert_issue(error);
            warnings.iter().for_each(assert_issue);
        }
        DeletionResult::LocalFailure {
            receipt,
            error,
            sign_out_error,
            warnings,
        } => {
            assert_receipt(receipt);
            assert_issue(error);
            if let Some(error) = sign_out_error {
                assert_issue(error);
            }
            warnings.iter().for_each(assert_issue);
        }
        DeletionResult::SignoutFailure {
            receipt,
            error,
            warnings,
        } => {
            assert_receipt(receipt);
            assert_issue(error);
            warnings.iter().for_each(assert_issue);
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocaleResolutionFixture {
    name: String,
    preference: Value,
    device_locales: Vec<String>,
    supported: Vec<String>,
    fallback: String,
    expected: String,
}

#[test]
fn locale_resolution_fixtures_have_the_rust_visible_schema() {
    let fixtures: Vec<LocaleResolutionFixture> = load_fixture("locale-resolution.json");
    assert!(!fixtures.is_empty());

    for fixture in fixtures {
        assert!(!fixture.name.is_empty());
        assert!(!fixture.supported.is_empty(), "{}", fixture.name);
        assert!(!fixture.fallback.is_empty(), "{}", fixture.name);
        assert!(!fixture.expected.is_empty(), "{}", fixture.name);
        assert!(
            fixture
                .device_locales
                .iter()
                .all(|locale| !locale.is_empty()),
            "{}",
            fixture.name
        );
        let _ = fixture.preference;
    }
}
