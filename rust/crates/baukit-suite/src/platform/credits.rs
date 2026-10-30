use super::{HubClient, HubError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// The hub rejects amounts above the largest JSON-safe integer.
pub const MAX_CREDIT_AMOUNT: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationStatus {
    Pending,
    Settled,
    Released,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditReservation {
    pub id: Uuid,
    pub wallet: String,
    pub amount: u64,
    #[serde(default)]
    pub charged: Option<u64>,
    pub status: ReservationStatus,
    /// Unix seconds.
    pub expires_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditPool {
    /// `family` or `organization`.
    pub owner_kind: String,
    pub available: u64,
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditGrant {
    pub granted: u64,
    pub used: u64,
    pub available: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditPack {
    pub remaining: u64,
    /// Unix seconds.
    pub expires_at: i64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditBalance {
    pub suite_subject: String,
    pub wallet: String,
    pub available: u64,
    /// Counts `credits.changed` events for this wallet.
    pub revision: u64,
    pub pools: Vec<CreditPool>,
    pub grant: CreditGrant,
    pub packs: Vec<CreditPack>,
}

/// A zero-cost action holds nothing and needs no settle or release.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreditHold {
    Free,
    Reserved(CreditReservation),
}

/// Hub credit error codes map to these variants. Other hub failures keep their
/// `HubError`.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CreditsError {
    #[error("credits_insufficient")]
    Insufficient,
    #[error("credits_idempotency_conflict")]
    IdempotencyConflict,
    #[error("credits_reservation_expired")]
    ReservationExpired,
    #[error("credits_reservation_not_found")]
    ReservationNotFound,
    #[error("credits_wallet_unknown")]
    WalletUnknown,
    #[error("credits_amount_invalid")]
    AmountInvalid,
    #[error("credits_idempotency_key_invalid")]
    IdempotencyKeyInvalid,
    #[error("platform_subject_not_found")]
    SubjectNotFound,
    #[error("account_inactive")]
    AccountInactive,
    /// `platform_credits_unavailable`, rate limiting or a transport failure. Retry later.
    #[error("credits_unavailable")]
    Unavailable,
    #[error(transparent)]
    Hub(HubError),
}
impl From<HubError> for CreditsError {
    fn from(error: HubError) -> Self {
        let HubError::Rejected {
            status,
            code: Some(code),
        } = &error
        else {
            return match error {
                HubError::Unavailable => Self::Unavailable,
                other => Self::Hub(other),
            };
        };
        match (*status, code.as_str()) {
            (409, "credits_insufficient") => Self::Insufficient,
            (409, "credits_idempotency_conflict") => Self::IdempotencyConflict,
            (409, "credits_reservation_expired") => Self::ReservationExpired,
            (404, "credits_reservation_not_found") => Self::ReservationNotFound,
            (404, "credits_wallet_unknown") => Self::WalletUnknown,
            (400, "credits_amount_invalid") => Self::AmountInvalid,
            (400, "credits_idempotency_key_invalid") => Self::IdempotencyKeyInvalid,
            (404, "platform_subject_not_found") => Self::SubjectNotFound,
            (403, "account_inactive") => Self::AccountInactive,
            _ => Self::Hub(error),
        }
    }
}

/// Spends the calling product's own wallet. Products name their credit actions
/// and keep each action's cost in product configuration, defaulting to zero.
#[async_trait]
pub trait Credits: Send + Sync {
    /// A zero amount returns `CreditHold::Free` without a ledger call. Repeating
    /// a key with the same amount returns the first reservation.
    async fn reserve(
        &self,
        suite_subject: &str,
        amount: u64,
        idempotency_key: &str,
    ) -> Result<CreditHold, CreditsError>;
    /// Charges at most the reserved amount and releases the rest.
    async fn settle(
        &self,
        reservation_id: Uuid,
        actual_amount: u64,
    ) -> Result<CreditReservation, CreditsError>;
    async fn release(&self, reservation_id: Uuid) -> Result<CreditReservation, CreditsError>;
    async fn balance(&self, suite_subject: &str) -> Result<CreditBalance, CreditsError>;
}

/// Standalone credits: every zero-cost action proceeds, and an action with a
/// configured cost fails with `WalletUnknown` because no hub wallet exists.
#[derive(Clone, Debug)]
pub struct ZeroCostCredits {
    product_id: String,
}
impl ZeroCostCredits {
    pub fn new(product_id: String) -> Self {
        Self { product_id }
    }
}
#[async_trait]
impl Credits for ZeroCostCredits {
    async fn reserve(
        &self,
        _suite_subject: &str,
        amount: u64,
        _idempotency_key: &str,
    ) -> Result<CreditHold, CreditsError> {
        match amount {
            0 => Ok(CreditHold::Free),
            1..=MAX_CREDIT_AMOUNT => Err(CreditsError::WalletUnknown),
            _ => Err(CreditsError::AmountInvalid),
        }
    }
    async fn settle(&self, _: Uuid, _: u64) -> Result<CreditReservation, CreditsError> {
        Err(CreditsError::ReservationNotFound)
    }
    async fn release(&self, _: Uuid) -> Result<CreditReservation, CreditsError> {
        Err(CreditsError::ReservationNotFound)
    }
    async fn balance(&self, suite_subject: &str) -> Result<CreditBalance, CreditsError> {
        Ok(CreditBalance {
            suite_subject: suite_subject.into(),
            wallet: self.product_id.clone(),
            available: 0,
            revision: 0,
            pools: Vec::new(),
            grant: CreditGrant::default(),
            packs: Vec::new(),
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReserveRequest<'a> {
    suite_subject: &'a str,
    amount: u64,
    idempotency_key: &'a str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettleRequest {
    actual_amount: u64,
}
#[derive(Serialize)]
struct ReleaseRequest {}

/// Hub credits through `/platform/v1/credits`. Requires the `credits:read`
/// and `credits:spend` scopes. Responses for another wallet or subject are
/// rejected as `HubError::InvalidResponse`.
#[derive(Clone)]
pub struct HubCredits {
    hub: Arc<HubClient>,
}
impl HubCredits {
    pub fn new(hub: Arc<HubClient>) -> Self {
        Self { hub }
    }
    fn owned(&self, reservation: CreditReservation) -> Result<CreditReservation, CreditsError> {
        if reservation.wallet != self.hub.product_id() {
            return Err(CreditsError::Hub(HubError::InvalidResponse));
        }
        Ok(reservation)
    }
    async fn finish(
        &self,
        reservation_id: Uuid,
        action: &str,
        body: &(impl Serialize + Sync),
    ) -> Result<CreditReservation, CreditsError> {
        let id = reservation_id.to_string();
        let reservation = self
            .hub
            .post_json(&["credits", "reservations", &id, action], body)
            .await?;
        let reservation = self.owned(reservation)?;
        if reservation.id != reservation_id {
            return Err(CreditsError::Hub(HubError::InvalidResponse));
        }
        Ok(reservation)
    }
}
#[async_trait]
impl Credits for HubCredits {
    async fn reserve(
        &self,
        suite_subject: &str,
        amount: u64,
        idempotency_key: &str,
    ) -> Result<CreditHold, CreditsError> {
        if amount == 0 {
            return Ok(CreditHold::Free);
        }
        if amount > MAX_CREDIT_AMOUNT {
            return Err(CreditsError::AmountInvalid);
        }
        let request = ReserveRequest {
            suite_subject,
            amount,
            idempotency_key,
        };
        let reservation = self
            .hub
            .post_json(&["credits", "reservations"], &request)
            .await?;
        let reservation = self.owned(reservation)?;
        if reservation.amount != amount {
            return Err(CreditsError::Hub(HubError::InvalidResponse));
        }
        Ok(CreditHold::Reserved(reservation))
    }
    async fn settle(
        &self,
        reservation_id: Uuid,
        actual_amount: u64,
    ) -> Result<CreditReservation, CreditsError> {
        if actual_amount > MAX_CREDIT_AMOUNT {
            return Err(CreditsError::AmountInvalid);
        }
        self.finish(reservation_id, "settle", &SettleRequest { actual_amount })
            .await
    }
    async fn release(&self, reservation_id: Uuid) -> Result<CreditReservation, CreditsError> {
        self.finish(reservation_id, "release", &ReleaseRequest {})
            .await
    }
    async fn balance(&self, suite_subject: &str) -> Result<CreditBalance, CreditsError> {
        let balance: CreditBalance = self.hub.get_json(&["credits", suite_subject]).await?;
        if balance.wallet != self.hub.product_id() || balance.suite_subject != suite_subject {
            return Err(CreditsError::Hub(HubError::InvalidResponse));
        }
        Ok(balance)
    }
}
