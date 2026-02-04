use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Account {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub tier: String,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AccountInfo {
    /// Unique account identifier
    pub id: Uuid,
    /// Email address
    pub email: String,
    /// Account tier (free, premium, etc.)
    pub tier: String,
    /// Whether this account has admin privileges (debug features)
    pub is_admin: bool,
}

impl From<Account> for AccountInfo {
    fn from(account: Account) -> Self {
        Self {
            id: account.id,
            email: account.email,
            tier: account.tier,
            is_admin: account.is_admin,
        }
    }
}
