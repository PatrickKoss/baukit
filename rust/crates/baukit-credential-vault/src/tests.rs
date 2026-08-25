use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_config::{Secret, Validate as _};
use uuid::Uuid;
use zeroize::Zeroize as _;

use crate::{
    CredentialCipher, CredentialSecrets, CredentialVaultConfig, CredentialVaultError, KEY_LENGTH,
    NONCE_LENGTH,
};

fn encoded_key(byte: u8) -> String {
    STANDARD.encode([byte; KEY_LENGTH])
}

fn secrets() -> CredentialSecrets {
    CredentialSecrets::new()
        .with("access_token", b"test-only-access-token".to_vec())
        .expect("access token field is valid")
        .with("refresh_token", b"test-only-refresh-token".to_vec())
        .expect("refresh token field is valid")
}

fn single_key_cipher(version: i32, key_byte: u8) -> CredentialCipher {
    CredentialCipher::parse(&format!("{version}:{}", encoded_key(key_byte)))
        .expect("test keyring should parse")
}

fn assert_matches_secrets(restored: &CredentialSecrets) {
    assert_eq!(
        restored.get("access_token"),
        Some(b"test-only-access-token".as_slice())
    );
    assert_eq!(
        restored.get("refresh_token"),
        Some(b"test-only-refresh-token".as_slice())
    );
    assert_eq!(restored.len(), 2);
}

#[test]
fn round_trip_restores_every_field_with_fresh_nonces() {
    let cipher = single_key_cipher(2, 7);
    let scope_id = Uuid::now_v7();

    let first = cipher.encrypt(scope_id, &secrets()).expect("encrypt");
    let second = cipher.encrypt(scope_id, &secrets()).expect("encrypt again");

    let first_access = &first.fields["access_token"];
    let second_access = &second.fields["access_token"];
    assert_eq!(first_access.nonce.len(), NONCE_LENGTH);
    assert_ne!(first_access.nonce, second_access.nonce);
    assert_ne!(first_access.ciphertext, second_access.ciphertext);
    assert_ne!(
        first_access.ciphertext,
        first.fields["refresh_token"].ciphertext
    );

    assert_matches_secrets(&cipher.decrypt(&first).expect("decrypt"));
    assert_eq!(first.key_version, 2);
    assert_eq!(first.scope_id, scope_id);
}

#[test]
fn wrong_associated_data_is_rejected() {
    let cipher = single_key_cipher(4, 5);
    let encrypted = cipher.encrypt(Uuid::now_v7(), &secrets()).expect("encrypt");

    let mut wrong_scope = encrypted.clone();
    wrong_scope.scope_id = Uuid::now_v7();
    assert!(matches!(
        cipher.decrypt(&wrong_scope),
        Err(CredentialVaultError::DecryptionFailed)
    ));

    let mut renamed_field = encrypted.clone();
    let field = renamed_field
        .fields
        .remove("access_token")
        .expect("access token field");
    renamed_field.fields.insert("api_key".to_owned(), field);
    assert!(matches!(
        cipher.decrypt(&renamed_field),
        Err(CredentialVaultError::DecryptionFailed)
    ));

    let mut swapped_fields = encrypted.clone();
    let access = swapped_fields.fields["access_token"].clone();
    let refresh = swapped_fields.fields["refresh_token"].clone();
    swapped_fields
        .fields
        .insert("access_token".to_owned(), refresh);
    swapped_fields
        .fields
        .insert("refresh_token".to_owned(), access);
    assert!(matches!(
        cipher.decrypt(&swapped_fields),
        Err(CredentialVaultError::DecryptionFailed)
    ));

    let mut wrong_version = encrypted.clone();
    wrong_version.key_version = 99;
    assert!(matches!(
        cipher.decrypt(&wrong_version),
        Err(CredentialVaultError::DecryptionFailed)
    ));

    let wrong_key = single_key_cipher(4, 6);
    assert!(matches!(
        wrong_key.decrypt(&encrypted),
        Err(CredentialVaultError::DecryptionFailed)
    ));
}

#[test]
fn tampered_ciphertext_nonce_and_empty_input_are_rejected() {
    let cipher = single_key_cipher(1, 9);
    let encrypted = cipher.encrypt(Uuid::now_v7(), &secrets()).expect("encrypt");

    let mut flipped = encrypted.clone();
    flipped
        .fields
        .get_mut("access_token")
        .expect("access token field")
        .ciphertext[0] ^= 0x80;
    assert!(matches!(
        cipher.decrypt(&flipped),
        Err(CredentialVaultError::DecryptionFailed)
    ));

    let mut flipped_tag = encrypted.clone();
    let ciphertext = &mut flipped_tag
        .fields
        .get_mut("access_token")
        .expect("access token field")
        .ciphertext;
    let last = ciphertext.len() - 1;
    ciphertext[last] ^= 0x01;
    assert!(matches!(
        cipher.decrypt(&flipped_tag),
        Err(CredentialVaultError::DecryptionFailed)
    ));

    let mut truncated_nonce = encrypted.clone();
    truncated_nonce
        .fields
        .get_mut("access_token")
        .expect("access token field")
        .nonce
        .pop();
    assert!(matches!(
        cipher.decrypt(&truncated_nonce),
        Err(CredentialVaultError::DecryptionFailed)
    ));

    let mut flipped_nonce = encrypted.clone();
    flipped_nonce
        .fields
        .get_mut("access_token")
        .expect("access token field")
        .nonce[0] ^= 0x40;
    assert!(matches!(
        cipher.decrypt(&flipped_nonce),
        Err(CredentialVaultError::DecryptionFailed)
    ));

    let mut empty = encrypted;
    empty.fields.clear();
    assert!(matches!(
        cipher.decrypt(&empty),
        Err(CredentialVaultError::DecryptionFailed)
    ));

    assert!(matches!(
        cipher.encrypt(Uuid::now_v7(), &CredentialSecrets::new()),
        Err(CredentialVaultError::InvalidConfiguration)
    ));
}

#[test]
fn rotation_writes_the_highest_version_and_still_reads_old_ones() {
    let old = single_key_cipher(1, 3);
    let scope_id = Uuid::now_v7();
    let old_ciphertext = old.encrypt(scope_id, &secrets()).expect("encrypt with old");

    let rotated = CredentialCipher::parse(&format!("2:{},1:{}", encoded_key(9), encoded_key(3)))
        .expect("rotated keyring should parse");

    assert_eq!(rotated.active_version(), 2);
    assert_eq!(rotated.known_versions().collect::<Vec<_>>(), vec![1, 2]);
    assert_matches_secrets(&rotated.decrypt(&old_ciphertext).expect("old key reads"));
    assert_eq!(
        rotated
            .encrypt(scope_id, &secrets())
            .expect("encrypt with rotated")
            .key_version,
        2
    );

    // Order in the string does not decide the write key; the highest version does.
    let unordered = CredentialCipher::parse(&format!("1:{}, 2:{}", encoded_key(3), encoded_key(9)))
        .expect("unordered keyring should parse");
    assert_eq!(unordered.active_version(), 2);

    let retired = single_key_cipher(2, 9);
    assert!(matches!(
        retired.decrypt(&old_ciphertext),
        Err(CredentialVaultError::DecryptionFailed)
    ));
}

#[test]
fn malformed_keyrings_are_rejected() {
    let short_key = STANDARD.encode([1_u8; 16]);
    let long_key = STANDARD.encode([1_u8; 48]);
    let duplicate = format!("1:{},1:{}", encoded_key(1), encoded_key(2));
    for value in [
        "",
        "   ",
        ",,",
        "0:AAAA",
        "-1:AAAA",
        "abc:AAAA",
        "1:not-base64!",
        &encoded_key(1),
        &format!("1:{short_key}"),
        &format!("1:{long_key}"),
        &duplicate,
    ] {
        assert!(
            matches!(
                CredentialCipher::parse(value),
                Err(CredentialVaultError::InvalidConfiguration)
            ),
            "keyring {value:?} should be rejected"
        );
    }
}

#[test]
fn secret_fields_reject_invalid_names_and_empty_values() {
    let mut secrets = CredentialSecrets::new();
    assert!(matches!(
        secrets.insert("", b"value".to_vec()),
        Err(CredentialVaultError::InvalidConfiguration)
    ));
    assert!(matches!(
        secrets.insert("has space", b"value".to_vec()),
        Err(CredentialVaultError::InvalidConfiguration)
    ));
    assert!(matches!(
        secrets.insert("a".repeat(65), b"value".to_vec()),
        Err(CredentialVaultError::InvalidConfiguration)
    ));
    assert!(matches!(
        secrets.insert("access_token", Vec::new()),
        Err(CredentialVaultError::InvalidConfiguration)
    ));
    assert!(secrets.is_empty());

    secrets
        .insert("api.key-1", b"value".to_vec())
        .expect("valid");
    assert_eq!(secrets.field_names().collect::<Vec<_>>(), vec!["api.key-1"]);
}

#[test]
fn zeroize_clears_plaintext_fields() {
    let mut secrets = secrets();
    assert_eq!(secrets.len(), 2);
    secrets.zeroize();
    assert!(secrets.is_empty());
    assert_eq!(secrets.get("access_token"), None);
}

#[test]
fn config_parses_validates_and_hides_the_keyring() {
    let disabled = CredentialVaultConfig::default();
    assert!(!disabled.is_enabled());
    assert!(disabled.cipher().expect("no keyring is allowed").is_none());
    assert!(disabled.validate().is_ok());

    let enabled = CredentialVaultConfig {
        keyring: Secret::new(format!("3:{}", encoded_key(4))),
    };
    assert!(enabled.is_enabled());
    assert!(enabled.validate().is_ok());
    assert_eq!(
        enabled
            .cipher()
            .expect("valid keyring")
            .expect("cipher present")
            .active_version(),
        3
    );
    assert!(!format!("{enabled:?}").contains(&encoded_key(4)));

    let broken = CredentialVaultConfig {
        keyring: Secret::new("1:not-base64!".to_owned()),
    };
    let error = broken.validate().expect_err("invalid keyring");
    assert!(error.to_string().contains("keyring"));
    assert!(matches!(
        broken.cipher(),
        Err(CredentialVaultError::InvalidConfiguration)
    ));
}

#[test]
fn cipher_debug_hides_key_material() {
    let rendered = format!("{:?}", single_key_cipher(5, 8));
    assert!(rendered.contains("active_version: 5"));
    assert!(!rendered.contains(&encoded_key(8)));
}
