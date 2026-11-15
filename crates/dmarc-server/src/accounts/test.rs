//! The Microsoft part of an account's connection test: the token step
//! ([`Accounts::test_token`]). `POST /api/accounts/test` runs it before
//! reading the mailbox over Graph.

use super::oauth::TokenRequest;
use super::Accounts;
use crate::view::accounts::StepResult;
use crate::wording::microsoft::TOKEN_ISSUED;

impl Accounts {
    /// A fresh token for `request` (not the cached one, which it replaces:
    /// a change in Entra shows at once). It names no permission (the
    /// mailbox access comes from Exchange role assignments) and none is
    /// checked. The test's token line, and whether the token was issued.
    pub async fn test_token(&self, request: &TokenRequest) -> (StepResult, bool) {
        match self.tokens.fresh_token(request).await {
            Ok(_) => (StepResult::passed(TOKEN_ISSUED), true),
            Err(e) => (
                StepResult::failed(e.message, Some(e.detail)).with_docs(e.docs),
                false,
            ),
        }
    }
}
