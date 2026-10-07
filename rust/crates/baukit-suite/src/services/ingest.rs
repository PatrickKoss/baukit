use baukit_core::webhook_signature::verify_webhook_hmac_sha256;
use baukit_events::{EventEnvelope, IngestOutcomeStatus};
use sha2::{Digest as _, Sha256};

use super::*;

#[derive(Clone, Debug)]
pub struct SignedHeaders {
    pub(super) signature: String,
    pub(super) timestamp: i64,
    delivery_id: String,
    pub(super) source: String,
    replay: bool,
}
impl SignedHeaders {
    pub fn parse<'a>(
        headers: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, SuiteServiceError> {
        let mut values = std::collections::BTreeMap::new();
        for (key, value) in headers {
            let key = key.to_ascii_lowercase();
            if matches!(
                key.as_str(),
                "content-type"
                    | "x-suite-signature"
                    | "x-suite-timestamp"
                    | "x-suite-delivery-id"
                    | "x-suite-source"
                    | "x-suite-replay"
            ) && values.insert(key, value).is_some()
            {
                return Err(SuiteServiceError::SignatureInvalid);
            }
        }
        let required = |key| {
            values
                .get(key)
                .copied()
                .ok_or(SuiteServiceError::SignatureInvalid)
        };
        if !required("content-type")?
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
        {
            return Err(SuiteServiceError::SignatureInvalid);
        }
        let text = required("x-suite-timestamp")?;
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SuiteServiceError::SignatureInvalid);
        }
        let timestamp = text
            .parse()
            .map_err(|_| SuiteServiceError::SignatureInvalid)?;
        let delivery_id = required("x-suite-delivery-id")?;
        if delivery_id.is_empty()
            || delivery_id.len() > SUITE_MAX_TEXT_CHARACTERS
            || delivery_id.chars().any(char::is_control)
        {
            return Err(SuiteServiceError::SignatureInvalid);
        }
        let replay = match values.get("x-suite-replay") {
            None => false,
            Some(&"1") => true,
            _ => return Err(SuiteServiceError::SignatureInvalid),
        };
        Ok(Self {
            signature: required("x-suite-signature")?.to_owned(),
            timestamp,
            delivery_id: delivery_id.to_owned(),
            source: required("x-suite-source")?.to_owned(),
            replay,
        })
    }
}
#[derive(Clone, Debug)]
pub struct IngestResult {
    pub duplicate: bool,
    pub outcome: IngestOutcome,
}

pub struct SuiteIngestService {
    context: Arc<SuiteContext>,
    applier: Option<Arc<dyn SuiteEventApplier>>,
}
impl SuiteIngestService {
    pub fn new(context: Arc<SuiteContext>, applier: Arc<dyn SuiteEventApplier>) -> Self {
        Self {
            context,
            applier: Some(applier),
        }
    }
    pub fn with_optional_applier(
        context: Arc<SuiteContext>,
        applier: Option<Arc<dyn SuiteEventApplier>>,
    ) -> Self {
        Self { context, applier }
    }
    fn verify(
        &self,
        link: &SuiteLink,
        h: &SignedHeaders,
        body: &[u8],
        now: DateTime<Utc>,
    ) -> Result<(), SuiteServiceError> {
        let secret = self
            .context
            .cipher
            .decrypt(link.id, "link_secret", &link.secret)
            .map_err(|_| SuiteServiceError::SignatureInvalid)?;
        if !signature_timestamp_valid(h.timestamp, now.timestamp())
            || h.source != link.peer_app
            || !verify_webhook_hmac_sha256(
                [secret.as_slice()],
                h.timestamp,
                &h.delivery_id,
                body,
                &h.signature,
            )
        {
            return Err(SuiteServiceError::SignatureInvalid);
        }
        if link.status == LinkStatus::Revoked {
            return Err(SuiteServiceError::LinkRevoked);
        }
        Ok(())
    }
    async fn authenticate(
        &self,
        id: Uuid,
        h: &SignedHeaders,
        body: &[u8],
        now: DateTime<Utc>,
    ) -> Result<SuiteLink, SuiteServiceError> {
        if body.len() > SUITE_MAX_BODY_BYTES {
            return Err(SuiteServiceError::PayloadInvalid);
        }
        if self.context.registry.active_peer(&h.source).is_none() {
            return Err(SuiteServiceError::SignatureInvalid);
        }
        let Some(link) = self.context.store.find_link(id).await? else {
            self.context
                .limit(
                    &format!("suite:inbound:unknown:{}", h.source),
                    SUITE_INBOUND_UNKNOWN_LIMIT,
                )
                .await?;
            return Err(SuiteServiceError::SignatureInvalid);
        };
        self.context
            .limit(&format!("suite:inbound:{id}"), SUITE_INBOUND_LINK_LIMIT)
            .await?;
        self.verify(&link, h, body, now)?;
        Ok(link)
    }
    pub async fn ingest(
        &self,
        id: Uuid,
        h: SignedHeaders,
        body: Vec<u8>,
        now: DateTime<Utc>,
    ) -> Result<IngestResult, SuiteServiceError> {
        let result = self.ingest_inner(id, h, body, now).await;
        if let Err(error) = &result {
            ingest_metric(
                &self.context.metric_prefix,
                if matches!(error, SuiteServiceError::SignatureInvalid) {
                    "unauthorized"
                } else {
                    "rejected"
                },
            );
        }
        result
    }
    async fn ingest_inner(
        &self,
        id: Uuid,
        h: SignedHeaders,
        body: Vec<u8>,
        now: DateTime<Utc>,
    ) -> Result<IngestResult, SuiteServiceError> {
        let link = self.authenticate(id, &h, &body, now).await?;
        let envelope: EventEnvelope =
            serde_json::from_slice(&body).map_err(|_| SuiteServiceError::PayloadInvalid)?;
        if envelope.event_id != h.delivery_id {
            return Err(SuiteServiceError::PayloadInvalid);
        }
        validate_inbound(&self.context.catalog, &envelope, &link, now, h.replay)?;
        let mut tx = self.context.store.begin_transaction().await?;
        self.context
            .store
            .lock_ingest_user(&mut tx, link.user_id)
            .await?;
        let mut link = self.context.store.link_for_update(&mut tx, id).await?;
        self.verify(&link, &h, &body, now)?;
        let payload = validate_inbound(&self.context.catalog, &envelope, &link, now, h.replay)?;
        let payload = match &payload {
            ValidatedInbound::Activity(payload) => payload,
            ValidatedInbound::ConnectionTest => {
                self.context
                    .store
                    .record_received_in_transaction(&mut tx, id, now)
                    .await?;
                self.context.store.commit_transaction(tx).await?;
                ingest_metric(&self.context.metric_prefix, "no_rule");
                return Ok(IngestResult {
                    duplicate: false,
                    outcome: IngestOutcome {
                        outcome: IngestOutcomeStatus::NoRule,
                        ledger_entry_id: None,
                    },
                });
            }
        };
        let payload_hash = Sha256::digest(
            serde_json::to_vec(&envelope.payload).map_err(|_| SuiteServiceError::PayloadInvalid)?,
        )
        .into();
        if let InboxLookup::Duplicate(stored) = self
            .context
            .store
            .inbox_in_transaction(&mut tx, id, &envelope.event_id, payload_hash)
            .await?
        {
            self.context
                .store
                .record_received_in_transaction(&mut tx, id, now)
                .await?;
            self.context.store.commit_transaction(tx).await?;
            ingest_metric(&self.context.metric_prefix, "duplicate");
            return Ok(IngestResult {
                duplicate: true,
                outcome: IngestOutcome {
                    outcome: IngestOutcomeStatus::Duplicate,
                    ledger_entry_id: stored.ledger_entry_id,
                },
            });
        }
        let mut inbound = SuiteInboundEvent {
            link_id: id,
            event_id: envelope.event_id.clone(),
            user_id: link.user_id,
            event_type: envelope.event_type.clone(),
            occurred_at: envelope.occurred_at,
            payload_hash,
            replay: h.replay,
            outcome: AppliedOutcome {
                outcome: AppliedOutcomeStatus::NoRule,
                ledger_entry_id: None,
            },
            received_at: now,
        };
        self.context
            .store
            .record_inbound_in_transaction(&mut tx, &inbound)
            .await?;
        if h.replay {
            link.reward_mode = RewardMode::Off;
        }
        let outcome = self
            .applier
            .as_ref()
            .ok_or(SuiteServiceError::Unavailable)?
            .apply(
                &mut tx,
                SuiteApplyEvent {
                    owner_id: link.user_id,
                    link: &link,
                    envelope: &envelope,
                    payload,
                    reward_mode: link.reward_mode,
                    payload_hash,
                    replay: h.replay,
                    received_at: now,
                },
            )
            .await?;
        if h.replay
            && (outcome.outcome == AppliedOutcomeStatus::Granted
                || outcome.ledger_entry_id.is_some())
        {
            return Err(
                SuiteStoreError::InvalidData("replay applier granted a reward".to_owned()).into(),
            );
        }
        inbound.outcome = outcome.clone();
        self.context
            .store
            .record_inbound_in_transaction(&mut tx, &inbound)
            .await?;
        self.context.store.commit_transaction(tx).await?;
        ingest_metric(
            &self.context.metric_prefix,
            match outcome.outcome {
                AppliedOutcomeStatus::Granted => "granted",
                AppliedOutcomeStatus::NoRule => "no_rule",
                AppliedOutcomeStatus::Capped => "capped",
            },
        );
        Ok(IngestResult {
            duplicate: false,
            outcome: outcome.into(),
        })
    }
    pub async fn revoke_from_peer(
        &self,
        id: Uuid,
        h: SignedHeaders,
        body: Vec<u8>,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteServiceError> {
        if body.len() > SUITE_MAX_BODY_BYTES
            || h.replay
            || Uuid::parse_str(&h.delivery_id).is_err()
            || serde_json::from_slice::<serde_json::Value>(&body).ok()
                != Some(serde_json::json!({}))
        {
            return Err(SuiteServiceError::PayloadInvalid);
        }
        match self.authenticate(id, &h, &body, now).await {
            Ok(_) => {}
            Err(SuiteServiceError::LinkRevoked) => return Ok(()),
            Err(e) => return Err(e),
        }
        let mut tx = self.context.store.begin_transaction().await?;
        let link = self.context.store.link_for_update(&mut tx, id).await?;
        match self.verify(&link, &h, &body, now) {
            Ok(()) | Err(SuiteServiceError::LinkRevoked) => {}
            Err(e) => return Err(e),
        }
        self.context
            .store
            .revoke_in_transaction(&mut tx, id, now)
            .await?;
        self.context.store.commit_transaction(tx).await?;
        Ok(())
    }
}
