use crate::auth::{AccountLockout, JwtService, PasswordService, SessionService};
use crate::config::Settings;
use crate::errors::AppError;
use crate::models::*;
use chrono::{Duration, Utc};
use sqlx::{PgPool, Row, FromRow};
use std::sync::Arc;
use uuid::Uuid;
use validator::Validate;

// ==============================================================================
// USER SERVICE
// ==============================================================================

pub struct UserService;

impl UserService {
    pub async fn create_user(
        pool: &PgPool,
        req: CreateUserRequest,
        created_by: Option<Uuid>,
    ) -> Result<User, AppError> {
        req.validate().map_err(|e| AppError::Validation(e.to_string()))?;

        // Check if email already exists
        let existing: (bool,) = sqlx::query_as(
            r#"SELECT EXISTS(SELECT 1 FROM users WHERE email = $1)"#
        )
        .bind(&req.email)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        if existing.0 {
            return Err(AppError::Validation("Email already registered".to_string()));
        }

        let password_hash = PasswordService::hash_password(&req.password)?;
        let user_id = Uuid::new_v4();
        let role_str = format!("{:?}", req.role).to_lowercase();

        let user: User = sqlx::query_as(
            r#"
            INSERT INTO users (
                id, organization_id, email, password_hash, full_name, 
                employee_id, role, phone, department, position, created_by
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7::user_role, $8, $9, $10, $11)
            RETURNING *
            "#
        )
        .bind(user_id)
        .bind(req.organization_id)
        .bind(&req.email)
        .bind(&password_hash)
        .bind(&req.full_name)
        .bind(&req.employee_id)
        .bind(&role_str)
        .bind(&req.phone)
        .bind(&req.department)
        .bind(&req.position)
        .bind(created_by)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(user)
    }

    pub async fn authenticate(
        pool: &PgPool,
        email: &str,
        password: &str,
    ) -> Result<User, AppError> {
        let user: Option<User> = sqlx::query_as(
            r#"SELECT * FROM users WHERE email = $1"#
        )
        .bind(email)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let user = user.ok_or_else(|| AppError::Authentication("Invalid credentials".to_string()))?;

        // Check if account is locked
        if AccountLockout::is_locked(&user) {
            return Err(AppError::Authentication("Account is temporarily locked".to_string()));
        }

        // Check if account is active
        if !user.is_active {
            return Err(AppError::Authentication("Account is disabled".to_string()));
        }

        // Verify password
        let is_valid = PasswordService::verify_password(password, &user.password_hash)?;

        if !is_valid {
            // Record failed attempt
            AccountLockout::record_failed_attempt(pool, user.id).await?;
            return Err(AppError::Authentication("Invalid credentials".to_string()));
        }

        // Reset failed attempts and update last login
        AccountLockout::reset_attempts(pool, user.id).await?;
        
        sqlx::query("UPDATE users SET last_login = NOW() WHERE id = $1")
            .bind(user.id)
            .execute(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(user)
    }

    pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<User, AppError> {
        let user: User = sqlx::query_as("SELECT * FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;
        Ok(user)
    }

    pub async fn list(
        pool: &PgPool,
        page: i64,
        per_page: i64,
        search: Option<&str>,
    ) -> Result<PaginatedResponse<User>, AppError> {
        let offset = (page - 1) * per_page;

        let (users, total): (Vec<User>, i64) = if let Some(search_term) = search {
            let pattern = format!("%{}%", search_term);
            let users: Vec<User> = sqlx::query_as(
                r#"
                SELECT * FROM users 
                WHERE full_name ILIKE $1 OR email ILIKE $1 OR employee_id ILIKE $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#
            )
            .bind(&pattern)
            .bind(per_page)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

            let total: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM users WHERE full_name ILIKE $1 OR email ILIKE $1"
            )
            .bind(&pattern)
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

            (users, total.0)
        } else {
            let users: Vec<User> = sqlx::query_as(
                "SELECT * FROM users ORDER BY created_at DESC LIMIT $1 OFFSET $2"
            )
            .bind(per_page)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

            let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
                .fetch_one(pool)
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            (users, total.0)
        };

        Ok(PaginatedResponse {
            data: users,
            total,
            page: page as u32,
            per_page: per_page as u32,
            total_pages: (total as f64 / per_page as f64).ceil() as u32,
        })
    }

    pub async fn update_profile(
        pool: &PgPool,
        user_id: Uuid,
        req: UpdateProfileRequest,
    ) -> Result<User, AppError> {
        let user: User = sqlx::query_as(
            r#"
            UPDATE users SET
                full_name = COALESCE($2, full_name),
                phone = COALESCE($3, phone),
                department = COALESCE($4, department),
                position = COALESCE($5, position),
                updated_at = NOW()
            WHERE id = $1
            RETURNING *
            "#
        )
        .bind(user_id)
        .bind(&req.full_name)
        .bind(&req.phone)
        .bind(&req.department)
        .bind(&req.position)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(user)
    }

    pub async fn change_password(
        pool: &PgPool,
        user_id: Uuid,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), AppError> {
        let user = Self::get_by_id(pool, user_id).await?;

        let is_valid = PasswordService::verify_password(current_password, &user.password_hash)?;
        if !is_valid {
            return Err(AppError::Authentication("Current password is incorrect".to_string()));
        }

        let new_hash = PasswordService::hash_password(new_password)?;

        sqlx::query("UPDATE users SET password_hash = $2, updated_at = NOW() WHERE id = $1")
            .bind(user_id)
            .bind(&new_hash)
            .execute(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }
}

// ==============================================================================
// PROJECT SERVICE
// ==============================================================================

pub struct ProjectService;

impl ProjectService {
    pub async fn create(
        pool: &PgPool,
        req: CreateProjectRequest,
        created_by: Uuid,
    ) -> Result<Project, AppError> {
        req.validate().map_err(|e| AppError::Validation(e.to_string()))?;

        let project_id = Uuid::new_v4();
        let status_str = "draft";
        
        // Convert success_metrics to JSON
        let success_metrics_json = req.success_metrics.as_ref()
            .map(|m| serde_json::to_value(m).ok())
            .flatten();

        let project: Project = sqlx::query_as(
            r#"
            INSERT INTO projects (
                id, code, title, background, objectives, hypothesis, 
                methodology, expected_outcomes, success_metrics,
                start_date, end_date, budget_amount, budget_currency,
                crop_type, crop_variety, location_name, location_type,
                location_address, experiment_design, replications,
                treatments_count, blocks_count, plot_size,
                status, created_by, organization_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19::experiment_design, $20, $21, $22, $23, $24::project_status, $25, $26)
            RETURNING *
            "#
        )
        .bind(project_id)
        .bind(&req.code)
        .bind(&req.title)
        .bind(&req.background)
        .bind(&req.objectives)
        .bind(&req.hypothesis)
        .bind(&req.methodology)
        .bind(&req.expected_outcomes)
        .bind(success_metrics_json)
        .bind(req.start_date)
        .bind(req.end_date)
        .bind(req.budget_amount)
        .bind(&req.budget_currency)
        .bind(&req.crop_type)
        .bind(&req.crop_variety)
        .bind(&req.location_name)
        .bind(&req.location_type)
        .bind(&req.location_address)
        .bind(req.experiment_design)
        .bind(req.replications)
        .bind(req.treatments_count)
        .bind(req.blocks_count)
        .bind(&req.plot_size)
        .bind(status_str)
        .bind(created_by)
        .bind::<Option<Uuid>>(None) // organization_id - will be set later
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Add creator as team member
        let _ = Self::add_team_member(pool, project_id, created_by, "principal_researcher").await?;

        // Create audit log
        AuditService::log_simple(pool, project_id, created_by, "project_created", None).await?;

        Ok(project)
    }

    async fn generate_project_code(pool: &PgPool) -> Result<String, AppError> {
        let year = Utc::now().format("%Y");
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM projects WHERE code LIKE $1"
        )
        .bind(format!("CB-{}-%", year))
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(format!("CB-{}-{:03}", year, count.0 + 1))
    }

    pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<Project, AppError> {
        let project: Project = sqlx::query_as("SELECT * FROM projects WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| AppError::NotFound("Project not found".to_string()))?;
        Ok(project)
    }

    pub async fn list(
        pool: &PgPool,
        page: i64,
        per_page: i64,
        status: Option<&str>,
    ) -> Result<PaginatedResponse<Project>, AppError> {
        let offset = (page - 1) * per_page;

        let (projects, total): (Vec<Project>, i64) = if let Some(status_filter) = status {
            let projects: Vec<Project> = sqlx::query_as(
                "SELECT * FROM projects WHERE status = $1::project_status ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            )
            .bind(status_filter)
            .bind(per_page)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

            let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM projects WHERE status = $1::project_status")
                .bind(status_filter)
                .fetch_one(pool)
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            (projects, total.0)
        } else {
            let projects: Vec<Project> = sqlx::query_as(
                "SELECT * FROM projects ORDER BY created_at DESC LIMIT $1 OFFSET $2"
            )
            .bind(per_page)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

            let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM projects")
                .fetch_one(pool)
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            (projects, total.0)
        };

        Ok(PaginatedResponse {
            data: projects,
            total,
            page: page as u32,
            per_page: per_page as u32,
            total_pages: (total as f64 / per_page as f64).ceil() as u32,
        })
    }

    pub async fn update_status(
        pool: &PgPool,
        id: Uuid,
        status: &str,
        user_id: Uuid,
    ) -> Result<Project, AppError> {
        let project: Project = sqlx::query_as(
            "UPDATE projects SET status = $2::project_status, updated_at = NOW() WHERE id = $1 RETURNING *"
        )
        .bind(id)
        .bind(status)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        AuditService::log_simple(pool, id, user_id, "status_changed", Some(&format!("New status: {}", status))).await?;

        Ok(project)
    }

    pub async fn lock_project(
        pool: &PgPool,
        id: Uuid,
        user_id: Uuid,
        lock_reason: Option<&str>,
    ) -> Result<Project, AppError> {
        let project: Project = sqlx::query_as(
            r#"
            UPDATE projects SET 
                is_locked = true,
                locked_at = NOW(),
                locked_by = $2,
                lock_reason = $3,
                updated_at = NOW()
            WHERE id = $1 
            RETURNING *
            "#
        )
        .bind(id)
        .bind(user_id)
        .bind(lock_reason)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        AuditService::log_simple(pool, id, user_id, "project_locked", lock_reason).await?;

        Ok(project)
    }

    pub async fn add_team_member(
        pool: &PgPool,
        project_id: Uuid,
        user_id: Uuid,
        role: &str,
    ) -> Result<ProjectTeamMember, AppError> {
        let member_id = Uuid::new_v4();
        
        let member: ProjectTeamMember = sqlx::query_as(
            r#"
            INSERT INTO project_team_members (id, project_id, user_id, role)
            VALUES ($1, $2, $3, $4::team_member_role)
            ON CONFLICT (project_id, user_id) DO UPDATE SET role = $4::team_member_role
            RETURNING id, project_id, user_id, role, responsibilities, 
                      created_at as assigned_at, assigned_by, is_active
            "#
        )
        .bind(member_id)
        .bind(project_id)
        .bind(user_id)
        .bind(role)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(member)
    }
}

// ==============================================================================
// FORMULA SERVICE
// ==============================================================================

pub struct FormulaService;

impl FormulaService {
    pub async fn create(
        pool: &PgPool,
        req: CreateFormulaRequest,
        created_by: Uuid,
    ) -> Result<Formula, AppError> {
        let formula_id = Uuid::new_v4();
        let code = Self::generate_formula_code(pool).await?;

        let formula: Formula = sqlx::query_as(
            r#"
            INSERT INTO formulas (
                id, project_id, code, name, description, version, status, created_by
            )
            VALUES ($1, $2, $3, $4, $5, 1, 'draft'::formula_status, $6)
            RETURNING *
            "#
        )
        .bind(formula_id)
        .bind(req.project_id)
        .bind(&code)
        .bind(&req.name)
        .bind(&req.description)
        .bind(created_by)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(formula)
    }

    async fn generate_formula_code(pool: &PgPool) -> Result<String, AppError> {
        let year = Utc::now().format("%Y");
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM formulas WHERE code LIKE $1"
        )
        .bind(format!("F-{}-%", year))
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(format!("F-{}-{:04}", year, count.0 + 1))
    }

    pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<Formula, AppError> {
        let formula: Formula = sqlx::query_as("SELECT * FROM formulas WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| AppError::NotFound("Formula not found".to_string()))?;
        Ok(formula)
    }

    pub async fn submit_for_qc(
        pool: &PgPool,
        id: Uuid,
        user_id: Uuid,
    ) -> Result<Formula, AppError> {
        let formula: Formula = sqlx::query_as(
            "UPDATE formulas SET status = 'pending_qc'::formula_status, updated_at = NOW() WHERE id = $1 RETURNING *"
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        AuditService::log_simple(pool, id, user_id, "formula_submitted_qc", None).await?;

        Ok(formula)
    }

    pub async fn approve_qc(
        pool: &PgPool,
        id: Uuid,
        user_id: Uuid,
        notes: Option<&str>,
    ) -> Result<Formula, AppError> {
        let formula: Formula = sqlx::query_as(
            r#"
            UPDATE formulas SET 
                status = 'qc_passed'::formula_status,
                qc_approved_at = NOW(),
                qc_approved_by = $2,
                qc_notes = $3,
                updated_at = NOW()
            WHERE id = $1 
            RETURNING *
            "#
        )
        .bind(id)
        .bind(user_id)
        .bind(notes)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        AuditService::log_simple(pool, id, user_id, "formula_qc_approved", notes).await?;

        Ok(formula)
    }

    pub async fn reject_qc(
        pool: &PgPool,
        id: Uuid,
        user_id: Uuid,
        reason: &str,
    ) -> Result<Formula, AppError> {
        let formula: Formula = sqlx::query_as(
            r#"
            UPDATE formulas SET 
                status = 'qc_failed'::formula_status,
                qc_notes = $3,
                updated_at = NOW()
            WHERE id = $1 
            RETURNING *
            "#
        )
        .bind(id)
        .bind(user_id)
        .bind(reason)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        AuditService::log_simple(pool, id, user_id, "formula_qc_rejected", Some(reason)).await?;

        Ok(formula)
    }

    pub async fn create_version(
        pool: &PgPool,
        parent_id: Uuid,
        version: &str,
        created_by: Uuid,
    ) -> Result<Formula, AppError> {
        // Get parent formula
        let parent = Self::get_by_id(pool, parent_id).await?;
        
        // Mark parent as not latest
        sqlx::query("UPDATE formulas SET is_latest_version = false WHERE id = $1")
            .bind(parent_id)
            .execute(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        // Create new version
        let new_id = Uuid::new_v4();
        let formula: Formula = sqlx::query_as(
            r#"
            INSERT INTO formulas (
                id, project_id, code, name, description, version, 
                parent_formula_id, is_latest_version, status, created_by
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, true, 'draft'::formula_status, $8)
            RETURNING *
            "#
        )
        .bind(new_id)
        .bind(parent.project_id)
        .bind(&parent.code)
        .bind(&parent.name)
        .bind(&parent.description)
        .bind(version)
        .bind(parent_id)
        .bind(created_by)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        AuditService::log_simple(pool, new_id, created_by, "formula_version_created", None).await?;

        Ok(formula)
    }
}

// ==============================================================================
// LAB TEST SERVICE
// ==============================================================================

pub struct LabTestService;

impl LabTestService {
    pub async fn create(
        pool: &PgPool,
        req: CreateLabTestRequest,
        created_by: Uuid,
    ) -> Result<LabTest, AppError> {
        let test_id = Uuid::new_v4();

        let test: LabTest = sqlx::query_as(
            r#"
            INSERT INTO lab_tests (
                id, formula_id, test_code, test_name, test_method,
                parameter_tested, standard_min, standard_max, standard_unit
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#
        )
        .bind(test_id)
        .bind(req.formula_id)
        .bind(&req.test_code)
        .bind(&req.test_name)
        .bind(&req.test_method)
        .bind(&req.parameter_tested)
        .bind(req.standard_min)
        .bind(req.standard_max)
        .bind(&req.standard_unit)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(test)
    }

    pub async fn submit_result(
        pool: &PgPool,
        id: Uuid,
        result_value: rust_decimal::Decimal,
        user_id: Uuid,
    ) -> Result<LabTest, AppError> {
        let test: LabTest = sqlx::query_as(
            r#"
            UPDATE lab_tests SET 
                result_value = $2,
                tested_by = $3,
                tested_at = NOW(),
                updated_at = NOW()
            WHERE id = $1 
            RETURNING *
            "#
        )
        .bind(id)
        .bind(result_value)
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Check if result passes specification
        let is_pass = test.standard_min.map_or(true, |min| result_value >= min)
            && test.standard_max.map_or(true, |max| result_value <= max);

        let status = if is_pass { "completed" } else { "failed" };

        let test: LabTest = sqlx::query_as(
            "UPDATE lab_tests SET status = $2::lab_test_status, is_passed = $3 WHERE id = $1 RETURNING *"
        )
        .bind(id)
        .bind(status)
        .bind(is_pass)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(test)
    }

    pub async fn list_by_formula(pool: &PgPool, formula_id: Uuid) -> Result<Vec<LabTest>, AppError> {
        let tests: Vec<LabTest> = sqlx::query_as(
            "SELECT * FROM lab_tests WHERE formula_id = $1 ORDER BY created_at"
        )
        .bind(formula_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(tests)
    }
}

// ==============================================================================
// MONITORING SERVICE
// ==============================================================================

pub struct MonitoringService;

impl MonitoringService {
    pub async fn create_session(
        pool: &PgPool,
        req: CreateMonitoringSessionRequest,
        created_by: Uuid,
    ) -> Result<MonitoringSession, AppError> {
        let session_id = Uuid::new_v4();

        let session: MonitoringSession = sqlx::query_as(
            r#"
            INSERT INTO monitoring_sessions (
                id, project_id, session_date, weather_condition, temperature,
                humidity, notes, created_by
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#
        )
        .bind(session_id)
        .bind(req.project_id)
        .bind(req.session_date)
        .bind(&req.weather_condition)
        .bind(req.temperature)
        .bind(req.humidity)
        .bind(&req.notes)
        .bind(created_by)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(session)
    }

    pub async fn submit_data(
        pool: &PgPool,
        req: SubmitMonitoringDataRequest,
        created_by: Uuid,
    ) -> Result<MonitoringData, AppError> {
        let data_id = Uuid::new_v4();

        let data: MonitoringData = sqlx::query_as(
            r#"
            INSERT INTO monitoring_data (
                id, session_id, unit_id, parameter_id, numeric_value,
                text_value, observation_notes, recorded_by
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#
        )
        .bind(data_id)
        .bind(req.session_id)
        .bind(req.unit_id)
        .bind(req.parameter_id)
        .bind(req.numeric_value)
        .bind(&req.text_value)
        .bind(&req.observation_notes)
        .bind(created_by)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(data)
    }

    pub async fn verify_data(
        pool: &PgPool,
        id: Uuid,
        user_id: Uuid,
    ) -> Result<MonitoringData, AppError> {
        let data: MonitoringData = sqlx::query_as(
            r#"
            UPDATE monitoring_data SET 
                is_verified = true,
                verified_by = $2,
                verified_at = NOW()
            WHERE id = $1 
            RETURNING *
            "#
        )
        .bind(id)
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(data)
    }

    pub async fn complete_session(
        pool: &PgPool,
        id: Uuid,
    ) -> Result<MonitoringSession, AppError> {
        let session: MonitoringSession = sqlx::query_as(
            "UPDATE monitoring_sessions SET status = 'completed'::monitoring_session_status WHERE id = $1 RETURNING *"
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(session)
    }
}

// ==============================================================================
// AUDIT SERVICE
// ==============================================================================

pub struct AuditService;

impl AuditService {
    /// Main audit log function with full parameters
    pub async fn log(
        pool: &PgPool,
        user_id: Option<Uuid>,
        action: &str,
        entity_type: &str,
        entity_id: Option<Uuid>,
        old_values: Option<serde_json::Value>,
        new_values: Option<serde_json::Value>,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"
            INSERT INTO audit_logs (id, user_id, action, entity_type, entity_id, old_values, new_values, ip_address, user_agent)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(action)
        .bind(entity_type)
        .bind(entity_id)
        .bind(old_values)
        .bind(new_values)
        .bind(ip_address)
        .bind(user_agent)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    /// Simple audit log for internal service use
    pub async fn log_simple(
        pool: &PgPool,
        entity_id: Uuid,
        user_id: Uuid,
        action: &str,
        details: Option<&str>,
    ) -> Result<(), AppError> {
        let new_values = details.map(|d| serde_json::json!({"details": d}));
        sqlx::query(
            r#"
            INSERT INTO audit_logs (id, entity_type, entity_id, user_id, action, new_values)
            VALUES ($1, 'project', $2, $3, $4, $5)
            "#
        )
        .bind(Uuid::new_v4())
        .bind(entity_id)
        .bind(user_id)
        .bind(action)
        .bind(new_values)
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    pub async fn get_logs(
        pool: &PgPool,
        entity_id: Uuid,
        page: i64,
        per_page: i64,
    ) -> Result<Vec<AuditLog>, AppError> {
        let offset = (page - 1) * per_page;

        let logs: Vec<AuditLog> = sqlx::query_as(
            "SELECT * FROM audit_logs WHERE entity_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(entity_id)
        .bind(per_page)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(logs)
    }
}
