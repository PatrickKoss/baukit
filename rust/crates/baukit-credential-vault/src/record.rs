//! All-or-nothing sealing of one credential record (format v2).

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::{
    cipher::{CredentialCipher, open_field},
    error::CredentialVaultError,
    model::{CredentialSecrets, EncryptedField},
};

const RECORD_AAD_DOMAIN: &[u8] = b"baukit-credential-vault/record/v2\0";
const RECORD_ID_LENGTH: usize = 16;

/// A credential record sealed as one unit.
///
/// Every field's associated data binds the scope id, the key version, a random
/// per-seal record id and the full sorted list of field names. Dropping a
/// field, adding one, renaming one, or pasting a field from an earlier seal of
/// the same scope (a different record id) fails as
/// [`CredentialVaultError::DecryptionFailed`]. Replacing the whole record with
/// an older one, record id included, is a rollback the storage layer must stop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SealedCredentials {
    /// Identifier of the credential scope the ciphertext is bound to.
    pub scope_id: Uuid,
    /// Random id of this seal. Store it with the fields.
    pub record_id: Uuid,
    /// Encrypted fields keyed by their plaintext field name.
    pub fields: BTreeMap<String, EncryptedField>,
    /// Keyring version that encrypted these fields and must decrypt them.
    pub key_version: i32,
}

impl CredentialCipher {
    /// Seals every secret as one record under the active key version.
    ///
    /// Prefer this over [`CredentialCipher::encrypt`], whose fields can be
    /// opened, dropped or replaced one at a time.
    ///
    /// # Errors
    ///
    /// [`CredentialVaultError::InvalidConfiguration`] under the same conditions as
    /// [`CredentialCipher::encrypt`].
    pub fn seal_record(
        &self,
        scope_id: Uuid,
        secrets: &CredentialSecrets,
    ) -> Result<SealedCredentials, CredentialVaultError> {
        if secrets.is_empty() {
            return Err(CredentialVaultError::InvalidConfiguration);
        }
        let key = self.active_key()?;
        let record_id = Uuid::from_bytes(self.random_bytes::<RECORD_ID_LENGTH>()?);
        let names: Vec<&str> = secrets.field_names().collect();
        let mut fields = BTreeMap::new();
        for (name, plaintext) in secrets.iter() {
            let aad = record_aad(scope_id, self.active_version(), record_id, &names, name);
            fields.insert(name.to_owned(), self.seal_field(key, &aad, plaintext)?);
        }
        Ok(SealedCredentials {
            scope_id,
            record_id,
            fields,
            key_version: self.active_version(),
        })
    }

    /// Opens a record sealed by [`CredentialCipher::seal_record`], all fields or none.
    ///
    /// # Errors
    ///
    /// [`CredentialVaultError::DecryptionFailed`] when the record has no fields,
    /// names an unknown key version, misses or adds a field, or any field fails
    /// to authenticate under its scope, version, record id and field list.
    pub fn open_record(
        &self,
        sealed: &SealedCredentials,
    ) -> Result<CredentialSecrets, CredentialVaultError> {
        if sealed.fields.is_empty() {
            return Err(CredentialVaultError::DecryptionFailed);
        }
        let key = self
            .key(sealed.key_version)
            .ok_or(CredentialVaultError::DecryptionFailed)?;
        let names: Vec<&str> = sealed.fields.keys().map(String::as_str).collect();
        let mut secrets = CredentialSecrets::new();
        for (name, field) in &sealed.fields {
            let aad = record_aad(
                sealed.scope_id,
                sealed.key_version,
                sealed.record_id,
                &names,
                name,
            );
            let plaintext = open_field(key, &aad, field)?;
            secrets
                .insert(name.clone(), plaintext)
                .map_err(|_| CredentialVaultError::DecryptionFailed)?;
        }
        Ok(secrets)
    }
}

/// Field names are at most 64 bytes, so a one-byte length prefix is unambiguous.
fn record_aad(
    scope_id: Uuid,
    version: i32,
    record_id: Uuid,
    names: &[&str],
    name: &str,
) -> Vec<u8> {
    let mut value = Vec::with_capacity(RECORD_AAD_DOMAIN.len() + 64 + names.len() * 16);
    value.extend_from_slice(RECORD_AAD_DOMAIN);
    value.extend_from_slice(scope_id.as_bytes());
    value.extend_from_slice(&version.to_be_bytes());
    value.extend_from_slice(record_id.as_bytes());
    value.extend_from_slice(&u32::try_from(names.len()).unwrap_or(u32::MAX).to_be_bytes());
    for listed in names.iter().chain([&name]) {
        value.push(u8::try_from(listed.len()).unwrap_or(u8::MAX));
        value.extend_from_slice(listed.as_bytes());
    }
    value
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use proptest::prelude::*;

    use super::*;

    const TAG_LENGTH: usize = 16;

    fn cipher() -> CredentialCipher {
        CredentialCipher::parse(&format!("3:{}", STANDARD.encode([4_u8; 32]))).expect("cipher")
    }

    fn secrets(values: &[(String, Vec<u8>)]) -> CredentialSecrets {
        values
            .iter()
            .fold(CredentialSecrets::new(), |set, (name, value)| {
                set.with(name.clone(), value.clone()).expect("valid field")
            })
    }

    fn field_set() -> impl Strategy<Value = Vec<(String, Vec<u8>)>> {
        prop::collection::btree_map(
            "[a-z_]{1,12}",
            prop::collection::vec(any::<u8>(), 1..48),
            2..5,
        )
        .prop_map(|map| map.into_iter().collect())
    }

    fn failed(result: &Result<CredentialSecrets, CredentialVaultError>) -> bool {
        matches!(result, Err(CredentialVaultError::DecryptionFailed))
    }

    #[test]
    fn round_trip_restores_all_fields() {
        let cipher = cipher();
        let input = secrets(&[
            ("access_token".into(), b"a".to_vec()),
            ("refresh_token".into(), b"r".to_vec()),
        ]);
        let sealed = cipher.seal_record(Uuid::now_v7(), &input).expect("seal");
        assert!(cipher.open_record(&sealed).expect("open") == input);
    }

    proptest! {
        #[test]
        fn dropping_any_field_fails(values in field_set(), index in any::<prop::sample::Index>()) {
            let cipher = cipher();
            let mut sealed = cipher.seal_record(Uuid::now_v7(), &secrets(&values)).expect("seal");
            let name = values[index.index(values.len())].0.clone();
            sealed.fields.remove(&name);
            prop_assert!(failed(&cipher.open_record(&sealed)));
        }

        #[test]
        fn substituting_a_field_from_an_earlier_seal_fails(
            values in field_set(),
            index in any::<prop::sample::Index>(),
        ) {
            let cipher = cipher();
            let scope = Uuid::now_v7();
            let old = cipher.seal_record(scope, &secrets(&values)).expect("seal");
            let mut current = cipher.seal_record(scope, &secrets(&values)).expect("seal");
            let name = values[index.index(values.len())].0.clone();
            current.fields.insert(name.clone(), old.fields[&name].clone());
            prop_assert!(failed(&cipher.open_record(&current)));
        }

        #[test]
        fn truncating_below_the_tag_fails(
            values in field_set(),
            index in any::<prop::sample::Index>(),
            keep in 0..TAG_LENGTH,
        ) {
            let cipher = cipher();
            let mut sealed = cipher.seal_record(Uuid::now_v7(), &secrets(&values)).expect("seal");
            let name = values[index.index(values.len())].0.clone();
            let field = sealed.fields.get_mut(&name).expect("field");
            field.ciphertext.truncate(keep);
            prop_assert!(failed(&cipher.open_record(&sealed)));
        }

        #[test]
        fn legacy_decrypt_opens_a_subset_as_documented(
            values in field_set(),
            index in any::<prop::sample::Index>(),
        ) {
            let cipher = cipher();
            let mut encrypted = cipher.encrypt(Uuid::now_v7(), &secrets(&values)).expect("encrypt");
            let name = values[index.index(values.len())].0.clone();
            encrypted.fields.remove(&name);
            let opened = cipher.decrypt(&encrypted).expect("legacy subset opens");
            prop_assert_eq!(opened.len(), values.len() - 1);
            prop_assert!(opened.get(&name).is_none());
        }
    }
}
