//! P-06: `ApiToken::is_active_at` agrees with `ApiTokenService::verify_at`.

use std::{collections::BTreeSet, sync::Arc};

use baukit_auth::{
    ApiToken, ApiTokenRecord, ApiTokenService, ApiTokenStore, ApiTokenStoreError,
    ApiTokenStoreFuture, StoredApiToken, hash_api_token,
};
use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;
use uuid::Uuid;

const SECRET: &str = "bk_0123456789abcdefghijABCDEFGHIJ01";

struct OneToken(StoredApiToken);

impl ApiTokenStore for OneToken {
    fn create(
        &self,
        _: ApiTokenRecord,
    ) -> ApiTokenStoreFuture<'_, Result<ApiToken, ApiTokenStoreError>> {
        Box::pin(async { Err(ApiTokenStoreError::internal("read only")) })
    }

    fn find_by_hash<'a>(
        &'a self,
        secret_hash: &'a [u8],
    ) -> ApiTokenStoreFuture<'a, Result<Option<StoredApiToken>, ApiTokenStoreError>> {
        let found = (self.0.secret_hash == secret_hash).then(|| self.0.clone());
        Box::pin(async move { Ok(found) })
    }

    fn touch_last_used(
        &self,
        _: Uuid,
        _: Timestamp,
    ) -> ApiTokenStoreFuture<'_, Result<(), ApiTokenStoreError>> {
        Box::pin(async { Ok(()) })
    }

    fn revoke(
        &self,
        _: Uuid,
        _: Uuid,
        _: Timestamp,
    ) -> ApiTokenStoreFuture<'_, Result<bool, ApiTokenStoreError>> {
        Box::pin(async { Ok(false) })
    }

    fn list_for_owner(
        &self,
        _: Uuid,
    ) -> ApiTokenStoreFuture<'_, Result<Vec<ApiToken>, ApiTokenStoreError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn token(created: Timestamp, expires_in: Option<i64>, revoked_in: Option<i64>) -> ApiToken {
    let at = |seconds: i64| created + SignedDuration::from_secs(seconds);
    ApiToken {
        id: Uuid::nil(),
        owner_id: Uuid::nil(),
        name: "ci".to_owned(),
        display_prefix: "bk_0123".to_owned(),
        grants: BTreeSet::new(),
        created_at: created,
        expires_at: expires_in.map(at),
        last_used_at: None,
        revoked_at: revoked_in.map(at),
    }
}

proptest! {
    #[test]
    fn is_active_at_matches_verify_at(
        expires_in in proptest::option::of(-5_i64..5),
        revoked_in in proptest::option::of(-5_i64..5),
        now_offset in -5_i64..5,
    ) {
        let created = Timestamp::from_second(1_800_000_000).expect("timestamp");
        let token = token(created, expires_in, revoked_in);
        let now = created + SignedDuration::from_secs(now_offset);
        let store = OneToken(StoredApiToken { token: token.clone(), secret_hash: hash_api_token(SECRET) });
        let service = ApiTokenService::new(Arc::new(store));
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let verified = runtime.block_on(service.verify_at(SECRET, now));
        prop_assert_eq!(token.is_active_at(now), verified.is_ok(), "{:?}", verified.err());
    }
}

#[test]
fn expiry_equal_to_now_is_inactive() {
    let created = Timestamp::from_second(1_800_000_000).expect("timestamp");
    assert!(!token(created, Some(0), None).is_active_at(created));
}
