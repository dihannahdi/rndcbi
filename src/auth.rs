use crate::config::Settings;
use crate::errors::AppError;
use crate::models::{User, UserRole};
use actix_web::{dev::ServiceRequest, http::header::AUTHORIZATION, HttpMessage};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, TokenData, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ==============================================================================
// JWT Claims
// ==============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,       // User ID
    pub email: String,
    pub role: UserRole,
    pub org_id: Option<Uuid>,
    pub iat: i64,          // Issued at
    pub exp: i64,          // Expiration
    pub token_type: TokenType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TokenType {
    Access,
    Refresh,
}

impl Claims {
    pub fn new_access(user: &User, settings: &Settings) -> Self {
        let now = Utc::now();
        Self {
            sub: user.id.to_string(),
            email: user.email.clone(),
            role: user.role.clone(),
            org_id: user.organization_id,
            iat: now.timestamp(),
            exp: (now + Duration::hours(settings.jwt.expiry_hours)).timestamp(),
            token_type: TokenType::Access,
        }
    }

    pub fn new_refresh(user: &User, settings: &Settings) -> Self {
        let now = Utc::now();
        Self {
            sub: user.id.to_string(),
            email: user.email.clone(),
            role: user.role.clone(),
            org_id: user.organization_id,
            iat: now.timestamp(),
            exp: (now + Duration::days(settings.jwt.refresh_expiry_days)).timestamp(),
            token_type: TokenType::Refresh,
        }
    }

    pub fn user_id(&self) -> Result<Uuid, AppError> {
        Uuid::parse_str(&self.sub).map_err(|_| AppError::Authentication("Invalid user ID in token".to_string()))
    }
}

// ==============================================================================
// JWT Service
// ==============================================================================

pub struct JwtService {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl JwtService {
    pub fn new(secret: &str) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
        }
    }

    pub fn generate_access_token(&self, user: &User, settings: &Settings) -> Result<String, AppError> {
        let claims = Claims::new_access(user, settings);
        encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| AppError::InternalError(format!("Failed to generate access token: {}", e)))
    }

    pub fn generate_refresh_token(&self, user: &User, settings: &Settings) -> Result<String, AppError> {
        let claims = Claims::new_refresh(user, settings);
        encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| AppError::InternalError(format!("Failed to generate refresh token: {}", e)))
    }

    pub fn validate_token(&self, token: &str) -> Result<TokenData<Claims>, AppError> {
        let mut validation = Validation::default();
        validation.set_required_spec_claims(&["exp", "sub"]);

        decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                    AppError::Authentication("Token has expired".to_string())
                }
                jsonwebtoken::errors::ErrorKind::InvalidToken => {
                    AppError::Authentication("Invalid token".to_string())
                }
                _ => AppError::Authentication(format!("Token validation failed: {}", e)),
            })
    }

    pub fn extract_token(req: &ServiceRequest) -> Option<String> {
        req.headers()
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| {
                if h.starts_with("Bearer ") {
                    Some(h[7..].to_string())
                } else {
                    None
                }
            })
    }
}

// ==============================================================================
// Password Hashing
// ==============================================================================

pub struct PasswordService;

impl PasswordService {
    pub fn hash_password(password: &str) -> Result<String, AppError> {
        use argon2::{
            password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
            Argon2,
        };

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| AppError::InternalError(format!("Password hashing failed: {}", e)))
    }

    pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
        use argon2::{
            password_hash::{PasswordHash, PasswordVerifier},
            Argon2,
        };

        let parsed_hash = PasswordHash::new(hash)
            .map_err(|e| AppError::InternalError(format!("Invalid password hash format: {}", e)))?;

        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }
}

// ==============================================================================
// Middleware for Authentication
// ==============================================================================

use actix_web::Error as ActixError;
use actix_web_httpauth::extractors::bearer::BearerAuth;

pub type AuthenticatedUser = Claims;

pub async fn validator(
    req: ServiceRequest,
    credentials: BearerAuth,
) -> Result<ServiceRequest, (ActixError, ServiceRequest)> {
    let jwt_service = req
        .app_data::<web::Data<Arc<JwtService>>>()
        .expect("JwtService not configured");

    match jwt_service.validate_token(credentials.token()) {
        Ok(token_data) => {
            if token_data.claims.token_type != TokenType::Access {
                return Err((AppError::Authentication("Invalid token type".to_string()).into(), req));
            }
            req.extensions_mut().insert(token_data.claims);
            Ok(req)
        }
        Err(e) => Err((e.into(), req)),
    }
}

// ==============================================================================
// Authorization Helpers
// ==============================================================================

pub struct Authorization;

impl Authorization {
    pub fn require_roles(user: &Claims, allowed_roles: &[UserRole]) -> Result<(), AppError> {
        if allowed_roles.contains(&user.role) {
            Ok(())
        } else {
            Err(AppError::Authorization(format!(
                "Role {:?} is not authorized for this action",
                user.role
            )))
        }
    }

    pub fn require_project_member(
        user: &Claims,
        _project_id: Uuid,
        _pool: &sqlx::PgPool,
    ) -> Result<(), AppError> {
        // System admin and R&D manager can access all projects
        if matches!(user.role, UserRole::SystemAdmin | UserRole::RdManager) {
            return Ok(());
        }
        // TODO: Check project_team_members table for membership
        Ok(())
    }

    pub fn can_approve_qc(user: &Claims) -> Result<(), AppError> {
        if user.role.can_approve_qc() {
            Ok(())
        } else {
            Err(AppError::Authorization(
                "Only QC Analysts, R&D Managers, and System Admins can approve QC".to_string(),
            ))
        }
    }

    pub fn can_lock_project(user: &Claims) -> Result<(), AppError> {
        if user.role.can_lock_project() {
            Ok(())
        } else {
            Err(AppError::Authorization(
                "Only R&D Managers and System Admins can lock projects".to_string(),
            ))
        }
    }

    pub fn can_create_project(user: &Claims) -> Result<(), AppError> {
        if user.role.can_create_project() {
            Ok(())
        } else {
            Err(AppError::Authorization(
                "Only Principal Researchers, R&D Managers, and System Admins can create projects".to_string(),
            ))
        }
    }
}

// ==============================================================================
// Session Management
// ==============================================================================

use sqlx::PgPool;

pub struct SessionService;

impl SessionService {
    pub async fn create_session(
        pool: &PgPool,
        user_id: Uuid,
        refresh_token: &str,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<Uuid, AppError> {
        let session_id = Uuid::new_v4();
        let token_hash = Self::hash_token(refresh_token);

        sqlx::query(
            r#"
            INSERT INTO user_sessions (id, user_id, token_hash, ip_address, user_agent, expires_at)
            VALUES ($1, $2, $3, $4::inet, $5, $6)
            "#
        )
        .bind(session_id)
        .bind(user_id)
        .bind(&token_hash)
        .bind(ip_address)
        .bind(user_agent)
        .bind(expires_at)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(session_id)
    }

    pub async fn validate_session(
        pool: &PgPool,
        user_id: Uuid,
        refresh_token: &str,
    ) -> Result<bool, AppError> {
        let token_hash = Self::hash_token(refresh_token);

        let result: (bool,) = sqlx::query_as(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM user_sessions 
                WHERE user_id = $1 
                AND token_hash = $2 
                AND is_revoked = false 
                AND expires_at > NOW()
            )
            "#
        )
        .bind(user_id)
        .bind(&token_hash)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(result.0)
    }

    pub async fn revoke_session(
        pool: &PgPool,
        user_id: Uuid,
        refresh_token: &str,
    ) -> Result<(), AppError> {
        let token_hash = Self::hash_token(refresh_token);

        sqlx::query(
            r#"
            UPDATE user_sessions 
            SET is_revoked = true, revoked_at = NOW() 
            WHERE user_id = $1 AND token_hash = $2
            "#
        )
        .bind(user_id)
        .bind(&token_hash)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    pub async fn revoke_all_user_sessions(pool: &PgPool, user_id: Uuid) -> Result<u64, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE user_sessions 
            SET is_revoked = true, revoked_at = NOW() 
            WHERE user_id = $1 AND is_revoked = false
            "#
        )
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(result.rows_affected())
    }

    pub async fn cleanup_expired_sessions(pool: &PgPool) -> Result<u64, AppError> {
        let result = sqlx::query(
            r#"
            DELETE FROM user_sessions 
            WHERE expires_at < NOW() OR is_revoked = true
            "#
        )
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(result.rows_affected())
    }

    fn hash_token(token: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

// ==============================================================================
// Rate Limiting
// ==============================================================================

use std::collections::HashMap;
use std::time::{Duration as StdDuration, Instant};
use tokio::sync::RwLock;

pub struct RateLimiter {
    requests: RwLock<HashMap<String, Vec<Instant>>>,
    max_requests: usize,
    window: StdDuration,
}

impl RateLimiter {
    pub fn new(max_requests: usize, window_seconds: u64) -> Self {
        Self {
            requests: RwLock::new(HashMap::new()),
            max_requests,
            window: StdDuration::from_secs(window_seconds),
        }
    }

    pub async fn check(&self, key: &str) -> Result<(), AppError> {
        let mut requests = self.requests.write().await;
        let now = Instant::now();

        let timestamps = requests.entry(key.to_string()).or_insert_with(Vec::new);

        // Remove expired timestamps
        timestamps.retain(|t| now.duration_since(*t) < self.window);

        if timestamps.len() >= self.max_requests {
            return Err(AppError::RateLimitError);
        }

        timestamps.push(now);
        Ok(())
    }

    pub async fn cleanup(&self) {
        let mut requests = self.requests.write().await;
        let now = Instant::now();

        requests.retain(|_, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < self.window);
            !timestamps.is_empty()
        });
    }
}

// ==============================================================================
// Account Lockout
// ==============================================================================

pub struct AccountLockout;

impl AccountLockout {
    const MAX_FAILED_ATTEMPTS: i32 = 5;
    const LOCKOUT_DURATION_MINUTES: i64 = 30;

    pub async fn record_failed_attempt(pool: &PgPool, user_id: Uuid) -> Result<(), AppError> {
        sqlx::query(
            r#"
            UPDATE users 
            SET failed_login_attempts = failed_login_attempts + 1,
                locked_until = CASE 
                    WHEN failed_login_attempts + 1 >= $2 
                    THEN NOW() + INTERVAL '30 minutes'
                    ELSE locked_until
                END
            WHERE id = $1
            "#
        )
        .bind(user_id)
        .bind(Self::MAX_FAILED_ATTEMPTS)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    pub async fn reset_attempts(pool: &PgPool, user_id: Uuid) -> Result<(), AppError> {
        sqlx::query(
            r#"
            UPDATE users 
            SET failed_login_attempts = 0, locked_until = NULL
            WHERE id = $1
            "#
        )
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    pub fn is_locked(user: &User) -> bool {
        if let Some(locked_until) = user.locked_until {
            return locked_until > Utc::now();
        }
        false
    }
}
