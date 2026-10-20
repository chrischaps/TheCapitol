use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

pub use capitol_protocol::rest::{AccountInfo, AccountKind};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Account {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    /// `subscription_status` (E4.1), under its old name
    pub tier: String,
    pub is_admin: bool,
    /// `human` or `citizen` (C0.1); [`AccountKind`] names them
    pub kind: String,
    pub created_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
}

impl From<Account> for AccountInfo {
    fn from(account: Account) -> Self {
        Self {
            id: account.id,
            email: account.email,
            tier: account.tier,
            is_admin: account.is_admin,
            kind: AccountKind::parse(&account.kind).unwrap_or_default(),
        }
    }
}
