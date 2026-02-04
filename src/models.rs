use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use validator::Validate;

// ==============================================================================
// ENUMS (matching database enums)
// ==============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "user_role", rename_all = "snake_case")]
pub enum UserRole {
    PrincipalResearcher,
    QcAnalyst,
    FieldOfficer,
    RdManager,
    SystemAdmin,
}

impl UserRole {
    pub fn can_create_project(&self) -> bool {
        matches!(self, UserRole::PrincipalResearcher | UserRole::RdManager | UserRole::SystemAdmin)
    }

    pub fn can_approve_qc(&self) -> bool {
        matches!(self, UserRole::QcAnalyst | UserRole::RdManager | UserRole::SystemAdmin)
    }

    pub fn can_lock_project(&self) -> bool {
        matches!(self, UserRole::RdManager | UserRole::SystemAdmin)
    }

    pub fn can_input_monitoring(&self) -> bool {
        matches!(self, UserRole::FieldOfficer | UserRole::PrincipalResearcher | UserRole::SystemAdmin)
    }

    pub fn can_view_all_projects(&self) -> bool {
        matches!(self, UserRole::RdManager | UserRole::SystemAdmin)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "project_status", rename_all = "snake_case")]
pub enum ProjectStatus {
    Draft,
    Active,
    OnHold,
    Completed,
    Archived,
    Locked,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "formula_status", rename_all = "snake_case")]
pub enum FormulaStatus {
    Draft,
    PendingQc,
    QcInProgress,
    QcPassed,
    QcFailed,
    RevisionRequired,
    Archived,
}

impl FormulaStatus {
    pub fn can_be_used_in_field(&self) -> bool {
        matches!(self, FormulaStatus::QcPassed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "lab_test_status", rename_all = "snake_case")]
pub enum LabTestStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "experiment_design", rename_all = "snake_case")]
pub enum ExperimentDesign {
    Rak,
    Ral,
    Factorial,
    SplitPlot,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "monitoring_type", rename_all = "snake_case")]
pub enum MonitoringType {
    Height,
    LeafCount,
    StemDiameter,
    LeafArea,
    Chlorophyll,
    PestLevel,
    DiseaseLevel,
    Yield,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "measurement_unit", rename_all = "snake_case")]
pub enum MeasurementUnit {
    Cm, Mm, M,
    G, Kg, Mg,
    Ml, L, Ul,
    Ppm, Percent,
    Count, Score,
    Celsius, Ph,
    Custom,
}

// ==============================================================================
// DATABASE MODELS
// ==============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub description: Option<String>,
    pub address: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub logo_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub organization_id: Option<Uuid>,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub full_name: String,
    pub employee_id: Option<String>,
    pub role: UserRole,
    pub phone: Option<String>,
    pub avatar_url: Option<String>,
    pub department: Option<String>,
    pub position: Option<String>,
    pub is_active: bool,
    pub is_email_verified: bool,
    pub last_login: Option<DateTime<Utc>>,
    pub failed_login_attempts: i32,
    pub locked_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Project {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub code: String,
    pub title: String,
    pub background: Option<String>,
    pub objectives: Option<String>,
    pub hypothesis: Option<String>,
    pub methodology: Option<String>,
    pub expected_outcomes: Option<String>,
    pub success_metrics: Option<serde_json::Value>,
    pub status: ProjectStatus,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub actual_end_date: Option<chrono::NaiveDate>,
    pub budget_amount: Option<rust_decimal::Decimal>,
    pub budget_currency: Option<String>,
    pub actual_cost: Option<rust_decimal::Decimal>,
    pub crop_type: Option<String>,
    pub crop_variety: Option<String>,
    pub growth_stage: Option<String>,
    pub location_name: Option<String>,
    pub location_type: Option<String>,
    pub location_address: Option<String>,
    pub experiment_design: Option<ExperimentDesign>,
    pub replications: Option<i32>,
    pub treatments_count: Option<i32>,
    pub blocks_count: Option<i32>,
    pub plot_size: Option<String>,
    pub is_locked: bool,
    pub locked_at: Option<DateTime<Utc>>,
    pub locked_by: Option<Uuid>,
    pub lock_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ProjectTeamMember {
    pub id: Uuid,
    pub project_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub responsibilities: Option<String>,
    pub assigned_at: DateTime<Utc>,
    pub assigned_by: Option<Uuid>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Formula {
    pub id: Uuid,
    pub project_id: Uuid,
    pub code: String,
    pub name: String,
    pub version: String,
    pub parent_formula_id: Option<Uuid>,
    pub is_latest_version: bool,
    pub status: FormulaStatus,
    pub description: Option<String>,
    pub intended_use: Option<String>,
    pub target_crop: Option<String>,
    pub application_method: Option<String>,
    pub application_rate: Option<String>,
    pub total_volume: Option<rust_decimal::Decimal>,
    pub volume_unit: Option<String>,
    pub calculated_cost: Option<rust_decimal::Decimal>,
    pub cost_per_unit: Option<rust_decimal::Decimal>,
    pub cost_currency: Option<String>,
    pub target_ph_min: Option<rust_decimal::Decimal>,
    pub target_ph_max: Option<rust_decimal::Decimal>,
    pub target_density: Option<rust_decimal::Decimal>,
    pub target_viscosity: Option<rust_decimal::Decimal>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub qc_approved_by: Option<Uuid>,
    pub qc_approved_at: Option<DateTime<Utc>>,
    pub qc_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RawMaterial {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub code: String,
    pub name: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub stock_quantity: Option<rust_decimal::Decimal>,
    pub stock_unit: Option<String>,
    pub minimum_stock: Option<rust_decimal::Decimal>,
    pub unit_cost: Option<rust_decimal::Decimal>,
    pub cost_currency: Option<String>,
    pub specifications: Option<serde_json::Value>,
    pub safety_data_sheet: Option<String>,
    pub handling_instructions: Option<String>,
    pub storage_requirements: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FormulaIngredient {
    pub id: Uuid,
    pub formula_id: Uuid,
    pub raw_material_id: Uuid,
    pub quantity: rust_decimal::Decimal,
    pub unit: String,
    pub percentage: Option<rust_decimal::Decimal>,
    pub function_role: Option<String>,
    pub notes: Option<String>,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct LabTest {
    pub id: Uuid,
    pub formula_id: Uuid,
    pub test_code: String,
    pub test_name: String,
    pub test_method: Option<String>,
    pub parameter_tested: Option<String>,
    pub standard_min: Option<rust_decimal::Decimal>,
    pub standard_max: Option<rust_decimal::Decimal>,
    pub standard_unit: Option<String>,
    pub result_value: Option<rust_decimal::Decimal>,
    pub result_unit: Option<String>,
    pub status: LabTestStatus,
    pub is_passed: Option<bool>,
    pub tested_by: Option<Uuid>,
    pub tested_at: Option<DateTime<Utc>>,
    pub equipment_used: Option<String>,
    pub notes: Option<String>,
    pub observations: Option<String>,
    pub attachments: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ExperimentalBlock {
    pub id: Uuid,
    pub project_id: Uuid,
    pub block_code: String,
    pub block_name: Option<String>,
    pub formula_id: Option<Uuid>,
    pub treatment_description: Option<String>,
    pub is_control: bool,
    pub position_row: Option<i32>,
    pub position_column: Option<i32>,
    pub area_size: Option<rust_decimal::Decimal>,
    pub area_unit: Option<String>,
    pub plant_count: Option<i32>,
    pub qr_code_data: Option<String>,
    pub qr_code_generated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ExperimentalUnit {
    pub id: Uuid,
    pub block_id: Uuid,
    pub unit_code: String,
    pub unit_label: Option<String>,
    pub position_in_block: Option<i32>,
    pub row_number: Option<i32>,
    pub column_number: Option<i32>,
    pub qr_code_data: Option<String>,
    pub qr_code_url: Option<String>,
    pub is_active: bool,
    pub excluded_reason: Option<String>,
    pub excluded_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MonitoringParameter {
    pub id: Uuid,
    pub project_id: Uuid,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub parameter_type: Option<MonitoringType>,
    pub data_type: String,
    pub unit: Option<MeasurementUnit>,
    pub custom_unit: Option<String>,
    pub min_value: Option<rust_decimal::Decimal>,
    pub max_value: Option<rust_decimal::Decimal>,
    pub decimal_places: Option<i32>,
    pub outlier_threshold_percent: Option<rust_decimal::Decimal>,
    pub sort_order: i32,
    pub is_required: bool,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MonitoringSession {
    pub id: Uuid,
    pub project_id: Uuid,
    pub session_code: String,
    pub session_name: Option<String>,
    pub scheduled_date: chrono::NaiveDate,
    pub actual_date: Option<chrono::NaiveDate>,
    pub days_after_treatment: Option<i32>,
    pub week_number: Option<i32>,
    pub is_completed: bool,
    pub completed_at: Option<DateTime<Utc>>,
    pub completed_by: Option<Uuid>,
    pub weather_conditions: Option<String>,
    pub general_observations: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MonitoringData {
    pub id: Uuid,
    pub session_id: Uuid,
    pub unit_id: Uuid,
    pub parameter_id: Uuid,
    pub numeric_value: Option<rust_decimal::Decimal>,
    pub text_value: Option<String>,
    pub boolean_value: Option<bool>,
    pub is_outlier: bool,
    pub outlier_reason: Option<String>,
    pub is_verified: bool,
    pub verified_by: Option<Uuid>,
    pub verified_at: Option<DateTime<Utc>>,
    pub collected_by: Uuid,
    pub collected_at: DateTime<Utc>,
    pub device_id: Option<String>,
    pub latitude: Option<rust_decimal::Decimal>,
    pub longitude: Option<rust_decimal::Decimal>,
    pub notes: Option<String>,
    pub offline_id: Option<Uuid>,
    pub synced_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AnalysisResult {
    pub id: Uuid,
    pub project_id: Uuid,
    pub analysis_type: String,
    pub analysis_name: Option<String>,
    pub input_parameters: serde_json::Value,
    pub results: serde_json::Value,
    pub ai_model_used: Option<String>,
    pub ai_prompt: Option<String>,
    pub ai_insights: Option<String>,
    pub ai_recommendations: Option<String>,
    pub generated_at: DateTime<Utc>,
    pub generated_by: Uuid,
    pub is_valid: bool,
    pub invalidated_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AuditLog {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    pub action: String,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub old_values: Option<serde_json::Value>,
    pub new_values: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ==============================================================================
// DTOs (Data Transfer Objects)
// ==============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateUserRequest {
    #[validate(email(message = "Invalid email format"))]
    pub email: String,
    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,
    #[validate(length(min = 2, max = 255, message = "Full name must be between 2 and 255 characters"))]
    pub full_name: String,
    pub employee_id: Option<String>,
    pub role: UserRole,
    pub phone: Option<String>,
    pub department: Option<String>,
    pub position: Option<String>,
    pub organization_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email(message = "Invalid email format"))]
    pub email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub user: UserResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub full_name: String,
    pub role: UserRole,
    pub organization_id: Option<Uuid>,
    pub department: Option<String>,
    pub position: Option<String>,
    pub avatar_url: Option<String>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            email: user.email,
            full_name: user.full_name,
            role: user.role,
            organization_id: user.organization_id,
            department: user.department,
            position: user.position,
            avatar_url: user.avatar_url,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateProjectRequest {
    #[validate(length(min = 3, max = 50, message = "Project code must be between 3 and 50 characters"))]
    pub code: String,
    #[validate(length(min = 5, max = 500, message = "Title must be between 5 and 500 characters"))]
    pub title: String,
    pub background: Option<String>,
    pub objectives: Option<String>,
    pub hypothesis: Option<String>,
    pub methodology: Option<String>,
    pub expected_outcomes: Option<String>,
    pub success_metrics: Option<Vec<SuccessMetric>>,
    pub start_date: Option<chrono::NaiveDate>,
    pub end_date: Option<chrono::NaiveDate>,
    pub budget_amount: Option<rust_decimal::Decimal>,
    pub budget_currency: Option<String>,
    pub crop_type: Option<String>,
    pub crop_variety: Option<String>,
    pub location_name: Option<String>,
    pub location_type: Option<String>,
    pub location_address: Option<String>,
    pub experiment_design: Option<ExperimentDesign>,
    pub replications: Option<i32>,
    pub treatments_count: Option<i32>,
    pub blocks_count: Option<i32>,
    pub plot_size: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessMetric {
    pub name: String,
    pub target: String,
    pub unit: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateFormulaRequest {
    pub project_id: Uuid,
    #[validate(length(min = 2, max = 50, message = "Formula code must be between 2 and 50 characters"))]
    pub code: String,
    #[validate(length(min = 2, max = 255, message = "Formula name must be between 2 and 255 characters"))]
    pub name: String,
    pub description: Option<String>,
    pub intended_use: Option<String>,
    pub target_crop: Option<String>,
    pub application_method: Option<String>,
    pub application_rate: Option<String>,
    pub total_volume: Option<rust_decimal::Decimal>,
    pub volume_unit: Option<String>,
    pub target_ph_min: Option<rust_decimal::Decimal>,
    pub target_ph_max: Option<rust_decimal::Decimal>,
    pub target_density: Option<rust_decimal::Decimal>,
    pub target_viscosity: Option<rust_decimal::Decimal>,
    pub ingredients: Vec<FormulaIngredientInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormulaIngredientInput {
    pub raw_material_id: Uuid,
    pub quantity: rust_decimal::Decimal,
    pub unit: String,
    pub percentage: Option<rust_decimal::Decimal>,
    pub function_role: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateLabTestRequest {
    pub formula_id: Uuid,
    #[validate(length(min = 2, max = 50))]
    pub test_code: String,
    #[validate(length(min = 2, max = 255))]
    pub test_name: String,
    pub test_method: Option<String>,
    pub parameter_tested: Option<String>,
    pub standard_min: Option<rust_decimal::Decimal>,
    pub standard_max: Option<rust_decimal::Decimal>,
    pub standard_unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitLabTestResultRequest {
    pub result_value: rust_decimal::Decimal,
    pub result_unit: Option<String>,
    pub equipment_used: Option<String>,
    pub notes: Option<String>,
    pub observations: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateMonitoringDataRequest {
    pub session_id: Uuid,
    pub unit_id: Uuid,
    pub parameter_id: Uuid,
    pub numeric_value: Option<rust_decimal::Decimal>,
    pub text_value: Option<String>,
    pub boolean_value: Option<bool>,
    pub latitude: Option<rust_decimal::Decimal>,
    pub longitude: Option<rust_decimal::Decimal>,
    pub device_id: Option<String>,
    pub notes: Option<String>,
    pub offline_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchMonitoringDataRequest {
    pub session_id: Uuid,
    pub data: Vec<CreateMonitoringDataRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRequest {
    pub project_id: Uuid,
    pub analysis_type: String, // anova, comparison, cost_benefit
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateReportRequest {
    pub project_id: Uuid,
    pub report_type: String,
    pub title: String,
    pub sections: Vec<String>,
    pub include_ai_insights: bool,
}

// Pagination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
}

impl Default for PaginationParams {
    fn default() -> Self {
        Self {
            page: Some(1),
            per_page: Some(20),
            sort_by: None,
            sort_order: Some("desc".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: i64,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

impl<T> PaginatedResponse<T> {
    pub fn new(data: Vec<T>, total: i64, page: u32, per_page: u32) -> Self {
        let total_pages = ((total as f64) / (per_page as f64)).ceil() as u32;
        Self {
            data,
            total,
            page,
            per_page,
            total_pages,
        }
    }
}

// API Response wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub message: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: None,
        }
    }

    pub fn success_with_message(data: T, message: &str) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: Some(message.to_string()),
        }
    }
}

impl ApiResponse<()> {
    pub fn message(message: &str) -> Self {
        Self {
            success: true,
            data: None,
            message: Some(message.to_string()),
        }
    }

    pub fn success_message(message: &str) -> Self {
        Self {
            success: true,
            data: None,
            message: Some(message.to_string()),
        }
    }
}

// ==============================================================================
// ADDITIONAL REQUEST/RESPONSE DTOs
// ==============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProfileRequest {
    pub full_name: Option<String>,
    pub phone: Option<String>,
    pub department: Option<String>,
    pub position: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMonitoringSessionRequest {
    pub project_id: Uuid,
    pub session_date: chrono::NaiveDate,
    pub weather_condition: Option<String>,
    pub temperature: Option<rust_decimal::Decimal>,
    pub humidity: Option<rust_decimal::Decimal>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitMonitoringDataRequest {
    pub session_id: Uuid,
    pub unit_id: Uuid,
    pub parameter_id: Uuid,
    pub numeric_value: Option<rust_decimal::Decimal>,
    pub text_value: Option<String>,
    pub observation_notes: Option<String>,
}

// ==============================================================================
// ADDITIONAL DTOs FOR HANDLERS
// ==============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTeamMemberRequest {
    pub user_id: Uuid,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QCDecisionRequest {
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFormulaVersionRequest {
    pub version: String,
}
