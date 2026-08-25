use baukit_config::{Validate, ValidationErrors};
use baukit_credential_vault::{
    CredentialSecrets, CredentialVault, CredentialVaultConfig, CredentialVaultError, VaultFuture,
};
use serde::Deserialize;
use uuid::Uuid;

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

struct PostgresCredentialVault;

impl CredentialVault for PostgresCredentialVault {
    fn store<'a>(
        &'a self,
        owner_id: Uuid,
        scope_id: Uuid,
        secrets: &'a CredentialSecrets,
    ) -> VaultFuture<'a, Result<(), CredentialVaultError>> {
        Box::pin(async move {
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

#[test]
fn readme_product_config_and_port_implementation_compile() {
    let config = ProductConfig::default();
    assert!(config.validate().is_ok());
    let vault: &dyn CredentialVault = &PostgresCredentialVault;
    let _ = vault;
}
