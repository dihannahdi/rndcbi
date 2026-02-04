use std::env;

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub server: ServerSettings,
    pub database: DatabaseSettings,
    pub jwt: JwtSettings,
    pub openai: OpenAISettings,
    pub storage: StorageSettings,
    pub security: SecuritySettings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
    pub workers: Option<usize>,
    pub max_connections: u32,
    pub request_timeout_secs: u64,
    pub cors_allowed_origins: Vec<String>,
    pub public_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u64,
    pub idle_timeout_secs: u64,
}

impl DatabaseSettings {
    pub fn connection_string(&self) -> String {
        format!(
            "postgres://{}:{}@{}:{}/{}",
            self.username, self.password, self.host, self.port, self.database
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct JwtSettings {
    pub secret: String,
    pub expiry_hours: i64,
    pub refresh_expiry_days: i64,
    pub issuer: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAISettings {
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageSettings {
    pub upload_path: String,
    pub reports_path: String,
    pub max_file_size_mb: u64,
    pub allowed_extensions: Vec<String>,
    pub public_url_prefix: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecuritySettings {
    pub password_min_length: usize,
    pub max_login_attempts: u32,
    pub lockout_duration_minutes: u32,
    pub bcrypt_cost: u32,
    pub rate_limit_requests: u32,
    pub rate_limit_window: u64,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let run_mode = env::var("RUN_MODE").unwrap_or_else(|_| "development".into());

        let s = Config::builder()
            // Start with default configuration
            .add_source(File::with_name("config/default"))
            // Add environment-specific config
            .add_source(File::with_name(&format!("config/{}", run_mode)).required(false))
            // Add local overrides
            .add_source(File::with_name("config/local").required(false))
            // Add environment variables (with prefix CENTRABIO_)
            .add_source(
                Environment::with_prefix("CENTRABIO")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;

        s.try_deserialize()
    }

    /// Load settings from environment variables directly (simpler for production)
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self::default().with_env_overrides())
    }
}

impl Settings {
    /// Apply environment variable overrides to default settings
    fn with_env_overrides(mut self) -> Self {
        // Server
        if let Ok(host) = env::var("SERVER_HOST") { self.server.host = host; }
        if let Ok(port) = env::var("SERVER_PORT") { self.server.port = port.parse().unwrap_or(8082); }
        if let Ok(url) = env::var("PUBLIC_URL") { self.server.public_url = url; }
        
        // Database
        if let Ok(host) = env::var("DATABASE_HOST") { self.database.host = host; }
        if let Ok(port) = env::var("DATABASE_PORT") { self.database.port = port.parse().unwrap_or(5433); }
        if let Ok(user) = env::var("DATABASE_USERNAME") { self.database.username = user; }
        if let Ok(pass) = env::var("DATABASE_PASSWORD") { self.database.password = pass; }
        if let Ok(db) = env::var("DATABASE_NAME") { self.database.database = db; }
        if let Ok(url) = env::var("DATABASE_URL") {
            // Parse DATABASE_URL if provided
            if let Some(parsed) = Self::parse_database_url(&url) {
                self.database = parsed;
            }
        }
        
        // JWT
        if let Ok(secret) = env::var("JWT_SECRET") { self.jwt.secret = secret; }
        if let Ok(hours) = env::var("JWT_EXPIRY_HOURS") { self.jwt.expiry_hours = hours.parse().unwrap_or(24); }
        
        // OpenAI
        if let Ok(key) = env::var("OPENAI_API_KEY") { self.openai.api_key = key; }
        if let Ok(model) = env::var("OPENAI_MODEL") { self.openai.model = model; }
        
        // Storage
        if let Ok(path) = env::var("UPLOAD_PATH") { self.storage.upload_path = path; }
        if let Ok(path) = env::var("REPORTS_PATH") { self.storage.reports_path = path; }
        
        self
    }

    fn parse_database_url(url: &str) -> Option<DatabaseSettings> {
        // postgres://user:pass@host:port/database
        let url = url.strip_prefix("postgres://")?;
        let (auth, rest) = url.split_once('@')?;
        let (username, password) = auth.split_once(':')?;
        let (host_port, database) = rest.split_once('/')?;
        let (host, port) = host_port.split_once(':').unwrap_or((host_port, "5432"));
        
        Some(DatabaseSettings {
            host: host.to_string(),
            port: port.parse().unwrap_or(5432),
            username: username.to_string(),
            password: password.to_string(),
            database: database.to_string(),
            max_connections: 20,
            min_connections: 5,
            connect_timeout_secs: 30,
            idle_timeout_secs: 600,
        })
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: ServerSettings {
                host: "127.0.0.1".to_string(),
                port: 8082,
                workers: None,
                max_connections: 1000,
                request_timeout_secs: 60,
                cors_allowed_origins: vec!["*".to_string()],
                public_url: "http://localhost:8082".to_string(),
            },
            database: DatabaseSettings {
                host: "localhost".to_string(),
                port: 5433,
                username: "centrabio".to_string(),
                password: "centrabio_secure_password".to_string(),
                database: "centrabio_nexus".to_string(),
                max_connections: 20,
                min_connections: 5,
                connect_timeout_secs: 30,
                idle_timeout_secs: 600,
            },
            jwt: JwtSettings {
                secret: "your-super-secret-jwt-key-change-in-production".to_string(),
                expiry_hours: 24,
                refresh_expiry_days: 7,
                issuer: "centrabio-nexus".to_string(),
            },
            openai: OpenAISettings {
                api_key: "".to_string(),
                model: "gpt-4o".to_string(),
                max_tokens: 4096,
                temperature: 0.7,
            },
            storage: StorageSettings {
                upload_path: "./uploads".to_string(),
                reports_path: "./reports".to_string(),
                max_file_size_mb: 50,
                allowed_extensions: vec![
                    "jpg".to_string(), "jpeg".to_string(), "png".to_string(),
                    "gif".to_string(), "pdf".to_string(), "doc".to_string(),
                    "docx".to_string(), "xls".to_string(), "xlsx".to_string(),
                ],
                public_url_prefix: "/uploads".to_string(),
            },
            security: SecuritySettings {
                password_min_length: 8,
                max_login_attempts: 5,
                lockout_duration_minutes: 30,
                bcrypt_cost: 12,
                rate_limit_requests: 100,
                rate_limit_window: 60,
            },
        }
    }
}
