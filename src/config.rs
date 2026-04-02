use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_expiry_hours: i64,
    pub port: u16,
    pub claude_api_key: String,
    pub claude_model: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://wvi:wvi@localhost:5432/wvi".into()),
            jwt_secret: env::var("JWT_SECRET")
                .unwrap_or_else(|_| "wvi-super-secret-key-change-in-production".into()),
            jwt_expiry_hours: env::var("JWT_EXPIRY_HOURS")
                .unwrap_or_else(|_| "24".into())
                .parse()
                .unwrap_or(24),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8091".into())
                .parse()
                .unwrap_or(8091),
            claude_api_key: env::var("CLAUDE_API_KEY").unwrap_or_default(),
            claude_model: env::var("CLAUDE_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-6".into()),
        }
    }
}
