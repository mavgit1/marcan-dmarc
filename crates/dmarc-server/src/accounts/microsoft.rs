//! A Microsoft login, stored and typed: what saving one checks and writes
//! ([`Accounts::prepare_microsoft_save`]), the token request of an unsaved
//! form for the connection test ([`Accounts::microsoft_test_request`]),
//! and of a stored account for its passes ([`graph_config`]) and sends,
//! both over Microsoft Graph with one token.
//!
//! A Microsoft login reads and sends as the account's address, which has
//! no username of its own (the save stores none), and has no servers (the
//! save stores none). It proves itself with a certificate Marcan generated,
//! which stays with the account whatever tenant and client ID it is used
//! for: its key never leaves Marcan, only assertions signed with it go to
//! Microsoft.

use super::oauth::{
    is_guid, ClientCertificate, GraphAccountToken, TokenRequest, TokenSource, PASS_REFRESH_BEFORE,
};
use super::{login_user, AccountError, Accounts, MicrosoftToken, SecretChange, SecretState};
use crate::wording::accounts::locked_sentence;
use crate::wording::microsoft::{graph_docs, graph_sentence};
use crate::wording::microsoft::{
    CLIENT_NOT_GUID, DRAFT_GONE, ENTER_CLIENT, ENTER_MAILBOX, ENTER_TENANT, GENERATE_CERTIFICATE,
    TENANT_NOT_GUID,
};
use dmarc_assess::LockReason;
use dmarc_mailbox::{GraphApi, GraphConfig};
use dmarc_store::{
    get_account, AccountFields, AccountRow, LoginKind, MicrosoftLogin, SecretKind, StoreError,
};
use std::sync::Arc;
use std::time::Duration;

/// A Microsoft login as the account form sends it (from
/// `SaveLoginRequest::Microsoft`; the address comes with the account).
#[derive(Debug, Clone)]
pub struct MicrosoftLoginRequest {
    /// The Directory (tenant) ID, a GUID.
    pub tenant_id: String,
    /// The Application (client) ID, a GUID.
    pub client_id: String,
    /// The certificate generated for this form (`POST
    /// /api/accounts/certificate`); `None`: keep the stored one.
    pub draft_id: Option<String>,
}

/// What saving a Microsoft login writes: its columns, its secrets (the
/// password cleared), and the draft whose certificate it stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrosoftSave {
    pub microsoft: MicrosoftLogin,
    pub secrets: Vec<(SecretKind, SecretChange)>,
    /// Take this draft ([`Drafts::take`](super::certificate::Drafts::take))
    /// once the account is written.
    pub draft_id: Option<String>,
}

/// The Graph configuration of a Microsoft login, reading `fields`' folder
/// of its mailbox at `api` with `request`'s Graph tokens from `tokens`
/// (epoch 0: a pass sets the account's).
pub fn graph_config(
    id: &str,
    fields: &AccountFields,
    api: &GraphApi,
    tokens: &Arc<TokenSource>,
    request: TokenRequest,
) -> GraphConfig {
    GraphConfig {
        id: id.to_owned(),
        epoch: 0,
        address: login_user(fields).to_owned(),
        folder: fields.folder.clone(),
        api: api.clone(),
        token: Arc::new(GraphAccountToken::new(Arc::clone(tokens), request)),
        say: graph_sentence,
        docs: graph_docs,
    }
}

/// A stored Microsoft login's token request, if it can be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredRequest {
    Ready(TokenRequest),
    /// Something is not set; the text says what.
    Missing(&'static str),
    Locked(LockReason),
}

fn refused(why: &str) -> AccountError {
    AccountError::Refused(why.to_owned())
}

/// The tenant and client ID of `request`, trimmed and checked: an empty one
/// is asked for, a malformed one gets its format.
fn ids(request: &MicrosoftLoginRequest) -> Result<(String, String), AccountError> {
    let tenant = request.tenant_id.trim();
    let client = request.client_id.trim();
    if tenant.is_empty() {
        return Err(refused(ENTER_TENANT));
    }
    if !is_guid(tenant) {
        return Err(refused(TENANT_NOT_GUID));
    }
    if client.is_empty() {
        return Err(refused(ENTER_CLIENT));
    }
    if !is_guid(client) {
        return Err(refused(CLIENT_NOT_GUID));
    }
    Ok((tenant.to_owned(), client.to_owned()))
}

/// The draft id `request` names, if it names one.
fn draft_of(request: &MicrosoftLoginRequest) -> Option<&str> {
    request.draft_id.as_deref().filter(|d| !d.is_empty())
}

impl Accounts {
    async fn saved(&self, id: Option<i64>) -> Result<Option<AccountRow>, StoreError> {
        match id {
            Some(id) => get_account(&self.pool, id).await,
            None => Ok(None),
        }
    }

    /// The stored secret of `kind` of `account`, or the sentence why it
    /// cannot be used.
    async fn stored_secret(
        &self,
        account: &AccountRow,
        kind: SecretKind,
        missing: &'static str,
    ) -> Result<String, AccountError> {
        match self.secret(account.id, kind).await? {
            SecretState::Readable(value) => Ok(value),
            SecretState::Missing => Err(refused(missing)),
            SecretState::Locked(reason) => Err(AccountError::Refused(locked_sentence(
                &account.fields.name,
                reason,
            ))),
        }
    }

    /// Check a Microsoft login to be saved as account `id` (`None`: a new
    /// account) with `fields` (its mailbox address), and say what to store:
    /// the error is the sentence to answer. A draft certificate is stored
    /// (take the draft once the account is written); none keeps the stored
    /// one, and without one there is nothing to log in with.
    pub async fn prepare_microsoft_save(
        &self,
        id: Option<i64>,
        fields: &AccountFields,
        request: &MicrosoftLoginRequest,
    ) -> Result<MicrosoftSave, AccountError> {
        let (tenant, client) = ids(request)?;
        if login_user(fields).is_empty() {
            return Err(refused(ENTER_MAILBOX));
        }
        let mut microsoft = MicrosoftLogin {
            tenant_id: Some(tenant),
            client_id: Some(client),
            ..MicrosoftLogin::default()
        };
        let (change, draft_id) = match draft_of(request) {
            Some(draft) => {
                let cert = self.drafts.get(draft).ok_or_else(|| refused(DRAFT_GONE))?;
                microsoft.cert_der = Some(cert.cert_der);
                microsoft.cert_thumbprint = Some(cert.thumbprint);
                microsoft.cert_expires_at = Some(cert.expires_at);
                (
                    SecretChange::Set(cert.private_key_pem),
                    Some(draft.to_owned()),
                )
            }
            None => {
                let saved = self.saved(id).await?;
                let kept = saved
                    .as_ref()
                    .filter(|s| s.fields.login_kind == LoginKind::Microsoft)
                    .map(|s| &s.fields.microsoft)
                    .filter(|ms| ms.cert_der.is_some());
                let key_stored = match id {
                    Some(id) => self.secret(id, SecretKind::PrivateKey).await?.is_stored(),
                    None => false,
                };
                match kept {
                    Some(ms) if key_stored => {
                        microsoft.cert_der.clone_from(&ms.cert_der);
                        microsoft.cert_thumbprint.clone_from(&ms.cert_thumbprint);
                        microsoft.cert_expires_at = ms.cert_expires_at;
                        (SecretChange::Keep, None)
                    }
                    _ => return Err(refused(GENERATE_CERTIFICATE)),
                }
            }
        };
        Ok(MicrosoftSave {
            microsoft,
            secrets: vec![
                (SecretKind::Password, SecretChange::Clear),
                (SecretKind::PrivateKey, change),
            ],
            draft_id,
        })
    }

    /// The token request of a Microsoft login typed in the form of account
    /// `id` (`None`: not saved yet), for the connection test: the draft
    /// certificate (kept for the save), else the stored one.
    pub async fn microsoft_test_request(
        &self,
        id: Option<i64>,
        request: &MicrosoftLoginRequest,
    ) -> Result<TokenRequest, AccountError> {
        let (tenant, client) = ids(request)?;
        let certificate = match draft_of(request) {
            Some(draft) => {
                let cert = self.drafts.get(draft).ok_or_else(|| refused(DRAFT_GONE))?;
                ClientCertificate {
                    private_key_pem: cert.private_key_pem,
                    cert_der: cert.cert_der,
                }
            }
            None => {
                let saved = self.saved(id).await?;
                let (account, cert_der) = saved
                    .as_ref()
                    .and_then(|a| Some((a, a.fields.microsoft.cert_der.clone()?)))
                    .ok_or_else(|| refused(GENERATE_CERTIFICATE))?;
                let private_key_pem = self
                    .stored_secret(account, SecretKind::PrivateKey, GENERATE_CERTIFICATE)
                    .await?;
                ClientCertificate {
                    private_key_pem,
                    cert_der,
                }
            }
        };
        Ok(TokenRequest {
            tenant_id: tenant,
            client_id: client,
            certificate,
        })
    }

    /// The token request of stored account `account`'s Microsoft login.
    pub async fn stored_request(&self, account: &AccountRow) -> Result<StoredRequest, StoreError> {
        let ms = &account.fields.microsoft;
        let (Some(tenant), Some(client)) = (ms.tenant_id.clone(), ms.client_id.clone()) else {
            return Ok(StoredRequest::Missing(ENTER_TENANT));
        };
        let Some(cert_der) = ms.cert_der.clone() else {
            return Ok(StoredRequest::Missing(GENERATE_CERTIFICATE));
        };
        let private_key_pem = match self.secret(account.id, SecretKind::PrivateKey).await? {
            SecretState::Readable(value) => value,
            SecretState::Missing => return Ok(StoredRequest::Missing(GENERATE_CERTIFICATE)),
            SecretState::Locked(reason) => return Ok(StoredRequest::Locked(reason)),
        };
        Ok(StoredRequest::Ready(TokenRequest {
            tenant_id: tenant,
            client_id: client,
            certificate: ClientCertificate {
                private_key_pem,
                cert_der,
            },
        }))
    }

    /// A Microsoft login's Graph configuration for the pass about to start,
    /// its token fetched already (a refusal fails the pass before it reads
    /// anything, as an [`AccountError::Token`]); `None` while the login is
    /// incomplete or locked (logged once per `poll`).
    pub(super) async fn microsoft_mailbox_config(
        &self,
        account: &AccountRow,
        poll: Duration,
    ) -> Result<Option<GraphConfig>, AccountError> {
        let request = match self.stored_request(account).await? {
            StoredRequest::Ready(request) => request,
            StoredRequest::Missing(_) => return Ok(None),
            StoredRequest::Locked(reason) => {
                self.log_locked_pass(account, reason, poll);
                return Ok(None);
            }
        };
        self.tokens
            .token_lasting(&request, PASS_REFRESH_BEFORE)
            .await
            .map_err(AccountError::Token)?;
        Ok(Some(GraphConfig {
            epoch: account.receive_epoch,
            ..graph_config(
                &account.id.to_string(),
                &account.fields,
                &self.graph,
                &self.tokens,
                request,
            )
        }))
    }

    /// Forget the cached tokens of `account`'s Microsoft login (before it is
    /// saved or deleted, or after Microsoft refused one): its certificate's,
    /// or every one of its app registration when the key cannot be read. Nothing for a password login.
    pub async fn forget_token(&self, account: &AccountRow) {
        let ms = &account.fields.microsoft;
        if account.fields.login_kind != LoginKind::Microsoft {
            return;
        }
        match self.stored_request(account).await {
            Ok(StoredRequest::Ready(request)) => self.tokens.forget(&request),
            _ => {
                if let (Some(tenant), Some(client)) = (&ms.tenant_id, &ms.client_id) {
                    self.tokens.forget_login(tenant, client);
                }
            }
        }
    }

    /// [`forget_token`](Self::forget_token) of account `id`, if it exists.
    pub async fn forget_token_of(&self, id: i64) {
        if let Ok(Some(account)) = get_account(&self.pool, id).await {
            self.forget_token(&account).await;
        }
    }

    /// A Microsoft login's Graph token for sending: the one its passes
    /// read with, cached, fetched when needed.
    pub(super) async fn microsoft_send_token(
        &self,
        account: &AccountRow,
    ) -> Result<MicrosoftToken, StoreError> {
        Ok(match self.stored_request(account).await? {
            StoredRequest::Ready(request) => MicrosoftToken::Ready(Arc::new(
                GraphAccountToken::new(self.tokens.clone(), request),
            )),
            StoredRequest::Missing(why) => MicrosoftToken::Missing(why),
            StoredRequest::Locked(reason) => MicrosoftToken::Locked(reason),
        })
    }
}
