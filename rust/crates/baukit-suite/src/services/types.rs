use crate::domain::{RewardMode, SUITE_MAX_REPLAY_DAYS, SuiteLink};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Default)]
pub struct SuiteUser {
    pub id: Uuid,
    pub suite_subject: Option<String>,
    pub display_name: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizationRequest {
    pub client: String,
    pub state: String,
    pub code_challenge: String,
    pub hint: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizationDeny {
    pub client: String,
    pub state: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationResponse {
    pub redirect_url: String,
}
#[derive(Clone, Debug)]
pub struct SuiteLinkView {
    pub link: SuiteLink,
    pub replay_earliest: NaiveDate,
}

pub fn replay_earliest(now: DateTime<Utc>) -> NaiveDate {
    now.date_naive() - chrono::Duration::days(i64::from(SUITE_MAX_REPLAY_DAYS))
}

impl SuiteLinkView {
    #[cfg(feature = "runtime")]
    pub(super) fn new(link: SuiteLink, now: DateTime<Utc>) -> Self {
        Self {
            link,
            replay_earliest: replay_earliest(now),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuitePeerView {
    pub id: String,
    pub display_name: String,
    pub scheme: String,
    pub web_url: String,
    pub share_xp_available: bool,
    pub sends: Vec<String>,
    pub receives: Vec<String>,
    pub reward_modes: Vec<RewardMode>,
    pub link: Option<Uuid>,
}
