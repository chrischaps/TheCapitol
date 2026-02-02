use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Account {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub tier: String,
    pub created_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct AccountInfo {
    pub id: Uuid,
    pub email: String,
    pub tier: String,
}

impl From<Account> for AccountInfo {
    fn from(account: Account) -> Self {
        Self {
            id: account.id,
            email: account.email,
            tier: account.tier,
        }
    }
}
