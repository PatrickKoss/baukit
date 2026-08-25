# baukit-credential-vault

`baukit-credential-vault` encrypts credentials a service holds on someone
else's behalf: OAuth access and refresh tokens, provider API keys, webhook
signing secrets. It supplies a versioned AES-256-GCM keyring read from
configuration, per-field envelope encryption, and key rotation that needs no
data migration.

The storage adapter stays in the product. Table shape, ownership joins, and
soft-delete rules are product decisions, so the crate ships only the
`CredentialVault` port and the cipher that an adapter uses on the way in and
out of that table.

## Encryption model

Every secret field is sealed on its own, with a fresh random 12-byte nonce and
associated data built from three parts:

```text
scope_id (16 bytes) || key_version (4 bytes, big endian) || field name (UTF-8)
```

Binding all three means a ciphertext copied out of one row cannot be pasted
into another scope's row, moved from `refresh_token` into `access_token`, or
replayed under a different key version. Every one of those attempts fails as
`CredentialVaultError::DecryptionFailed`, the same error a wrong key or a
flipped bit produces. The variant carries no detail, so a decryption failure
cannot become an oracle in a log line.

Plaintext lives in `CredentialSecrets`, which zeroizes every value on drop and
implements neither `Debug` nor `Display`. Parsed key material is zeroized when
the cipher's last clone drops.

## Keyring format

One environment value holds the whole ring as comma-separated
`version:base64key` entries:

```text
2:bqZ0…=,1:8fA1…=
```

- `version` is a positive `i32` and must be unique in the ring.
- The key is standard base64 (with padding) of exactly 32 random bytes.
- Whitespace around entries and around the colon is ignored.
- **The highest version is the write key.** Order in the string does not
  matter. Every listed version stays readable.

Generate a key with:

```bash
openssl rand -base64 32
```

An empty value means the product runs without a vault. `CredentialVaultConfig`
then reports `is_enabled() == false` and `cipher()` returns `None` rather than
an error, so a deployment that stores no third-party credentials needs no
keyring at all.

## Configuration

Nest `CredentialVaultConfig` in the product configuration that
`baukit_config::BaukitConfig` flattens:

```rust
use baukit_config::{Validate, ValidationErrors};
use baukit_credential_vault::CredentialVaultConfig;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProductConfig {
    credential_vault: CredentialVaultConfig,
}

impl Validate for ProductConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        self.credential_vault.validate()
    }
}
```

The environment override then follows the standard application-prefixed form:

```text
ORDERS__CREDENTIAL_VAULT__KEYRING=2:bqZ0…=,1:8fA1…=
```

`CredentialVaultConfig::validate` rejects a malformed ring at startup, so a
typo in the secret fails the process instead of surfacing later as a
decryption error on live traffic.

## Rotation

Rotation is two deploys and a backfill in between. Nothing is re-encrypted
eagerly, and no window exists where stored ciphertext is unreadable.

1. **Add the new key.** Prepend a higher version to the ring and keep the old
   one:

   ```text
   ORDERS__CREDENTIAL_VAULT__KEYRING=2:<new key>,1:<old key>
   ```

   After this deploy, new writes use version 2 while version 1 rows keep
   decrypting.

2. **Re-encrypt the stored rows.** Load each credential set through the vault
   and store it again. The cipher writes version 2 on the way back out, and the
   row's `key_version` column moves with it. Products with few rows can do this
   lazily by re-storing on the next successful provider refresh; products that
   want a hard cutover run a one-off job.

3. **Retire the old key.** Once no row reports `key_version = 1`, drop it from
   the ring:

   ```text
   ORDERS__CREDENTIAL_VAULT__KEYRING=2:<new key>
   ```

Check step 3 against the data before you do it. A row still on the retired
version decrypts to `DecryptionFailed` afterwards, and the plaintext is not
recoverable without the removed key. Keep the retired key in your secret
manager's history until the backfill is confirmed complete.

## Implementing the port

```rust
use baukit_credential_vault::{
    CredentialSecrets, CredentialVault, CredentialVaultError, VaultFuture,
};
use uuid::Uuid;

struct PostgresCredentialVault {
    // pool + CredentialCipher
}

impl CredentialVault for PostgresCredentialVault {
    fn store<'a>(
        &'a self,
        owner_id: Uuid,
        scope_id: Uuid,
        secrets: &'a CredentialSecrets,
    ) -> VaultFuture<'a, Result<(), CredentialVaultError>> {
        Box::pin(async move {
            // cipher.encrypt(scope_id, secrets), then upsert the encrypted
            // fields joined against a scope this owner_id owns.
            let _ = (owner_id, scope_id, secrets);
            Ok(())
        })
    }

    fn load(
        &self,
        scope_id: Uuid,
    ) -> VaultFuture<'_, Result<CredentialSecrets, CredentialVaultError>> {
        Box::pin(async move {
            let _ = scope_id;
            Err(CredentialVaultError::NotFound)
        })
    }

    fn delete(
        &self,
        owner_id: Uuid,
        scope_id: Uuid,
    ) -> VaultFuture<'_, Result<(), CredentialVaultError>> {
        Box::pin(async move {
            let _ = (owner_id, scope_id);
            Ok(())
        })
    }
}
```

Store the ciphertext, the nonce, and the `key_version` per field. `store` and
`delete` take `owner_id` so the adapter can enforce ownership inside the same
statement rather than trusting a prior read.

Never log a `CredentialSecrets` value, a decrypted field, or the keyring. See
[integration reliability](../../../docs/platform/integration-reliability.md)
for the surrounding rules on provider credentials.
