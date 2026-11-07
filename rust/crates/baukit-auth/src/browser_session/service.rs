use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use aws_lc_rs::rand::SystemRandom;
use baukit_credential_vault::CredentialCipher;
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    crypto::{
        self, CryptoError, GRACE_ACCESS_TOKEN_FIELD, ID_TOKEN_FIELD, REFRESH_TOKEN_FIELD,
        SessionCredential,
    },
    idp::{IdentityProvider, IdpError, TokenSet},
    store::{SessionRow, SessionStore, SessionTransaction},
};

/// How long a rotated secret may still fetch its successor's access token.
pub(crate) const GRACE_WINDOW: Duration = Duration::from_secs(10);

pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// An access token for the browser, with the cookie to set when one rotated.
pub(crate) struct Issued {
    pub(crate) access_token: Zeroizing<String>,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) subject: String,
    pub(crate) cookie: Option<(SessionCredential, Duration)>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error("the authorization grant was rejected")]
    InvalidGrant,
    #[error("the browser session expired")]
    SessionExpired,
    #[error("the IdP is unavailable")]
    IdpUnavailable(#[source] IdpError),
    #[error("browser session storage failed")]
    Store(#[from] sqlx::Error),
    #[error("browser session encryption failed")]
    Crypto(#[from] CryptoError),
    #[error("the IdP response is invalid")]
    InvalidIdpResponse(#[source] IdpError),
}

fn idp_error(error: IdpError) -> SessionError {
    match error {
        IdpError::InvalidGrant => SessionError::InvalidGrant,
        IdpError::Unavailable(_) | IdpError::KeysUnavailable(_) => {
            SessionError::IdpUnavailable(error)
        }
        IdpError::InvalidResponse(_) | IdpError::InvalidIdToken(_) => {
            SessionError::InvalidIdpResponse(error)
        }
    }
}

pub(crate) struct SessionSettings {
    pub(crate) cipher: CredentialCipher,
    pub(crate) redirect_uris: BTreeSet<String>,
    pub(crate) idle_timeout: Duration,
    pub(crate) absolute_timeout: Duration,
}

pub(crate) struct SessionService {
    settings: SessionSettings,
    store: Arc<dyn SessionStore>,
    idp: Arc<dyn IdentityProvider>,
    clock: Arc<dyn Clock>,
    random: SystemRandom,
    checked_refresh_lifetime: AtomicBool,
}

struct Lifetimes {
    idle_expires_at: DateTime<Utc>,
    absolute_expires_at: DateTime<Utc>,
    max_age: Duration,
}

impl SessionService {
    pub(crate) fn new(
        settings: SessionSettings,
        store: Arc<dyn SessionStore>,
        idp: Arc<dyn IdentityProvider>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            settings,
            store,
            idp,
            clock,
            random: SystemRandom::new(),
            checked_refresh_lifetime: AtomicBool::new(false),
        }
    }

    pub(crate) async fn exchange(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<Issued, SessionError> {
        if !self.settings.redirect_uris.contains(redirect_uri) {
            return Err(SessionError::InvalidGrant);
        }
        let tokens = self
            .idp
            .exchange(code, code_verifier, redirect_uri)
            .await
            .map_err(idp_error)?;
        let id_token = tokens
            .id_token
            .as_ref()
            .ok_or(SessionError::InvalidIdpResponse(IdpError::InvalidResponse(
                "token response lacks an ID token",
            )))?;
        let subject = self.idp.subject(id_token).await.map_err(idp_error)?;
        let refresh_token =
            tokens
                .refresh_token
                .as_ref()
                .ok_or(SessionError::InvalidIdpResponse(IdpError::InvalidResponse(
                    "token response lacks a refresh token",
                )))?;
        let now = self.clock.now();
        let absolute_expires_at = after(now, self.settings.absolute_timeout);
        let credential = SessionCredential::generate(&self.random)?;
        let lifetimes = self.lifetimes(now, absolute_expires_at, tokens.refresh_expires_in);
        let row = self.new_row(
            &credential,
            credential.id,
            &subject,
            refresh_token,
            Some(id_token),
            None,
            now,
            &lifetimes,
        )?;
        let mut transaction = self.store.begin().await?;
        transaction.insert(&row).await?;
        transaction.commit().await?;
        Ok(Issued {
            access_token: tokens.access_token.clone(),
            expires_at: after(now, tokens.expires_in),
            subject,
            cookie: Some((credential, lifetimes.max_age)),
        })
    }

    pub(crate) async fn refresh(
        &self,
        credential: &SessionCredential,
    ) -> Result<Issued, SessionError> {
        let mut transaction = self.store.begin().await?;
        let row = locked_match(transaction.as_mut(), credential).await?;
        let now = self.clock.now();
        if let Some(successor) = row.rotated_to {
            return self.replay(transaction, &row, successor, now).await;
        }
        if now >= row.absolute_expires_at || now >= row.idle_expires_at {
            transaction.revoke_family(row.family_id, now).await?;
            transaction.commit().await?;
            return Err(SessionError::SessionExpired);
        }
        let refresh_token = self.open(&row, REFRESH_TOKEN_FIELD, &row.refresh_token)?;
        let tokens = match self.idp.refresh(&refresh_token).await {
            Ok(tokens) => tokens,
            Err(IdpError::InvalidGrant) => {
                transaction.revoke_family(row.family_id, now).await?;
                transaction.commit().await?;
                return Err(SessionError::SessionExpired);
            }
            Err(error) => return Err(idp_error(error)),
        };
        self.rotate(transaction, &row, &refresh_token, tokens, now)
            .await
    }

    async fn rotate(
        &self,
        mut transaction: Box<dyn SessionTransaction>,
        row: &SessionRow,
        refresh_token: &str,
        tokens: TokenSet,
        now: DateTime<Utc>,
    ) -> Result<Issued, SessionError> {
        if let Some(id_token) = &tokens.id_token
            && self.idp.subject(id_token).await.map_err(idp_error)? != row.subject
        {
            return Err(SessionError::InvalidIdpResponse(IdpError::InvalidResponse(
                "refreshed ID token names another subject",
            )));
        }
        let previous_id_token = row
            .id_token
            .as_ref()
            .map(|sealed| self.open(row, ID_TOKEN_FIELD, sealed))
            .transpose()?;
        let id_token = tokens.id_token.as_ref().or(previous_id_token.as_ref());
        let refresh_token = tokens
            .refresh_token
            .as_ref()
            .map_or(refresh_token, |token| token.as_str());
        let credential = SessionCredential::generate(&self.random)?;
        let lifetimes = self.lifetimes(now, row.absolute_expires_at, tokens.refresh_expires_in);
        let expires_at = after(now, tokens.expires_in);
        let successor = self.new_row(
            &credential,
            row.family_id,
            &row.subject,
            refresh_token,
            id_token.map(|token| token.as_str()),
            Some((tokens.access_token.as_str(), expires_at)),
            now,
            &lifetimes,
        )?;
        transaction.insert(&successor).await?;
        transaction.mark_rotated(row.id, successor.id, now).await?;
        transaction.commit().await?;
        Ok(Issued {
            access_token: tokens.access_token,
            expires_at,
            subject: row.subject.clone(),
            cookie: Some((credential, lifetimes.max_age)),
        })
    }

    async fn replay(
        &self,
        mut transaction: Box<dyn SessionTransaction>,
        row: &SessionRow,
        successor: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Issued, SessionError> {
        let successor = transaction.lock(successor).await?;
        if let Some(successor) = successor
            && successor.revoked_at.is_none()
            && let (Some(sealed), Some(expires_at), Some(grace_until)) = (
                &successor.grace_access_token,
                successor.grace_expires_at,
                successor.grace_until,
            )
            && now <= grace_until
        {
            let access_token = self.open(&successor, GRACE_ACCESS_TOKEN_FIELD, sealed)?;
            return Ok(Issued {
                access_token,
                expires_at,
                subject: successor.subject,
                cookie: None,
            });
        }
        let head = transaction.family_head(row.family_id).await?;
        transaction.revoke_family(row.family_id, now).await?;
        transaction.commit().await?;
        tracing::warn!(family_id = %row.family_id, "rotated browser session reused; family revoked");
        if let Some(head) = head {
            self.revoke_at_idp(&head).await;
        }
        Err(SessionError::SessionExpired)
    }

    /// Deletes the presented session's family and returns the end-session URL.
    pub(crate) async fn logout(
        &self,
        credential: Option<&SessionCredential>,
    ) -> Result<Option<String>, SessionError> {
        let Some(credential) = credential else {
            return Ok(None);
        };
        let mut transaction = self.store.begin().await?;
        let row = match locked_match(transaction.as_mut(), credential).await {
            Ok(row) => row,
            Err(SessionError::SessionExpired) => return Ok(None),
            Err(error) => return Err(error),
        };
        let head = transaction.family_head(row.family_id).await?;
        transaction.delete_family(row.family_id).await?;
        transaction.commit().await?;
        let Some(head) = head.filter(|head| head.revoked_at.is_none()) else {
            return Ok(None);
        };
        self.revoke_at_idp(&head).await;
        let id_token = head
            .id_token
            .as_ref()
            .map(|sealed| self.open(&head, ID_TOKEN_FIELD, sealed))
            .transpose()?;
        Ok(self
            .idp
            .end_session_url(id_token.as_ref().map(|token| token.as_str())))
    }

    async fn revoke_at_idp(&self, row: &SessionRow) {
        let refresh_token = match self.open(row, REFRESH_TOKEN_FIELD, &row.refresh_token) {
            Ok(token) => token,
            Err(error) => {
                tracing::warn!(error = %error, "could not decrypt refresh token for revocation");
                return;
            }
        };
        if let Err(error) = self.idp.revoke(&refresh_token).await {
            tracing::warn!(error = %error, "IdP refresh token revocation failed");
        }
    }

    fn open(
        &self,
        row: &SessionRow,
        field: &'static str,
        sealed: &[u8],
    ) -> Result<Zeroizing<String>, CryptoError> {
        crypto::open(
            &self.settings.cipher,
            row.id,
            &row.subject,
            row.key_version,
            field,
            sealed,
        )
    }

    fn lifetimes(
        &self,
        now: DateTime<Utc>,
        absolute_expires_at: DateTime<Utc>,
        refresh_expires_in: Option<Duration>,
    ) -> Lifetimes {
        let mut idle = self.settings.idle_timeout;
        if let Some(refresh_lifetime) = refresh_expires_in {
            if refresh_lifetime < idle
                && !self.checked_refresh_lifetime.swap(true, Ordering::Relaxed)
            {
                tracing::warn!(
                    idle_timeout_seconds = idle.as_secs(),
                    refresh_expires_in_seconds = refresh_lifetime.as_secs(),
                    "browser session idle timeout exceeds the IdP refresh token lifetime"
                );
            }
            idle = idle.min(refresh_lifetime);
        }
        let remaining = (absolute_expires_at - now)
            .to_std()
            .unwrap_or(Duration::ZERO);
        let max_age = idle.min(remaining);
        Lifetimes {
            idle_expires_at: after(now, max_age),
            absolute_expires_at,
            max_age,
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one call site per grant builds a row from distinct token and lifetime values"
    )]
    fn new_row(
        &self,
        credential: &SessionCredential,
        family_id: Uuid,
        subject: &str,
        refresh_token: &str,
        id_token: Option<&str>,
        grace: Option<(&str, DateTime<Utc>)>,
        now: DateTime<Utc>,
        lifetimes: &Lifetimes,
    ) -> Result<SessionRow, CryptoError> {
        let mut fields = vec![(REFRESH_TOKEN_FIELD, refresh_token)];
        if let Some(id_token) = id_token {
            fields.push((ID_TOKEN_FIELD, id_token));
        }
        if let Some((access_token, _)) = grace {
            fields.push((GRACE_ACCESS_TOKEN_FIELD, access_token));
        }
        let mut sealed = crypto::seal(&self.settings.cipher, credential.id, subject, &fields)?;
        let refresh_token = sealed
            .fields
            .remove(REFRESH_TOKEN_FIELD)
            .ok_or(CryptoError::MissingField(REFRESH_TOKEN_FIELD))?;
        Ok(SessionRow {
            id: credential.id,
            secret_hash: credential.secret_hash(),
            family_id,
            subject: subject.to_owned(),
            key_version: sealed.key_version,
            refresh_token,
            id_token: sealed.fields.remove(ID_TOKEN_FIELD),
            created_at: now,
            last_used_at: now,
            idle_expires_at: lifetimes.idle_expires_at,
            absolute_expires_at: lifetimes.absolute_expires_at,
            revoked_at: None,
            rotated_to: None,
            grace_access_token: sealed.fields.remove(GRACE_ACCESS_TOKEN_FIELD),
            grace_expires_at: grace.map(|(_, expires_at)| expires_at),
            grace_until: grace.map(|_| after(now, GRACE_WINDOW)),
        })
    }
}

async fn locked_match(
    transaction: &mut dyn SessionTransaction,
    credential: &SessionCredential,
) -> Result<SessionRow, SessionError> {
    let row = transaction
        .lock(credential.id)
        .await?
        .ok_or(SessionError::SessionExpired)?;
    if !credential.matches(&row.secret_hash) || row.revoked_at.is_some() {
        return Err(SessionError::SessionExpired);
    }
    Ok(row)
}

/// Adds a duration, saturating at the latest representable instant.
fn after(instant: DateTime<Utc>, duration: Duration) -> DateTime<Utc> {
    TimeDelta::from_std(duration)
        .ok()
        .and_then(|delta| instant.checked_add_signed(delta))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}
