use crate::auth::{AuthenticatedUser, Authorization, JwtService, PasswordService, SessionService};
use crate::config::Settings;
use crate::errors::AppError;
use crate::models::*;
use crate::services::*;
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use chrono::{Duration, Utc};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
use validator::Validate;

// ==============================================================================
// HEALTH CHECK
// ==============================================================================

pub async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "timestamp": Utc::now().to_rfc3339(),
        "service": "CENTRABIO R&D NEXUS",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

pub async fn readiness_check(pool: web::Data<PgPool>) -> HttpResponse {
    match sqlx::query("SELECT 1").execute(pool.get_ref()).await {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({
            "status": "ready",
            "database": "connected",
            "timestamp": Utc::now().to_rfc3339()
        })),
        Err(e) => HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "not_ready",
            "database": "disconnected",
            "error": e.to_string(),
            "timestamp": Utc::now().to_rfc3339()
        })),
    }
}

// ==============================================================================
// AUTH HANDLERS
// ==============================================================================

pub async fn register(
    pool: web::Data<PgPool>,
    body: web::Json<CreateUserRequest>,
) -> Result<HttpResponse, AppError> {
    let user = UserService::create_user(pool.get_ref(), body.into_inner(), None).await?;
    
    Ok(HttpResponse::Created().json(ApiResponse::success_with_message(
        UserResponse::from(user),
        "User registered successfully",
    )))
}

pub async fn login(
    pool: web::Data<PgPool>,
    jwt_service: web::Data<Arc<JwtService>>,
    settings: web::Data<Settings>,
    body: web::Json<LoginRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    body.validate().map_err(|e| AppError::Validation(e.to_string()))?;

    let user = UserService::authenticate(pool.get_ref(), &body.email, &body.password).await?;

    let access_token = jwt_service.generate_access_token(&user, &settings)?;
    let refresh_token = jwt_service.generate_refresh_token(&user, &settings)?;

    // Extract client info
    let ip_address = req
        .connection_info()
        .realip_remote_addr()
        .map(|s| s.to_string());
    let user_agent = req
        .headers()
        .get("User-Agent")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    // Create session
    let expires_at = Utc::now() + Duration::days(settings.jwt.refresh_expiry_days);
    SessionService::create_session(
        pool.get_ref(),
        user.id,
        &refresh_token,
        ip_address.as_deref(),
        user_agent.as_deref(),
        expires_at,
    )
    .await?;

    // Log audit
    AuditService::log(
        pool.get_ref(),
        Some(user.id),
        "LOGIN",
        "user",
        Some(user.id),
        None,
        None,
        ip_address.as_deref(),
        user_agent.as_deref(),
    )
    .await?;

    Ok(HttpResponse::Ok().json(LoginResponse {
        access_token,
        refresh_token,
        token_type: "Bearer".to_string(),
        expires_in: settings.jwt.expiry_hours * 3600,
        user: UserResponse::from(user),
    }))
}

#[derive(Debug, serde::Deserialize)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

pub async fn refresh_token(
    pool: web::Data<PgPool>,
    jwt_service: web::Data<Arc<JwtService>>,
    settings: web::Data<Settings>,
    body: web::Json<RefreshTokenRequest>,
) -> Result<HttpResponse, AppError> {
    let token_data = jwt_service.validate_token(&body.refresh_token)?;
    let claims = token_data.claims;

    if claims.token_type != crate::auth::TokenType::Refresh {
        return Err(AppError::Authentication("Invalid token type".to_string()));
    }

    let user_id = claims.user_id()?;

    // Validate session
    let is_valid = SessionService::validate_session(pool.get_ref(), user_id, &body.refresh_token).await?;
    if !is_valid {
        return Err(AppError::Authentication("Session expired or revoked".to_string()));
    }

    // Get fresh user data
    let user = UserService::get_by_id(pool.get_ref(), user_id).await?;

    // Generate new tokens
    let new_access_token = jwt_service.generate_access_token(&user, &settings)?;
    let new_refresh_token = jwt_service.generate_refresh_token(&user, &settings)?;

    // Update session
    SessionService::revoke_session(pool.get_ref(), user_id, &body.refresh_token).await?;
    let expires_at = Utc::now() + Duration::days(settings.jwt.refresh_expiry_days);
    SessionService::create_session(pool.get_ref(), user_id, &new_refresh_token, None, None, expires_at).await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "access_token": new_access_token,
        "refresh_token": new_refresh_token,
        "token_type": "Bearer",
        "expires_in": settings.jwt.expiry_hours * 3600
    })))
}

pub async fn logout(
    pool: web::Data<PgPool>,
    body: web::Json<RefreshTokenRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let user_id = user.user_id()?;

    // Revoke the session
    SessionService::revoke_session(pool.get_ref(), user_id, &body.refresh_token).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::<()>::success_message("Logged out successfully")))
}

pub async fn logout_all(
    pool: web::Data<PgPool>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let user_id = user.user_id()?;

    // Revoke all sessions
    let count = SessionService::revoke_all_user_sessions(pool.get_ref(), user_id).await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": format!("{} sessions revoked", count)
    })))
}

pub async fn get_me(req: HttpRequest, pool: web::Data<PgPool>) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let full_user = UserService::get_by_id(pool.get_ref(), user.user_id()?).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(UserResponse::from(full_user))))
}

pub async fn update_profile(
    pool: web::Data<PgPool>,
    body: web::Json<UpdateProfileRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let updated = UserService::update_profile(pool.get_ref(), user.user_id()?, body.into_inner()).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success_with_message(
        UserResponse::from(updated),
        "Profile updated successfully",
    )))
}

#[derive(Debug, serde::Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    pool: web::Data<PgPool>,
    body: web::Json<ChangePasswordRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    UserService::change_password(
        pool.get_ref(),
        user.user_id()?,
        &body.current_password,
        &body.new_password,
    )
    .await?;

    Ok(HttpResponse::Ok().json(ApiResponse::<()>::success_message("Password changed successfully")))
}

// ==============================================================================
// USER MANAGEMENT
// ==============================================================================

pub async fn list_users(
    pool: web::Data<PgPool>,
    query: web::Query<PaginationParams>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    Authorization::require_roles(&user, &[UserRole::SystemAdmin, UserRole::RdManager])?;

    let users = UserService::list(
        pool.get_ref(),
        query.page.unwrap_or(1) as i64,
        query.per_page.unwrap_or(20) as i64,
        None,
    )
    .await?;

    Ok(HttpResponse::Ok().json(users))
}

// ==============================================================================
// PROJECT HANDLERS
// ==============================================================================

pub async fn create_project(
    pool: web::Data<PgPool>,
    body: web::Json<CreateProjectRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    Authorization::can_create_project(&user)?;

    let org_id = user.org_id.ok_or_else(|| AppError::Validation("User has no organization".to_string()))?;
    let project = ProjectService::create(pool.get_ref(), body.into_inner(), user.user_id()?, org_id).await?;

    // Audit log
    AuditService::log(
        pool.get_ref(),
        Some(user.user_id()?),
        "CREATE_PROJECT",
        "project",
        Some(project.id),
        None,
        Some(serde_json::json!({"title": &project.title})),
        None,
        None,
    )
    .await?;

    Ok(HttpResponse::Created().json(ApiResponse::success_with_message(
        project,
        "Project created successfully",
    )))
}

pub async fn list_projects(
    pool: web::Data<PgPool>,
    query: web::Query<PaginationParams>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let _user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let projects = ProjectService::list(
        pool.get_ref(),
        query.page.unwrap_or(1) as i64,
        query.per_page.unwrap_or(20) as i64,
        None,
    )
    .await?;

    Ok(HttpResponse::Ok().json(projects))
}

pub async fn get_project(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let _user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let project = ProjectService::get_by_id(pool.get_ref(), path.into_inner()).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(project)))
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateProjectStatusRequest {
    pub status: String,
}

pub async fn update_project_status(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    body: web::Json<UpdateProjectStatusRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let project_id = path.into_inner();
    let project = ProjectService::update_status(pool.get_ref(), project_id, &body.status, user.user_id()?).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success_with_message(
        project,
        "Project status updated",
    )))
}

#[derive(Debug, serde::Deserialize)]
pub struct LockProjectRequest {
    pub reason: Option<String>,
}

pub async fn lock_project(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    body: web::Json<LockProjectRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    Authorization::can_lock_project(&user)?;

    let project_id = path.into_inner();
    let project = ProjectService::lock_project(
        pool.get_ref(),
        project_id,
        user.user_id()?,
        body.reason.as_deref(),
    )
    .await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success_with_message(
        project,
        "Project locked successfully",
    )))
}

// ==============================================================================
// FORMULA HANDLERS
// ==============================================================================

pub async fn create_formula(
    pool: web::Data<PgPool>,
    body: web::Json<CreateFormulaRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let formula = FormulaService::create(pool.get_ref(), body.into_inner(), user.user_id()?).await?;

    Ok(HttpResponse::Created().json(ApiResponse::success_with_message(
        formula,
        "Formula created successfully",
    )))
}

pub async fn get_formula(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let _user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let formula = FormulaService::get_by_id(pool.get_ref(), path.into_inner()).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(formula)))
}

pub async fn submit_formula_for_qc(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let formula_id = path.into_inner();
    let formula = FormulaService::submit_for_qc(pool.get_ref(), formula_id, user.user_id()?).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success_with_message(
        formula,
        "Formula submitted for QC review",
    )))
}

#[derive(Debug, serde::Deserialize)]
pub struct QCDecisionRequest {
    pub approved: bool,
    pub notes: Option<String>,
}

pub async fn qc_decision(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    body: web::Json<QCDecisionRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    Authorization::can_approve_qc(&user)?;

    let formula_id = path.into_inner();

    let formula = if body.approved {
        FormulaService::approve_qc(pool.get_ref(), formula_id, user.user_id()?, body.notes.as_deref()).await?
    } else {
        let reason = body.notes.as_deref().unwrap_or("No reason provided");
        FormulaService::reject_qc(pool.get_ref(), formula_id, user.user_id()?, reason).await?
    };

    let msg = if body.approved { "Formula QC approved" } else { "Formula QC rejected" };

    Ok(HttpResponse::Ok().json(ApiResponse::success_with_message(formula, msg)))
}

// ==============================================================================
// LAB TEST HANDLERS
// ==============================================================================

pub async fn create_lab_test(
    pool: web::Data<PgPool>,
    body: web::Json<CreateLabTestRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let test = LabTestService::create(pool.get_ref(), body.into_inner(), user.user_id()?).await?;

    Ok(HttpResponse::Created().json(ApiResponse::success(test)))
}

#[derive(Debug, serde::Deserialize)]
pub struct SubmitLabResultRequest {
    pub result_value: rust_decimal::Decimal,
}

pub async fn submit_lab_result(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    body: web::Json<SubmitLabResultRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let test_id = path.into_inner();
    let test = LabTestService::submit_result(pool.get_ref(), test_id, body.result_value, user.user_id()?).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success_with_message(
        test,
        "Lab test result submitted",
    )))
}

pub async fn list_formula_tests(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let _user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let formula_id = path.into_inner();
    let tests = LabTestService::list_by_formula(pool.get_ref(), formula_id).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(tests)))
}

// ==============================================================================
// MONITORING HANDLERS
// ==============================================================================

pub async fn create_monitoring_session(
    pool: web::Data<PgPool>,
    body: web::Json<CreateMonitoringSessionRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let session = MonitoringService::create_session(pool.get_ref(), body.into_inner(), user.user_id()?).await?;

    Ok(HttpResponse::Created().json(ApiResponse::success(session)))
}

pub async fn submit_monitoring_data(
    pool: web::Data<PgPool>,
    body: web::Json<SubmitMonitoringDataRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let data = MonitoringService::submit_data(pool.get_ref(), body.into_inner(), user.user_id()?).await?;

    Ok(HttpResponse::Created().json(ApiResponse::success(data)))
}

pub async fn verify_monitoring_data(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    Authorization::require_roles(&user, &[UserRole::PrincipalResearcher, UserRole::RdManager, UserRole::SystemAdmin])?;

    let data_id = path.into_inner();
    let data = MonitoringService::verify_data(pool.get_ref(), data_id, user.user_id()?).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(data)))
}

pub async fn complete_monitoring_session(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let _user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let session_id = path.into_inner();
    let session = MonitoringService::complete_session(pool.get_ref(), session_id).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success_with_message(
        session,
        "Monitoring session completed",
    )))
}

// ==============================================================================
// AUDIT LOGS
// ==============================================================================

pub async fn get_entity_audit_logs(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    query: web::Query<PaginationParams>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let _user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

    let entity_id = path.into_inner();
    let logs = AuditService::get_logs(
        pool.get_ref(),
        entity_id,
        query.page.unwrap_or(1) as i64,
        query.per_page.unwrap_or(50) as i64,
    )
    .await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(logs)))
}

// ==============================================================================
// MISSING HANDLERS - ALIASES FOR CONSISTENCY
// ==============================================================================

/// Alias for get_me - used by /auth/me route
pub async fn me(req: HttpRequest, pool: web::Data<PgPool>) -> Result<HttpResponse, AppError> {
    get_me(req, pool).await
}

/// Get a single user by ID
pub async fn get_user(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;
    
    // Only admin or the user themselves can view details
    let target_id = path.into_inner();
    if target_id != user.user_id()? && user.role != UserRole::SystemAdmin {
        return Err(AppError::Authorization("Cannot view other users".to_string()));
    }

    let target_user = UserService::get_by_id(pool.get_ref(), target_id).await?;
    Ok(HttpResponse::Ok().json(ApiResponse::success(target_user)))
}

/// Add team member to a project
pub async fn add_team_member(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    body: web::Json<AddTeamMemberRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;
    
    Authorization::require_roles(&user, &[UserRole::PrincipalResearcher, UserRole::RdManager, UserRole::SystemAdmin])?;
    
    let project_id = path.into_inner();
    let member = ProjectService::add_team_member(
        pool.get_ref(),
        project_id,
        body.user_id,
        &body.role,
    ).await?;

    Ok(HttpResponse::Created().json(ApiResponse::success_with_message(
        member,
        "Team member added successfully",
    )))
}

/// Approve formula QC
pub async fn approve_formula_qc(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    body: web::Json<QCDecisionRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;
    
    Authorization::require_roles(&user, &[UserRole::QcAnalyst, UserRole::RdManager, UserRole::SystemAdmin])?;
    
    let formula_id = path.into_inner();
    let formula = FormulaService::approve_qc(
        pool.get_ref(),
        formula_id,
        user.user_id()?,
        body.notes.as_deref(),
    ).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success_with_message(
        formula,
        "Formula QC approved",
    )))
}

/// Reject formula QC
pub async fn reject_formula_qc(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    body: web::Json<QCDecisionRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;
    
    Authorization::require_roles(&user, &[UserRole::QcAnalyst, UserRole::RdManager, UserRole::SystemAdmin])?;
    
    let formula_id = path.into_inner();
    let reason = body.notes.as_deref().unwrap_or("QC check failed");
    let formula = FormulaService::reject_qc(
        pool.get_ref(),
        formula_id,
        user.user_id()?,
        reason,
    ).await?;

    Ok(HttpResponse::Ok().json(ApiResponse::success_with_message(
        formula,
        "Formula QC rejected",
    )))
}

/// Create new formula version
pub async fn create_formula_version(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    body: web::Json<CreateFormulaVersionRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;
    
    Authorization::require_roles(&user, &[UserRole::PrincipalResearcher, UserRole::RdManager, UserRole::SystemAdmin])?;
    
    let parent_id = path.into_inner();
    let formula = FormulaService::create_version(
        pool.get_ref(),
        parent_id,
        &body.version,
        user.user_id()?,
    ).await?;

    Ok(HttpResponse::Created().json(ApiResponse::success_with_message(
        formula,
        "New formula version created",
    )))
}

/// Submit lab test result - alias for submit_lab_result
pub async fn submit_lab_test_result(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    body: web::Json<SubmitLabResultRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    submit_lab_result(pool, path, body, req).await
}

/// Batch submit monitoring data
pub async fn batch_submit_monitoring_data(
    pool: web::Data<PgPool>,
    body: web::Json<BatchMonitoringDataRequest>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let user = req.extensions().get::<AuthenticatedUser>().cloned()
        .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;
    
    let batch_session_id = body.session_id;
    let mut results = Vec::new();
    
    for item in &body.data {
        let req = SubmitMonitoringDataRequest {
            session_id: batch_session_id,
            unit_id: item.unit_id,
            parameter_id: item.parameter_id,
            numeric_value: item.numeric_value,
            text_value: item.text_value.clone(),
            observation_notes: item.notes.clone(),
        };
        let data = MonitoringService::submit_data(
            pool.get_ref(),
            req,
            user.user_id()?,
        ).await?;
        results.push(data);
    }

    let message = format!("{} data points submitted", body.data.len());
    Ok(HttpResponse::Created().json(ApiResponse::success_with_message(
        results,
        &message,
    )))
}
