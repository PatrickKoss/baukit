use super::*;

pub struct SuiteDeliveryService {
    context: Arc<SuiteContext>,
}
impl SuiteDeliveryService {
    pub fn new(context: Arc<SuiteContext>) -> Self {
        Self { context }
    }
    pub async fn deliver(
        &self,
        job: SuiteDeliverJobV1,
        now: DateTime<Utc>,
    ) -> Result<DeliveryAction, SuiteServiceError> {
        self.deliver_claimed(job, now, None).await
    }
    pub async fn deliver_claimed(
        &self,
        job: SuiteDeliverJobV1,
        now: DateTime<Utc>,
        completion: Option<(Uuid, &str)>,
    ) -> Result<DeliveryAction, SuiteServiceError> {
        let link = self.context.store.find_link(job.link_id).await?;
        if let Some(action) = delivery_preflight(link.as_ref()) {
            delivery_metric(&self.context.metric_prefix, action);
            return Ok(action);
        }
        let link = link.ok_or(SuiteStoreError::NotFound)?;
        let peer = self.context.active_peer(&link.peer_app)?;
        let secret = self
            .context
            .cipher
            .decrypt(link.id, "link_secret", &link.secret)
            .map_err(|_| SuiteServiceError::Cipher)?;
        let body =
            serde_json::to_vec(&job.envelope).map_err(|_| SuiteServiceError::PayloadInvalid)?;
        let response = self
            .context
            .peer
            .deliver(
                peer,
                SuiteSignedCall {
                    remote_link_id: link.remote_link_id,
                    delivery_id: &job.envelope.event_id,
                    timestamp: now.timestamp(),
                    source_app: &self.context.registry.own().id,
                    secret: &secret,
                    body: &body,
                    replay: job.replay,
                },
            )
            .await;
        let action = peer_action(response);
        if action == DeliveryAction::Delivered
            && let Some((job_id, worker_id)) = completion
        {
            self.context
                .store
                .complete_delivery(link.id, job_id, worker_id, now)
                .await?;
        } else {
            self.context
                .store
                .record_delivery(link.id, action, now)
                .await?;
        }
        delivery_metric(&self.context.metric_prefix, action);
        Ok(action)
    }
    pub async fn revoke(
        &self,
        job: SuiteRevokeJobV1,
        now: DateTime<Utc>,
    ) -> Result<DeliveryAction, SuiteServiceError> {
        let Some(link) = self.context.store.find_link(job.link_id).await? else {
            return Ok(DeliveryAction::LinkMissing);
        };
        self.revoke_link(&link, now).await
    }
    async fn revoke_link(
        &self,
        link: &SuiteLink,
        now: DateTime<Utc>,
    ) -> Result<DeliveryAction, SuiteServiceError> {
        let peer = self.context.active_peer(&link.peer_app)?;
        let secret = self
            .context
            .cipher
            .decrypt(link.id, "link_secret", &link.secret)
            .map_err(|_| SuiteServiceError::Cipher)?;
        let delivery_id = Uuid::now_v7().to_string();
        let response = self
            .context
            .peer
            .revoke(
                peer,
                SuiteSignedCall {
                    remote_link_id: link.remote_link_id,
                    delivery_id: &delivery_id,
                    timestamp: now.timestamp(),
                    source_app: &self.context.registry.own().id,
                    secret: &secret,
                    body: b"{}",
                    replay: false,
                },
            )
            .await;
        Ok(peer_action(response))
    }
    pub async fn replay(
        &self,
        owner: Uuid,
        id: Uuid,
        since: NaiveDate,
        now: DateTime<Utc>,
    ) -> Result<u64, SuiteServiceError> {
        validate_replay_since(since, now.date_naive()).map_err(|_| {
            SuiteServiceError::ReplayWindow {
                earliest: replay_earliest(now),
                latest: now.date_naive(),
            }
        })?;
        let link = self
            .context
            .store
            .find_link_for_owner(id, owner)
            .await?
            .ok_or(SuiteStoreError::NotFound)?;
        ensure_active(&link)?;
        self.context.active_peer(&link.peer_app)?;
        let retry_after = self.context.store.replay_retry_after(id, now).await?;
        if retry_after > 0 {
            return Err(SuiteStoreError::ReplayTooSoon(retry_after).into());
        }
        let mut tx = self.context.store.begin_transaction().await?;
        self.context.store.lock_ingest_user(&mut tx, owner).await?;
        let events = self
            .context
            .replay
            .replay_since(&mut tx, owner, since)
            .await?;
        let count = self
            .context
            .outbox
            .enqueue_replay_in_transaction(&mut tx, owner, id, since, now, &events)
            .await?;
        self.context.store.commit_transaction(tx).await?;
        Ok(count)
    }
    pub async fn test(
        &self,
        owner: Uuid,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteServiceError> {
        let link = self
            .context
            .store
            .find_link_for_owner(id, owner)
            .await?
            .ok_or(SuiteStoreError::NotFound)?;
        ensure_active(&link)?;
        self.context.active_peer(&link.peer_app)?;
        let mut tx = self.context.store.begin_transaction().await?;
        self.context
            .outbox
            .enqueue_test_in_transaction(
                &mut tx,
                owner,
                id,
                &SuiteEvent::connection_test(Uuid::now_v7(), now),
            )
            .await?;
        self.context.store.commit_transaction(tx).await?;
        Ok(())
    }
    pub async fn cleanup(
        &self,
        now: DateTime<Utc>,
    ) -> Result<SuiteCleanupOutcome, SuiteServiceError> {
        Ok(self.context.store.cleanup(now).await?)
    }
}
fn peer_action(response: Result<SuitePeerResponse, SuitePeerError>) -> DeliveryAction {
    map_delivery_result(match response {
        Ok(r) => DeliveryResult::Http {
            status: r.status,
            retry_after: r.retry_after,
        },
        Err(SuitePeerError::Timeout) => DeliveryResult::Timeout,
        Err(SuitePeerError::Dns) => DeliveryResult::Dns,
        Err(SuitePeerError::Transport) => DeliveryResult::Transport,
        Err(SuitePeerError::Rejected { status, .. }) => DeliveryResult::Http {
            status,
            retry_after: None,
        },
        Err(SuitePeerError::InvalidResponse) => DeliveryResult::Http {
            status: 400,
            retry_after: None,
        },
    })
}

fn ensure_active(link: &SuiteLink) -> Result<(), SuiteServiceError> {
    if link.status == LinkStatus::Revoked {
        return Err(SuiteServiceError::LinkRevoked);
    }
    if link.status != LinkStatus::Active
        || !link.can_enqueue()
        || link.delivery_health == DeliveryHealth::NeedsAttention
    {
        return Err(SuiteServiceError::LinkInactive);
    }
    Ok(())
}

#[async_trait]
impl SuiteProfileRevoker for SuiteDeliveryService {
    async fn revoke_before_erasure(&self, links: &[SuiteLink]) {
        for link in links {
            let result = tokio::time::timeout(
                Duration::from_millis(SUITE_ERASURE_REVOKE_TIMEOUT_MILLIS),
                self.revoke_link(link, Utc::now()),
            )
            .await;
            match result {
                Ok(Ok(DeliveryAction::Delivered | DeliveryAction::Revoked)) => {}
                Ok(Ok(action)) => {
                    tracing::warn!(link_id=%link.id, outcome=?action, "Suite erasure revoke was rejected")
                }
                Ok(Err(error)) => {
                    tracing::warn!(link_id=%link.id, code=error.code(), "Suite erasure revoke failed")
                }
                Err(_) => tracing::warn!(link_id=%link.id, "Suite erasure revoke timed out"),
            }
        }
    }
}
