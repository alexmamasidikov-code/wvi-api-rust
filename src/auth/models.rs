use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Privy access token claims (ES256 JWT)
#[derive(Debug, Serialize, Deserialize)]
pub struct PrivyClaims {
    pub sub: String,      // User's Privy DID (did:privy:...)
    pub sid: String,      // Session ID
    pub iss: String,      // Issuer ("privy.io")
    pub aud: String,      // Your Privy App ID
    pub iat: u64,         // Issued at (Unix timestamp)
    pub exp: u64,         // Expiration (Unix timestamp)
}

/// Privy token verification response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivyTokenResult {
    pub user_id: String,         // did:privy:...
    #[serde(default)]
    pub session_id: String,
}

/// Privy user from /api/v1/users/{did}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivyUser {
    pub id: String,
    #[serde(default)]
    pub created_at: Option<u64>,
    #[serde(default)]
    pub linked_accounts: Vec<PrivyLinkedAccount>,
    #[serde(default)]
    pub has_accepted_terms: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PrivyLinkedAccount {
    #[serde(rename = "email")]
    Email { address: String },
    #[serde(rename = "phone")]
    Phone { number: String },
    #[serde(rename = "wallet")]
    Wallet {
        address: String,
        #[serde(default)]
        chain_type: String,
    },
    #[serde(rename = "google_oauth")]
    Google { email: String },
    #[serde(rename = "apple_oauth")]
    Apple { email: String },
    #[serde(rename = "twitter_oauth")]
    Twitter { username: String },
    #[serde(other)]
    Unknown,
}

impl PrivyUser {
    pub fn email(&self) -> Option<String> {
        for acc in &self.linked_accounts {
            match acc {
                PrivyLinkedAccount::Email { address } => return Some(address.clone()),
                PrivyLinkedAccount::Google { email } => return Some(email.clone()),
                PrivyLinkedAccount::Apple { email } => return Some(email.clone()),
                _ => {}
            }
        }
        None
    }

    pub fn wallet_address(&self) -> Option<String> {
        for acc in &self.linked_accounts {
            if let PrivyLinkedAccount::Wallet { address, .. } = acc {
                return Some(address.clone());
            }
        }
        None
    }
}

/// Request to verify a Privy access token
#[derive(Debug, Deserialize)]
pub struct VerifyTokenRequest {
    pub token: String,
}

/// Request to link a crypto wallet
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkWalletRequest {
    pub wallet_address: String,
    pub chain_type: Option<String>,
}

/// Auth response returned to client
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUserResponse {
    pub user_id: String,
    pub privy_did: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub wallet_address: Option<String>,
    pub linked_accounts: serde_json::Value,
    pub created_at: DateTime<Utc>,
}
