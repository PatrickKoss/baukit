#![cfg(test)]

use std::collections::BTreeMap;

use baukit_credential_vault::{CredentialCipher, CredentialSecrets, EncryptedCredentials};
use proptest::prelude::*;
use uuid::Uuid;

const KEYRING: &str =
    "1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=,2:ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";
const ACTIVE_VERSION: i32 = 2;

fn cipher() -> CredentialCipher {
    CredentialCipher::parse(KEYRING).expect("test keyring")
}

fn field_name() -> impl Strategy<Value = String> {
    "[A-Za-z0-9_.-]{1,64}"
}

fn secrets() -> impl Strategy<Value = BTreeMap<String, Vec<u8>>> {
    prop::collection::btree_map(
        field_name(),
        prop::collection::vec(any::<u8>(), 1..128),
        1..6,
    )
}

fn credential_set(fields: &BTreeMap<String, Vec<u8>>) -> CredentialSecrets {
    let mut set = CredentialSecrets::new();
    for (name, value) in fields {
        set.insert(name.clone(), value.clone())
            .expect("valid field");
    }
    set
}

fn encrypt(fields: &BTreeMap<String, Vec<u8>>, scope: u128) -> EncryptedCredentials {
    cipher()
        .encrypt(Uuid::from_u128(scope), &credential_set(fields))
        .expect("encrypt")
}

proptest! {
    #[test]
    fn decrypt_inverts_encrypt(fields in secrets(), scope in any::<u128>()) {
        let encrypted = encrypt(&fields, scope);
        prop_assert_eq!(encrypted.key_version, ACTIVE_VERSION);
        let decrypted = cipher().decrypt(&encrypted).expect("decrypt");
        prop_assert!(decrypted == credential_set(&fields));
    }

    #[test]
    fn another_scope_fails(fields in secrets(), scope in any::<u128>(), other in any::<u128>()) {
        prop_assume!(scope != other);
        let mut encrypted = encrypt(&fields, scope);
        encrypted.scope_id = Uuid::from_u128(other);
        prop_assert!(cipher().decrypt(&encrypted).is_err());
    }

    #[test]
    fn renamed_field_fails(fields in secrets(), scope in any::<u128>(), index in any::<prop::sample::Index>(), new_name in field_name()) {
        let mut encrypted = encrypt(&fields, scope);
        prop_assume!(!encrypted.fields.contains_key(&new_name));
        let old_name = encrypted.fields.keys().nth(index.index(encrypted.fields.len())).cloned().expect("field");
        let field = encrypted.fields.remove(&old_name).expect("field");
        encrypted.fields.insert(new_name, field);
        prop_assert!(cipher().decrypt(&encrypted).is_err());
    }

    #[test]
    fn another_key_version_fails(fields in secrets(), scope in any::<u128>(), version in any::<i32>()) {
        prop_assume!(version != ACTIVE_VERSION);
        let mut encrypted = encrypt(&fields, scope);
        encrypted.key_version = version;
        prop_assert!(cipher().decrypt(&encrypted).is_err());
    }

    #[test]
    fn single_byte_change_fails(
        fields in secrets(),
        scope in any::<u128>(),
        field in any::<prop::sample::Index>(),
        position in any::<prop::sample::Index>(),
        flip in 1..=u8::MAX,
        in_nonce in any::<bool>(),
    ) {
        let mut encrypted = encrypt(&fields, scope);
        let count = encrypted.fields.len();
        let target = encrypted.fields.values_mut().nth(field.index(count)).expect("field");
        let bytes = if in_nonce { &mut target.nonce } else { &mut target.ciphertext };
        let at = position.index(bytes.len());
        bytes[at] ^= flip;
        prop_assert!(cipher().decrypt(&encrypted).is_err());
    }
}
