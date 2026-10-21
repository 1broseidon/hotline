//! What the room knows of its agent accounts while it runs: each account's
//! last status and its sign-in attempt, in memory only.

use super::{Room, now_ms};
use crate::accounts::login::AccountLogins;
use crate::accounts::resolve;
use crate::accounts::status::AccountStatuses;
use crate::contract::{AccountStatusView, LoginAttemptView};
use crate::driver::acp::StartBounds;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(super) struct Accounts {
    statuses: Arc<AccountStatuses>,
    logins: Arc<AccountLogins>,
}

impl Accounts {
    pub(super) fn new(bounds: StartBounds) -> Self {
        let statuses = Arc::new(AccountStatuses::new(bounds));
        Self {
            logins: Arc::new(AccountLogins::new(statuses.clone())),
            statuses,
        }
    }
}

impl Room {
    pub(crate) fn account_statuses(&self) -> &Arc<AccountStatuses> {
        &self.accounts.statuses
    }

    pub(crate) fn account_logins(&self) -> &Arc<AccountLogins> {
        &self.accounts.logins
    }

    pub(crate) fn account_live(
        &self,
        account_id: &str,
    ) -> (AccountStatusView, Option<LoginAttemptView>) {
        (
            self.account_statuses().cached(account_id),
            self.account_logins().view(account_id),
        )
    }

    pub(crate) fn accounts_refresh_stale(&self, account_ids: &[String]) {
        let root = self.log.root();
        for account_id in account_ids {
            if self.account_statuses().stale(account_id, now_ms())
                && let Ok(account) = resolve::account(root, &self.log, account_id)
            {
                self.account_statuses().refresh(root.to_path_buf(), account);
            }
        }
    }

    pub(crate) async fn accounts_status(
        &self,
        account_id: &str,
    ) -> Result<AccountStatusView, String> {
        let account = crate::accounts::resolved(&self.log, account_id)?;
        Ok(self
            .account_statuses()
            .check(self.log.root(), &account)
            .await)
    }

    pub(crate) async fn accounts_login_start(
        self: &Arc<Self>,
        account_id: &str,
        email: Option<String>,
        owner: CancellationToken,
        seat_is_desk: bool,
    ) -> Result<String, String> {
        let account = crate::accounts::resolved(&self.log, account_id)?;
        self.account_logins()
            .start(self, &account, email, owner, seat_is_desk)
            .await
    }
}
