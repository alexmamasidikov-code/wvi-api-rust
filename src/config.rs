use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    // Privy Auth
    pub privy_app_id: String,
    pub privy_app_secret: String,
    // AI (custom model — Nematron/Qwen, configured later)
    pub ai_api_url: String,
    pub ai_api_key: String,
    pub ai_model: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://wvi:wvi@localhost:5432/wvi".into()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8091".into())
                .parse()
                .unwrap_or(8091),
            privy_app_id: env::var("PRIVY_APP_ID").unwrap_or_default(),
            privy_app_secret: env::var("PRIVY_APP_SECRET").unwrap_or_default(),
            ai_api_url: env::var("AI_API_URL").unwrap_or_default(),
            ai_api_key: env::var("AI_API_KEY").unwrap_or_default(),
            ai_model: env::var("AI_MODEL").unwrap_or_else(|_| "nematron".into()),
        }
    }

    pub fn privy_configured(&self) -> bool {
        !self.privy_app_id.is_empty() && !self.privy_app_secret.is_empty()
    }
}
