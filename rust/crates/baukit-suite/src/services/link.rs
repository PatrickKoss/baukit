use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Duration;
use rand::RngCore as _;
use sha2::{Digest as _, Sha256};
use url::Url;

use super::*;

pub struct SuiteLinkService {
    pub(super) context: Arc<SuiteContext>,
}
impl SuiteLinkService {
    pub fn new(context: Arc<SuiteContext>) -> Self {
        Self { context }
    }
    pub async fn peers(&self, owner: Uuid) -> Result<Vec<SuitePeerView>, SuiteServiceError> {
        let links = self.context.store.list_links(owner).await?;
        Ok(self
            .context
            .registry
            .active_peers()
            .iter()
            .map(|p| SuitePeerView {
                id: p.metadata.id.clone(),
                display_name: p.metadata.display_name.clone(),
                sends: p.sends.clone(),
                receives: p.receives.clone(),
                reward_modes: if p.receives.is_empty() {
                    Vec::new()
                } else {
                    self.context.registry.own().reward_modes.clone()
                },
                scheme: p.metadata.scheme.clone(),
                web_url: p.web_url.clone(),
                share_xp_available: self.context.share_xp && !p.sends.is_empty(),
                link: links
                    .iter()
                    .find(|l| l.peer_app == p.metadata.id)
                    .map(|l| l.id),
            })
            .collect())
    }
    pub async fn start(
        &self,
        user: SuiteUser,
        input: LinkStartRequest,
        now: DateTime<Utc>,
    ) -> Result<LinkStartResponse, SuiteServiceError> {
        let peer = validate_link_start(&self.context.registry, &input)?;
        let state = random_secret();
        let verifier = random_secret();
        let id = Uuid::now_v7();
        let request = LinkRequest {
            id,
            user_id: user.id,
            peer_app: input.peer_app,
            state_hash: hash(&state),
            verifier: self
                .context
                .cipher
                .encrypt(id, "pkce_verifier", verifier.as_bytes())
                .map_err(|_| SuiteServiceError::Cipher)?,
            client_state_nonce: input.state_nonce,
            return_url: input.return_url,
            link_id: None,
            exchange: None,
            expires_at: now + Duration::seconds(SUITE_REQUEST_LIFETIME_SECONDS),
            consumed_at: None,
            created_at: now,
        };
        let mut url = endpoint(&peer.web_url, "/suite/authorize")?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("client", &self.context.registry.own().id)
                .append_pair("state", &state)
                .append_pair(
                    "code_challenge",
                    &pkce_challenge(&verifier).map_err(|_| SuiteServiceError::PayloadInvalid)?,
                );
            if let Some(subject) = user.suite_subject {
                pairs.append_pair("hint", &hint_for(&subject));
                if let Some((domain, _)) = subject.split_once('|') {
                    pairs.append_pair("hint_domain", domain);
                }
            }
        }
        let mut tx = self.context.store.begin_transaction().await?;
        self.context
            .store
            .create_request_in_transaction(&mut tx, &request, now)
            .await?;
        self.context.store.commit_transaction(tx).await?;
        Ok(LinkStartResponse {
            request_id: id,
            authorize_url: url.into(),
        })
    }
    pub async fn callback(
        &self,
        input: LinkCallbackQuery,
        now: DateTime<Utc>,
    ) -> Result<CallbackRedirect, SuiteServiceError> {
        let request = if let Some(state) = &input.state {
            self.context
                .store
                .find_request_by_state_hash(hash(state))
                .await?
        } else {
            None
        };
        relay_callback(request.as_ref(), &input, now)
            .map_err(|_| LinkProtocolError::CodeInvalid.into())
    }
    pub async fn complete(
        &self,
        user: SuiteUser,
        id: Uuid,
        code: String,
        now: DateTime<Utc>,
    ) -> Result<SuiteLink, SuiteServiceError> {
        let mut tx = self.context.store.begin_transaction().await?;
        let request = self
            .context
            .store
            .request_for_owner_for_update(&mut tx, id, user.id)
            .await?;
        match validate_complete(request.as_ref(), user.id, now)? {
            CompleteDecision::ExistingLink(id) => {
                let link = self.context.store.link_for_update(&mut tx, id).await?;
                self.context.store.commit_transaction(tx).await?;
                return if link.status == LinkStatus::Revoked {
                    Err(LinkProtocolError::NotFound.into())
                } else {
                    Ok(link)
                };
            }
            CompleteDecision::CreateLink => {}
        }
        if !token_valid(&code) {
            return Err(LinkProtocolError::CodeInvalid.into());
        }
        let request = request.ok_or(LinkProtocolError::NotFound)?;
        let peer = self.context.active_peer(&request.peer_app)?;
        let verifier = String::from_utf8(
            self.context
                .cipher
                .decrypt(id, "pkce_verifier", &request.verifier)
                .map_err(|_| SuiteServiceError::Cipher)?,
        )
        .map_err(|_| SuiteServiceError::Cipher)?;
        let prepared = match request.exchange {
            Some(exchange) => exchange,
            None => {
                let mut secret = [0_u8; 32];
                rand::rng().fill_bytes(&mut secret);
                let exchange = PreparedExchange {
                    link_id: Uuid::now_v7(),
                    secret: self
                        .context
                        .cipher
                        .encrypt(id, "link_secret", &secret)
                        .map_err(|_| SuiteServiceError::Cipher)?,
                };
                self.context
                    .store
                    .prepare_exchange_in_transaction(&mut tx, id, user.id, &exchange)
                    .await?;
                exchange
            }
        };
        self.context.store.commit_transaction(tx).await?;
        let secret = URL_SAFE_NO_PAD.encode(
            self.context
                .cipher
                .decrypt(id, "link_secret", &prepared.secret)
                .map_err(|_| SuiteServiceError::Cipher)?,
        );
        let link_id = prepared.link_id;
        let identity = self.context.identities.identity(user.id).await?;
        let exchange = ExchangeRequest {
            client: self.context.registry.own().id.clone(),
            code,
            code_verifier: verifier,
            initiator_link_id: link_id,
            link_secret: secret.clone(),
            initiator_subject: identity.subject,
            initiator_suite_subject: user.suite_subject.clone(),
            initiator_display_name: identity.display_name,
        };
        let result = match self.context.peer.exchange(peer, &exchange).await {
            Err(SuitePeerError::Timeout) => self.context.peer.exchange(peer, &exchange).await,
            r => r,
        };
        let response = result.map_err(|e| match e {
            SuitePeerError::Rejected { status: 409, .. } => {
                SuiteServiceError::Protocol(LinkProtocolError::AccountMismatch)
            }
            SuitePeerError::Rejected { status: 422, .. } => {
                SuiteServiceError::Protocol(LinkProtocolError::CodeInvalid)
            }
            _ => SuiteServiceError::PeerUnreachable,
        })?;
        if response.authorizer_sends != peer.receives
            || response.initiator_sends != peer.sends
            || response.link_id.get_version_num() != 7
            || !subject_valid(&response.subject)
            || response.display_name.as_ref().is_some_and(|s| {
                s.chars().count() > SUITE_MAX_TEXT_CHARACTERS || s.chars().any(char::is_control)
            })
        {
            return Err(SuiteServiceError::PeerUnreachable);
        }
        let shared = shared_suite_subject(
            user.suite_subject.as_deref(),
            response.suite_subject.as_deref(),
        )
        .map_err(|_| LinkProtocolError::AccountMismatch)?;
        let link = self.new_link(NewLink {
            id: link_id,
            owner: user.id,
            peer,
            role: LinkRole::Initiator,
            remote_id: response.link_id,
            remote_subject: response.subject,
            display_name: response.display_name,
            shared,
            secret,
            now,
        })?;
        let mut tx = self.context.store.begin_transaction().await?;
        self.context
            .store
            .active_link_for_update(&mut tx, user.id, &request.peer_app)
            .await?;
        let current = self
            .context
            .store
            .request_for_owner_for_update(&mut tx, id, user.id)
            .await?;
        if let CompleteDecision::ExistingLink(existing_id) =
            validate_complete(current.as_ref(), user.id, now)?
        {
            let existing = self
                .context
                .store
                .link_for_update(&mut tx, existing_id)
                .await?;
            self.context.store.commit_transaction(tx).await?;
            return if existing.status == LinkStatus::Revoked {
                Err(LinkProtocolError::NotFound.into())
            } else {
                Ok(existing)
            };
        }
        self.context
            .store
            .consume_request_in_transaction(&mut tx, id, user.id, now)
            .await?;
        self.replace(&mut tx, &link, now).await?;
        self.context
            .store
            .record_request_link_in_transaction(&mut tx, id, user.id, link.id)
            .await?;
        self.initial_replay(&mut tx, &link, now).await?;
        let persisted = self.context.store.link_for_update(&mut tx, link.id).await?;
        self.context.store.commit_transaction(tx).await?;
        Ok(persisted)
    }
    pub async fn preview(
        &self,
        user: SuiteUser,
        client: String,
        hint: Option<String>,
        hint_domain: Option<String>,
    ) -> Result<AuthorizationPreview, SuiteServiceError> {
        let existing = self
            .context
            .store
            .list_links(user.id)
            .await?
            .iter()
            .any(|l| l.peer_app == client);
        Ok(preview_authorization(
            &self.context.registry,
            &client,
            existing,
            hint.as_deref(),
            user.suite_subject.as_deref(),
            hint_domain.as_deref(),
        )?)
    }
    pub async fn authorize(
        &self,
        user: SuiteUser,
        input: AuthorizationRequest,
        now: DateTime<Utc>,
    ) -> Result<AuthorizationResponse, SuiteServiceError> {
        let peer = self.context.active_peer(&input.client)?;
        if !token_valid(&input.state) || !token_valid(&input.code_challenge) {
            return Err(SuiteServiceError::PayloadInvalid);
        }
        let preview = AuthorizationPreview::new(
            peer,
            false,
            input.hint.as_deref(),
            user.suite_subject.as_deref(),
            None,
        );
        let code = random_secret();
        let stored = LinkCode {
            code_hash: hash(&code),
            user_id: user.id,
            peer_app: input.client,
            code_challenge: input.code_challenge,
            suite_subject: user.suite_subject,
            auto_approved: preview.auto_approve,
            link_id: None,
            expires_at: now + Duration::seconds(SUITE_CODE_LIFETIME_SECONDS),
            consumed_at: None,
            created_at: now,
        };
        let mut url = endpoint(&peer.api_url, "/suite/links/callback")?;
        url.query_pairs_mut()
            .append_pair("state", &input.state)
            .append_pair("code", &code)
            .append_pair("from", &self.context.registry.own().id);
        let mut tx = self.context.store.begin_transaction().await?;
        self.context
            .store
            .create_code_in_transaction(&mut tx, &stored)
            .await?;
        self.context.store.commit_transaction(tx).await?;
        Ok(AuthorizationResponse {
            redirect_url: url.into(),
        })
    }
    pub fn deny(
        &self,
        input: AuthorizationDeny,
    ) -> Result<AuthorizationResponse, SuiteServiceError> {
        let peer = self.context.active_peer(&input.client)?;
        if !token_valid(&input.state) {
            return Err(SuiteServiceError::PayloadInvalid);
        }
        let mut url = endpoint(&peer.api_url, "/suite/links/callback")?;
        url.query_pairs_mut()
            .append_pair("state", &input.state)
            .append_pair("error", "access_denied")
            .append_pair("from", &self.context.registry.own().id);
        Ok(AuthorizationResponse {
            redirect_url: url.into(),
        })
    }
    pub async fn exchange(
        &self,
        input: ExchangeRequest,
        now: DateTime<Utc>,
    ) -> Result<ExchangeResponse, SuiteServiceError> {
        if input.client.chars().count() > SUITE_MAX_PEER_ID_CHARACTERS {
            return Err(SuiteServiceError::PayloadInvalid);
        }
        let peer = self.context.active_peer(&input.client)?;
        let key = format!("suite:exchange:{}", peer.metadata.id);
        let result = self.exchange_inner(peer, input, now).await;
        if matches!(
            &result,
            Err(SuiteServiceError::Protocol(
                LinkProtocolError::CodeInvalid | LinkProtocolError::AccountMismatch
            ) | SuiteServiceError::PayloadInvalid
                | SuiteServiceError::Store(SuiteStoreError::CodeInvalid))
        ) {
            self.context
                .limit(&key, SUITE_EXCHANGE_FAILURE_LIMIT)
                .await?;
        }
        result
    }
    async fn exchange_inner(
        &self,
        peer: &ActivePeer,
        input: ExchangeRequest,
        now: DateTime<Utc>,
    ) -> Result<ExchangeResponse, SuiteServiceError> {
        if !token_valid(&input.code)
            || !token_valid(&input.link_secret)
            || input.initiator_link_id.get_version_num() != 7
            || !subject_valid(&input.initiator_subject)
            || input.initiator_display_name.as_ref().is_some_and(|s| {
                s.chars().count() > SUITE_MAX_TEXT_CHARACTERS || s.chars().any(char::is_control)
            })
        {
            return Err(SuiteServiceError::PayloadInvalid);
        }
        let mut tx = self.context.store.begin_transaction().await?;
        let code = self
            .context
            .store
            .code_for_update(&mut tx, hash(&input.code))
            .await?;
        let existing = self
            .context
            .store
            .link_for_code_in_transaction(&mut tx, hash(&input.code))
            .await?;
        let decision = validate_exchange(code.as_ref(), &input, existing.as_ref(), now)?;
        if let ExchangeDecision::ExistingLink(_) = decision {
            let link = existing.ok_or(LinkProtocolError::CodeInvalid)?;
            self.context.store.commit_transaction(tx).await?;
            let identity = self.context.identities.identity(link.user_id).await?;
            return Ok(exchange_response(
                &link,
                identity.subject,
                identity.display_name,
            ));
        }
        let code = code.ok_or(LinkProtocolError::CodeInvalid)?;
        let ExchangeDecision::CreateLink { suite_subject } = decision else {
            return Err(LinkProtocolError::CodeInvalid.into());
        };
        let link = self.new_link(NewLink {
            id: Uuid::now_v7(),
            owner: code.user_id,
            peer,
            role: LinkRole::Authorizer,
            remote_id: input.initiator_link_id,
            remote_subject: input.initiator_subject,
            display_name: input.initiator_display_name,
            shared: suite_subject,
            secret: input.link_secret,
            now,
        })?;
        self.replace(&mut tx, &link, now).await?;
        self.context
            .store
            .consume_code_in_transaction(&mut tx, code.code_hash, link.id, now)
            .await?;
        self.initial_replay(&mut tx, &link, now).await?;
        self.context.store.commit_transaction(tx).await?;
        let identity = self.context.identities.identity(link.user_id).await?;
        Ok(exchange_response(
            &link,
            identity.subject,
            identity.display_name,
        ))
    }
    fn new_link(&self, i: NewLink<'_>) -> Result<SuiteLink, SuiteServiceError> {
        let raw = URL_SAFE_NO_PAD
            .decode(i.secret)
            .map_err(|_| SuiteServiceError::PayloadInvalid)?;
        if raw.len() != 32 {
            return Err(SuiteServiceError::PayloadInvalid);
        }
        Ok(SuiteLink {
            id: i.id,
            user_id: i.owner,
            peer_app: i.peer.metadata.id.clone(),
            role: i.role,
            remote_link_id: i.remote_id,
            remote_subject: i.remote_subject,
            remote_display_name: i.display_name,
            suite_subject: i.shared,
            status: LinkStatus::Active,
            secret: self
                .context
                .cipher
                .encrypt(i.id, "link_secret", &raw)
                .map_err(|_| SuiteServiceError::Cipher)?,
            sends: i.peer.sends.clone(),
            receives: i.peer.receives.clone(),
            share_xp: true,
            reward_mode: RewardMode::Native,
            delivery_health: DeliveryHealth::Healthy,
            consecutive_failures: 0,
            last_delivery_at: None,
            last_failure_at: None,
            last_failure_code: None,
            last_received_at: None,
            created_at: i.now,
            updated_at: i.now,
            revoked_at: None,
        })
    }
    async fn replace(
        &self,
        tx: &mut sqlx::PgConnection,
        link: &SuiteLink,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteServiceError> {
        if let Some(old) = self
            .context
            .store
            .active_link_for_update(tx, link.user_id, &link.peer_app)
            .await?
        {
            self.context
                .store
                .revoke_in_transaction(tx, old.id, now)
                .await?;
            self.context
                .outbox
                .enqueue_revoke_in_transaction(tx, old.id)
                .await?;
        }
        self.context
            .store
            .insert_link_in_transaction(tx, link)
            .await?;
        Ok(())
    }
    async fn initial_replay(
        &self,
        tx: &mut sqlx::PgConnection,
        link: &SuiteLink,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteServiceError> {
        let since = now.date_naive() - Duration::days(i64::from(self.context.initial_replay_days));
        let events = self
            .context
            .replay
            .replay_since(tx, link.user_id, since)
            .await?;
        self.context
            .outbox
            .enqueue_replay_in_transaction(tx, link.user_id, link.id, since, now, &events)
            .await?;
        Ok(())
    }
    pub async fn list(&self, owner: Uuid) -> Result<Vec<SuiteLink>, SuiteServiceError> {
        Ok(self.context.store.list_links(owner).await?)
    }
    pub async fn get(&self, owner: Uuid, id: Uuid) -> Result<SuiteLink, SuiteServiceError> {
        Ok(self
            .context
            .store
            .find_link_for_owner(id, owner)
            .await?
            .ok_or(SuiteStoreError::NotFound)?)
    }
    pub async fn update(
        &self,
        owner: Uuid,
        id: Uuid,
        xp: Option<bool>,
        mode: Option<RewardMode>,
        now: DateTime<Utc>,
    ) -> Result<SuiteLink, SuiteServiceError> {
        if mode.is_some_and(|m| !self.context.registry.own().reward_modes.contains(&m)) {
            return Err(SuiteServiceError::PayloadInvalid);
        }
        Ok(self
            .context
            .store
            .update_preferences(owner, id, xp, mode, now)
            .await?)
    }
    pub async fn disconnect(
        &self,
        owner: Uuid,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteServiceError> {
        self.get(owner, id).await?;
        let mut tx = self.context.store.begin_transaction().await?;
        let link = self.context.store.link_for_update(&mut tx, id).await?;
        if link.user_id != owner {
            return Err(LinkProtocolError::NotFound.into());
        }
        if link.status == LinkStatus::Revoked {
            self.context.store.commit_transaction(tx).await?;
            return Ok(());
        }
        self.context
            .store
            .revoke_in_transaction(&mut tx, id, now)
            .await?;
        self.context
            .outbox
            .enqueue_revoke_in_transaction(&mut tx, id)
            .await?;
        self.context.store.commit_transaction(tx).await?;
        Ok(())
    }
    pub async fn reenable(
        &self,
        owner: Uuid,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<SuiteLink, SuiteServiceError> {
        Ok(self.context.store.reenable(owner, id, now).await?)
    }
}
struct NewLink<'a> {
    id: Uuid,
    owner: Uuid,
    peer: &'a ActivePeer,
    role: LinkRole,
    remote_id: Uuid,
    remote_subject: String,
    display_name: Option<String>,
    shared: Option<String>,
    secret: String,
    now: DateTime<Utc>,
}
fn exchange_response(
    link: &SuiteLink,
    subject: String,
    display_name: Option<String>,
) -> ExchangeResponse {
    ExchangeResponse {
        link_id: link.id,
        subject,
        suite_subject: link.suite_subject.clone(),
        display_name,
        authorizer_sends: link.sends.clone(),
        initiator_sends: link.receives.clone(),
    }
}
fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
pub(super) fn hash(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}
fn token_valid(s: &str) -> bool {
    URL_SAFE_NO_PAD
        .decode(s)
        .is_ok_and(|bytes| bytes.len() == 32)
}
fn subject_valid(s: &str) -> bool {
    !s.is_empty() && s.len() <= SUITE_MAX_SUBJECT_BYTES && !s.chars().any(char::is_control)
}
fn endpoint(base: &str, path: &str) -> Result<Url, SuiteServiceError> {
    let mut url = Url::parse(base).map_err(|_| SuiteServiceError::PayloadInvalid)?;
    url.set_path(&format!("{}{}", url.path().trim_end_matches('/'), path));
    Ok(url)
}
