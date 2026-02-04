mod analysis;
mod auth;
mod config;
mod errors;
mod handlers;
mod models;
mod qrcode;
mod reports;
mod services;

use actix_cors::Cors;
use actix_files::Files;
use actix_web::{middleware, web, App, HttpServer};
use actix_web_httpauth::middleware::HttpAuthentication;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use auth::{validator, JwtService, RateLimiter};
use config::Settings;
use qrcode::QRCodeService;
use reports::ReportGenerator;

// ==============================================================================
// MAIN APPLICATION
// ==============================================================================

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,sqlx=warn".to_string()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting CENTRABIO R&D NEXUS - Scientific Decision Support System");

    // Load configuration
    let settings = Settings::from_env().expect("Failed to load configuration");
    info!("Configuration loaded successfully");

    // Create database connection pool
    let pool = PgPoolOptions::new()
        .max_connections(settings.database.max_connections)
        .connect(&settings.database.connection_string())
        .await
        .expect("Failed to create database pool");

    info!("Database connection pool established");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run database migrations");

    info!("Database migrations completed");

    // Initialize services
    let jwt_service = Arc::new(JwtService::new(&settings.jwt.secret));
    let rate_limiter = Arc::new(RateLimiter::new(
        settings.security.rate_limit_requests as usize,
        settings.security.rate_limit_window,
    ));
    let qr_service = web::Data::new(QRCodeService::new(&settings.server.public_url));
    let report_generator = web::Data::new(ReportGenerator::new(&settings));

    // Clone settings for handler access
    let settings_data = web::Data::new(settings.clone());

    info!(
        "Starting HTTP server on {}:{}",
        settings.server.host, settings.server.port
    );

    // Start HTTP server
    HttpServer::new(move || {
        // Configure CORS
        let cors = Cors::default()
            .allowed_origin_fn(|origin, _req_head| {
                // In production, restrict this to specific origins
                origin.as_bytes().starts_with(b"http://localhost")
                    || origin.as_bytes().starts_with(b"https://")
            })
            .allowed_methods(vec!["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"])
            .allowed_headers(vec![
                actix_web::http::header::AUTHORIZATION,
                actix_web::http::header::ACCEPT,
                actix_web::http::header::CONTENT_TYPE,
            ])
            .supports_credentials()
            .max_age(3600);

        // Configure authentication middleware
        let auth_middleware = HttpAuthentication::bearer(validator);

        App::new()
            // Global middleware
            .wrap(middleware::Logger::default())
            .wrap(middleware::Compress::default())
            .wrap(cors)
            // Application data
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(jwt_service.clone()))
            .app_data(web::Data::new(rate_limiter.clone()))
            .app_data(settings_data.clone())
            .app_data(qr_service.clone())
            .app_data(report_generator.clone())
            // Health check endpoints (no auth required)
            .route("/health", web::get().to(handlers::health_check))
            .route("/ready", web::get().to(handlers::readiness_check))
            // API v1 routes
            .service(
                web::scope("/api/v1")
                    // Public auth routes (no auth required)
                    .service(
                        web::scope("/auth")
                            .route("/register", web::post().to(handlers::register))
                            .route("/login", web::post().to(handlers::login))
                            .route("/refresh", web::post().to(handlers::refresh_token))
                    )
                    // Protected routes (auth required)
                    .service(
                        web::scope("")
                            .wrap(auth_middleware.clone())
                            // Auth routes
                            .route("/auth/logout", web::post().to(handlers::logout))
                            .route("/auth/me", web::get().to(handlers::me))
                            // User routes
                            .service(
                                web::scope("/users")
                                    .route("", web::get().to(handlers::list_users))
                                    .route("/{id}", web::get().to(handlers::get_user))
                                    .route("/profile", web::put().to(handlers::update_profile))
                                    .route("/password", web::put().to(handlers::change_password))
                            )
                            // Project routes
                            .service(
                                web::scope("/projects")
                                    .route("", web::post().to(handlers::create_project))
                                    .route("", web::get().to(handlers::list_projects))
                                    .route("/{id}", web::get().to(handlers::get_project))
                                    .route("/{id}/status", web::put().to(handlers::update_project_status))
                                    .route("/{id}/lock", web::post().to(handlers::lock_project))
                                    .route("/{id}/team", web::post().to(handlers::add_team_member))
                                    .route("/{id}/qr-codes", web::post().to(qrcode::generate_project_qr_codes_handler))
                                    .route("/{id}/qr-print", web::get().to(qrcode::generate_qr_print_sheet))
                            )
                            // Formula routes
                            .service(
                                web::scope("/formulas")
                                    .route("", web::post().to(handlers::create_formula))
                                    .route("/{id}", web::get().to(handlers::get_formula))
                                    .route("/{id}/submit-qc", web::post().to(handlers::submit_formula_for_qc))
                                    .route("/{id}/approve-qc", web::post().to(handlers::approve_formula_qc))
                                    .route("/{id}/reject-qc", web::post().to(handlers::reject_formula_qc))
                                    .route("/{id}/new-version", web::post().to(handlers::create_formula_version))
                                    .route("/{id}/tests", web::get().to(handlers::list_formula_tests))
                            )
                            // Lab test routes
                            .service(
                                web::scope("/lab-tests")
                                    .route("", web::post().to(handlers::create_lab_test))
                                    .route("/{id}/result", web::post().to(handlers::submit_lab_test_result))
                            )
                            // Monitoring routes
                            .service(
                                web::scope("/monitoring")
                                    .route("/sessions", web::post().to(handlers::create_monitoring_session))
                                    .route("/sessions/{id}/complete", web::post().to(handlers::complete_monitoring_session))
                                    .route("/data", web::post().to(handlers::submit_monitoring_data))
                                    .route("/data/batch", web::post().to(handlers::batch_submit_monitoring_data))
                                    .route("/data/{id}/verify", web::post().to(handlers::verify_monitoring_data))
                            )
                            // QR code routes
                            .service(
                                web::scope("/qr")
                                    .route("/block/{id}", web::get().to(qrcode::generate_block_qr_handler))
                                    .route("/unit/{id}", web::get().to(qrcode::generate_unit_qr_handler))
                                    .route("/scan", web::post().to(qrcode::scan_qr_handler))
                            )
                            // Analysis routes
                            .route("/analysis/descriptive", web::post().to(analysis_handler::descriptive_stats))
                            .route("/analysis/anova", web::post().to(analysis_handler::anova_analysis))
                            .route("/analysis/ai", web::post().to(analysis_handler::ai_analysis))
                            .route("/analysis/cost-benefit", web::post().to(analysis_handler::cost_benefit))
                            // Report routes
                            .route("/reports/generate", web::post().to(report_handler::generate_report))
                    )
            )
            // Static files (must be last to catch all other routes)
            .service(Files::new("/", "./static").index_file("index.html"))
    })
    .bind(format!("{}:{}", settings.server.host, settings.server.port))?
    .run()
    .await
}

// ==============================================================================
// ANALYSIS HANDLERS MODULE
// ==============================================================================

mod analysis_handler {
    use super::*;
    use crate::analysis::{AIAnalysisService, CostBenefitAnalysis, StatisticalAnalysis};
    use crate::auth::AuthenticatedUser;
    use crate::errors::AppError;
    use crate::models::ApiResponse;
    use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
    use sqlx::PgPool;

    #[derive(Debug, serde::Deserialize)]
    pub struct DescriptiveStatsRequest {
        pub values: Vec<f64>,
    }

    pub async fn descriptive_stats(
        body: web::Json<DescriptiveStatsRequest>,
        req: HttpRequest,
    ) -> Result<HttpResponse, AppError> {
        let _user = req
            .extensions()
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

        let stats = StatisticalAnalysis::descriptive(&body.values);

        Ok(HttpResponse::Ok().json(ApiResponse::success(stats)))
    }

    #[derive(Debug, serde::Deserialize)]
    pub struct AnovaRequest {
        pub groups: Vec<Vec<f64>>,
    }

    pub async fn anova_analysis(
        body: web::Json<AnovaRequest>,
        req: HttpRequest,
    ) -> Result<HttpResponse, AppError> {
        let _user = req
            .extensions()
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

        let result = StatisticalAnalysis::one_way_anova(&body.groups);

        // Calculate LSD if significant
        let lsd_comparisons = if result.is_significant_05 {
            Some(StatisticalAnalysis::lsd_test(
                &body.groups,
                result.source_within.ms,
                result.source_within.df as f64,
            ))
        } else {
            None
        };

        Ok(HttpResponse::Ok().json(serde_json::json!({
            "anova": result,
            "lsd_comparisons": lsd_comparisons
        })))
    }

    #[derive(Debug, serde::Deserialize)]
    pub struct AIAnalysisRequest {
        pub project_id: uuid::Uuid,
        pub analysis_type: String,
    }

    pub async fn ai_analysis(
        pool: web::Data<PgPool>,
        settings: web::Data<Settings>,
        body: web::Json<AIAnalysisRequest>,
        req: HttpRequest,
    ) -> Result<HttpResponse, AppError> {
        let _user = req
            .extensions()
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

        let ai_service = AIAnalysisService::new(&settings)
            .ok_or_else(|| AppError::AIError("OpenAI API not configured".to_string()))?;

        let result = ai_service
            .analyze_experiment_data(pool.get_ref(), body.project_id, &body.analysis_type)
            .await?;

        Ok(HttpResponse::Ok().json(ApiResponse::success(result)))
    }

    #[derive(Debug, serde::Deserialize)]
    pub struct CostBenefitRequest {
        pub project_id: uuid::Uuid,
        pub crop_price_per_kg: rust_decimal::Decimal,
    }

    pub async fn cost_benefit(
        pool: web::Data<PgPool>,
        body: web::Json<CostBenefitRequest>,
        req: HttpRequest,
    ) -> Result<HttpResponse, AppError> {
        let _user = req
            .extensions()
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

        let result = CostBenefitAnalysis::analyze(
            pool.get_ref(),
            body.project_id,
            body.crop_price_per_kg,
        )
        .await?;

        Ok(HttpResponse::Ok().json(ApiResponse::success(result)))
    }
}

// ==============================================================================
// REPORT HANDLERS MODULE
// ==============================================================================

mod report_handler {
    use super::*;
    use crate::auth::AuthenticatedUser;
    use crate::errors::AppError;
    use crate::models::ApiResponse;
    use crate::reports::{ReportGenerator, ReportSection, ReportType};
    use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
    use sqlx::PgPool;

    #[derive(Debug, serde::Deserialize)]
    pub struct GenerateReportRequest {
        pub project_id: uuid::Uuid,
        pub report_type: String,
        pub sections: Option<Vec<String>>,
        pub include_ai_insights: Option<bool>,
    }

    pub async fn generate_report(
        pool: web::Data<PgPool>,
        settings: web::Data<Settings>,
        report_generator: web::Data<ReportGenerator>,
        body: web::Json<GenerateReportRequest>,
        req: HttpRequest,
    ) -> Result<HttpResponse, AppError> {
        let user = req
            .extensions()
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or_else(|| AppError::Authentication("Not authenticated".to_string()))?;

        let report_type = match body.report_type.to_lowercase().as_str() {
            "executive_summary" | "executive" => ReportType::ExecutiveSummary,
            "full" | "full_experiment" => ReportType::FullExperiment,
            "statistical" | "statistics" => ReportType::StatisticalAnalysis,
            "qc" | "qc_report" => ReportType::QCReport,
            "field" | "field_progress" | "progress" => ReportType::FieldProgress,
            _ => {
                return Err(AppError::Validation(format!(
                    "Invalid report type: {}",
                    body.report_type
                )))
            }
        };

        let sections: Vec<ReportSection> = body
            .sections
            .as_ref()
            .map(|s| {
                s.iter()
                    .filter_map(|sec| match sec.to_lowercase().as_str() {
                        "overview" => Some(ReportSection::Overview),
                        "methodology" => Some(ReportSection::Methodology),
                        "treatments" => Some(ReportSection::Treatments),
                        "results" => Some(ReportSection::Results),
                        "statistics" => Some(ReportSection::Statistics),
                        "ai" | "ai_insights" => Some(ReportSection::AIInsights),
                        "conclusions" => Some(ReportSection::Conclusions),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![
                    ReportSection::Overview,
                    ReportSection::Methodology,
                    ReportSection::Treatments,
                    ReportSection::Results,
                    ReportSection::Statistics,
                    ReportSection::Conclusions,
                ]
            });

        // Get AI insights if requested
        let ai_insights = if body.include_ai_insights.unwrap_or(false) {
            use crate::analysis::AIAnalysisService;
            if let Some(ai_service) = AIAnalysisService::new(&settings) {
                ai_service
                    .generate_report_insights(pool.get_ref(), body.project_id)
                    .await
                    .ok()
            } else {
                None
            }
        } else {
            None
        };

        let report = report_generator
            .generate_project_report(
                pool.get_ref(),
                body.project_id,
                report_type,
                &sections,
                ai_insights,
            )
            .await?;

        Ok(HttpResponse::Ok().json(ApiResponse::success(report)))
    }
}
