use std::collections::BTreeSet;

use serde_json::Value;

use crate::VerificationError;

pub(crate) fn validate_clerk_authorized_party(
    azp: Option<&Value>,
    authorized_parties: &BTreeSet<String>,
    allow_missing_azp: bool,
) -> Result<(), VerificationError> {
    match azp {
        None | Some(Value::Null) if allow_missing_azp => Ok(()),
        None | Some(Value::Null) => Err(VerificationError::WrongAuthorizedParty),
        Some(Value::String(value)) if authorized_parties.contains(value) => Ok(()),
        Some(Value::String(_)) => Err(VerificationError::WrongAuthorizedParty),
        Some(_) => Err(VerificationError::InvalidPrincipalContext),
    }
}

pub(crate) fn validate_client_id(
    client_id: Option<&Value>,
    expected: Option<&str>,
) -> Result<(), VerificationError> {
    match (client_id, expected) {
        (Some(Value::String(value)), Some(expected)) if value == expected => Ok(()),
        (Some(Value::String(value)), None) if !value.is_empty() => Ok(()),
        (Some(Value::String(_)) | None | Some(Value::Null), _) => {
            Err(VerificationError::WrongClientId)
        }
        (Some(_), _) => Err(VerificationError::InvalidPrincipalContext),
    }
}
