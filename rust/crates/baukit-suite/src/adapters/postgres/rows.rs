use crate::domain::*;
use crate::ports::SuiteStoreError;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::invalid;

#[derive(FromRow)]
pub(super) struct LinkRow {
    pub(super) id: Uuid,
    pub(super) user_id: Uuid,
    pub(super) peer_app: String,
    pub(super) role: String,
    pub(super) remote_link_id: Uuid,
    pub(super) remote_subject: String,
    pub(super) remote_display_name: Option<String>,
    pub(super) suite_subject: Option<String>,
    pub(super) status: String,
    pub(super) secret_ciphertext: Vec<u8>,
    pub(super) secret_nonce: Vec<u8>,
    pub(super) secret_key_version: i32,
    pub(super) sends: Vec<String>,
    pub(super) receives: Vec<String>,
    pub(super) share_xp: bool,
    pub(super) reward_mode: String,
    pub(super) delivery_health: String,
    pub(super) consecutive_failures: i32,
    pub(super) last_delivery_at: Option<DateTime<Utc>>,
    pub(super) last_failure_at: Option<DateTime<Utc>>,
    pub(super) last_failure_code: Option<String>,
    pub(super) last_received_at: Option<DateTime<Utc>>,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
    pub(super) revoked_at: Option<DateTime<Utc>>,
}
impl TryFrom<LinkRow> for SuiteLink {
    type Error = SuiteStoreError;
    fn try_from(r: LinkRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: r.id,
            user_id: r.user_id,
            peer_app: r.peer_app,
            role: r.role.parse().map_err(invalid)?,
            remote_link_id: r.remote_link_id,
            remote_subject: r.remote_subject,
            remote_display_name: r.remote_display_name,
            suite_subject: r.suite_subject,
            status: r.status.parse().map_err(invalid)?,
            secret: EncryptedPayload {
                ciphertext: r.secret_ciphertext,
                nonce: r.secret_nonce,
                key_version: r.secret_key_version,
            },
            sends: r.sends,
            receives: r.receives,
            share_xp: r.share_xp,
            reward_mode: r.reward_mode.parse().map_err(invalid)?,
            delivery_health: r.delivery_health.parse().map_err(invalid)?,
            consecutive_failures: r.consecutive_failures.try_into().map_err(invalid)?,
            last_delivery_at: r.last_delivery_at,
            last_failure_at: r.last_failure_at,
            last_failure_code: r.last_failure_code,
            last_received_at: r.last_received_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
            revoked_at: r.revoked_at,
        })
    }
}
#[derive(FromRow)]
pub(super) struct RequestRow {
    pub(super) id: Uuid,
    pub(super) user_id: Uuid,
    pub(super) peer_app: String,
    pub(super) state_hash: Vec<u8>,
    pub(super) verifier_ciphertext: Vec<u8>,
    pub(super) verifier_nonce: Vec<u8>,
    pub(super) verifier_key_version: i32,
    pub(super) client_state_nonce: String,
    pub(super) return_url: String,
    pub(super) link_id: Option<Uuid>,
    pub(super) initiator_link_id: Option<Uuid>,
    pub(super) secret_ciphertext: Option<Vec<u8>>,
    pub(super) secret_nonce: Option<Vec<u8>>,
    pub(super) secret_key_version: Option<i32>,
    pub(super) expires_at: DateTime<Utc>,
    pub(super) consumed_at: Option<DateTime<Utc>>,
    pub(super) created_at: DateTime<Utc>,
}
impl TryFrom<RequestRow> for LinkRequest {
    type Error = SuiteStoreError;
    fn try_from(r: RequestRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: r.id,
            user_id: r.user_id,
            peer_app: r.peer_app,
            state_hash: r
                .state_hash
                .try_into()
                .map_err(|_| invalid("invalid state hash"))?,
            verifier: EncryptedPayload {
                ciphertext: r.verifier_ciphertext,
                nonce: r.verifier_nonce,
                key_version: r.verifier_key_version,
            },
            exchange: match (
                r.initiator_link_id,
                r.secret_ciphertext,
                r.secret_nonce,
                r.secret_key_version,
            ) {
                (None, None, None, None) => None,
                (Some(link_id), Some(ciphertext), Some(nonce), Some(key_version)) => {
                    Some(PreparedExchange {
                        link_id,
                        secret: EncryptedPayload {
                            ciphertext,
                            nonce,
                            key_version,
                        },
                    })
                }
                _ => return Err(invalid("incomplete exchange credentials")),
            },
            client_state_nonce: r.client_state_nonce,
            return_url: r.return_url,
            link_id: r.link_id,
            expires_at: r.expires_at,
            consumed_at: r.consumed_at,
            created_at: r.created_at,
        })
    }
}
#[derive(FromRow)]
pub(super) struct CodeRow {
    pub(super) code_hash: Vec<u8>,
    pub(super) user_id: Uuid,
    pub(super) peer_app: String,
    pub(super) code_challenge: String,
    pub(super) suite_subject: Option<String>,
    pub(super) auto_approved: bool,
    pub(super) link_id: Option<Uuid>,
    pub(super) expires_at: DateTime<Utc>,
    pub(super) consumed_at: Option<DateTime<Utc>>,
    pub(super) created_at: DateTime<Utc>,
}
impl TryFrom<CodeRow> for LinkCode {
    type Error = SuiteStoreError;
    fn try_from(r: CodeRow) -> Result<Self, Self::Error> {
        Ok(Self {
            code_hash: r
                .code_hash
                .try_into()
                .map_err(|_| invalid("invalid code hash"))?,
            user_id: r.user_id,
            peer_app: r.peer_app,
            code_challenge: r.code_challenge,
            suite_subject: r.suite_subject,
            auto_approved: r.auto_approved,
            link_id: r.link_id,
            expires_at: r.expires_at,
            consumed_at: r.consumed_at,
            created_at: r.created_at,
        })
    }
}
