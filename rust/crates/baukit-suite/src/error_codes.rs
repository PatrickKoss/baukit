macro_rules! error_codes {
    ($($variant:ident => $code:literal),+ $(,)?) => {
        /// Error codes emitted by the suite protocol and its default HTTP adapters.
        pub const SUITE_ERROR_CODES: &[&str] = &[$(SuiteErrorCode::$variant.as_str()),+];

        #[derive(Clone, Copy, Debug, serde::Serialize)]
        #[cfg_attr(feature = "http", derive(utoipa::ToSchema))]
        #[serde(rename_all = "snake_case")]
        pub(crate) enum SuiteErrorCode {
            $(#[serde(rename = $code)] $variant),+
        }
        impl SuiteErrorCode {
            #[cfg(feature = "http")]
            pub(crate) fn parse(code: &str) -> Option<Self> {
                match code { $($code => Some(Self::$variant)),+, _ => None }
            }
            pub(crate) const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $code),+ }
            }
        }
    };
}

error_codes! {
   BadRequest => "bad_request",
   EventIdInvalid => "event_id_invalid",
   EventSchemaUnsupported => "event_schema_unsupported",
   EventTooOld => "event_too_old",
   EventTypeInvalid => "event_type_invalid",
   EventUserMismatch => "event_user_mismatch",
   Forbidden => "forbidden",
   Internal => "internal",
   InternalError => "internal_error",
   InternalServerError => "internal_server_error",
   InvalidJson => "invalid_json",
   NotFound => "not_found",
   PayloadTooLarge => "payload_too_large",
   RateLimited => "rate_limited",
   RequestTimeout => "request_timeout",
   SuiteCodeInvalid => "suite_code_invalid",
   SuiteDisabled => "suite_disabled",
   SuiteEventIdConflict => "suite_event_id_conflict",
   SuiteEventTypeUnsupported => "suite_event_type_unsupported",
   SuiteJobTypeUnsupported => "suite_job_type_unsupported",
   SuiteLinkAccountMismatch => "suite_link_account_mismatch",
   SuiteLinkDisabled => "suite_link_disabled",
   SuiteLinkInactive => "suite_link_inactive",
   SuiteLinkRevoked => "suite_link_revoked",
   SuitePayloadInvalid => "suite_payload_invalid",
   SuitePeerUnknown => "suite_peer_unknown",
   SuitePeerUnreachable => "suite_peer_unreachable",
   SuiteQuotaExceeded => "suite_quota_exceeded",
   SuiteRejected => "suite_rejected",
   SuiteReplayThrottled => "suite_replay_throttled",
   SuiteReplayWindow => "suite_replay_window",
   SuiteReturnUrlInvalid => "suite_return_url_invalid",
   SuiteSignatureInvalid => "suite_signature_invalid",
   SuiteUnauthorized => "suite_unauthorized",
   SuiteUnavailable => "suite_unavailable",
   Unauthenticated => "unauthenticated",
   UnsupportedMediaType => "unsupported_media_type",
   ValidationFailed => "validation_failed",
}

impl From<baukit_events::EventValidationCode> for SuiteErrorCode {
    fn from(code: baukit_events::EventValidationCode) -> Self {
        use baukit_events::EventValidationCode as E;
        match code {
            E::EventIdInvalid => Self::EventIdInvalid,
            E::EventTypeInvalid => Self::EventTypeInvalid,
            E::EventUserMismatch => Self::EventUserMismatch,
            E::EventTooOld => Self::EventTooOld,
            E::EventSchemaUnsupported => Self::EventSchemaUnsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_match_shared_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(
            baukit_test::suite::fixture("error-codes.json").expect("packaged error codes"),
        )
        .expect("valid error codes");
        assert_eq!(fixture["schemaVersion"], 1);
        assert_eq!(fixture["codes"], serde_json::json!(SUITE_ERROR_CODES));
        assert!(SUITE_ERROR_CODES.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
