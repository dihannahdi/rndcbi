use crate::config::Settings;
use crate::errors::AppError;
use crate::models::*;
use crate::analysis::{StatisticalAnalysis, DescriptiveStats, AnovaResult};
use chrono::{NaiveDate, Utc};
use printpdf::*;
use serde::{Serialize, Deserialize};
use sqlx::{PgPool, Row, FromRow};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use uuid::Uuid;

// ==============================================================================
// REPORT GENERATOR SERVICE
// ==============================================================================

pub struct ReportGenerator {
    storage_path: String,
}

impl ReportGenerator {
    pub fn new(settings: &Settings) -> Self {
        Self {
            storage_path: settings.storage.reports_path.clone(),
        }
    }

    pub async fn generate_project_report(
        &self,
        pool: &PgPool,
        project_id: Uuid,
        report_type: ReportType,
        sections: &[ReportSection],
        ai_insights: Option<String>,
    ) -> Result<GeneratedReport, AppError> {
        // Fetch project data
        let project = self.fetch_project_data(pool, project_id).await?;

        // Generate report based on type
        let (content, pdf_path) = match report_type {
            ReportType::ExecutiveSummary => {
                self.generate_executive_summary(&project, ai_insights.as_deref()).await?
            }
            ReportType::FullExperiment => {
                self.generate_full_report(&project, sections, ai_insights.as_deref()).await?
            }
            ReportType::StatisticalAnalysis => {
                self.generate_statistical_report(&project).await?
            }
            ReportType::QCReport => {
                self.generate_qc_report(&project).await?
            }
            ReportType::FieldProgress => {
                self.generate_field_progress(&project).await?
            }
        };

        // Save report record to database
        let report_id = Uuid::new_v4();
        let report = sqlx::query_as::<_, GeneratedReportRecord>(
            r#"
            INSERT INTO generated_reports (
                id, project_id, report_type, content_json, 
                pdf_path, generated_by
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING 
                id, project_id, report_type,
                content_json, pdf_path, generated_at, generated_by, is_latest
            "#
        )
        .bind(report_id)
        .bind(project_id)
        .bind(report_type.to_string())
        .bind(serde_json::to_value(&content).unwrap_or_default())
        .bind(pdf_path.clone())
        .bind(project.project.created_by)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(GeneratedReport {
            id: report.id,
            project_id: report.project_id,
            report_type,
            title: report.report_type.clone().unwrap_or_default(),
            content,
            pdf_path: report.pdf_path.clone(),
            generated_at: report.generated_at.unwrap_or_else(|| Utc::now()),
        })
    }

    async fn fetch_project_data(
        &self,
        pool: &PgPool,
        project_id: Uuid,
    ) -> Result<ProjectReportData, AppError> {
        // Fetch project
        let project = sqlx::query_as::<_, Project>(
            r#"
            SELECT 
                id, organization_id, code, title, background, objectives,
                hypothesis, methodology, expected_outcomes, success_metrics,
                status, start_date, end_date, 
                actual_end_date, budget_amount, budget_currency, actual_cost,
                crop_type, crop_variety, growth_stage, location_name, 
                location_type, location_address, 
                experiment_design,
                replications, treatments_count, blocks_count, plot_size,
                is_locked, locked_at, locked_by, lock_reason,
                created_at, updated_at, created_by, approved_by, approved_at
            FROM projects WHERE id = $1
            "#
        )
        .bind(project_id)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Fetch formulas
        let formulas = sqlx::query_as::<_, Formula>(
            r#"
            SELECT 
                id, project_id, code, name, version, parent_formula_id,
                is_latest_version, status,
                description, intended_use, target_crop, application_method,
                application_rate, total_volume, volume_unit, calculated_cost,
                cost_per_unit, cost_currency, target_ph_min, target_ph_max,
                target_density, target_viscosity, created_at, updated_at,
                created_by, qc_approved_by, qc_approved_at, qc_notes
            FROM formulas WHERE project_id = $1 AND is_latest_version = true
            "#
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Fetch experimental blocks
        let blocks = sqlx::query_as::<_, ExperimentalBlock>(
            r#"
            SELECT 
                id, project_id, block_code, block_name, formula_id,
                treatment_description, is_control, position_row, position_column,
                area_size, area_unit, plant_count, qr_code_data,
                qr_code_generated_at, created_at, updated_at
            FROM experimental_blocks WHERE project_id = $1
            ORDER BY block_code
            "#
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Fetch monitoring sessions
        let sessions = sqlx::query_as::<_, MonitoringSession>(
            r#"
            SELECT 
                id, project_id, session_code, session_name, scheduled_date,
                actual_date, days_after_treatment, week_number, is_completed,
                completed_at, completed_by, weather_conditions,
                general_observations, notes, created_at
            FROM monitoring_sessions WHERE project_id = $1
            ORDER BY scheduled_date
            "#
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Fetch monitoring parameters
        let parameters = sqlx::query_as::<_, MonitoringParameter>(
            r#"
            SELECT 
                id, project_id, code, name, description,
                parameter_type,
                data_type, unit, custom_unit,
                min_value, max_value, decimal_places, outlier_threshold_percent,
                sort_order, is_required, is_active, created_at
            FROM monitoring_parameters WHERE project_id = $1 AND is_active = true
            ORDER BY sort_order
            "#
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Fetch aggregated data by parameter and block
        #[derive(Debug, FromRow)]
        struct DataSummaryRow {
            parameter_id: Option<Uuid>,
            parameter_name: Option<String>,
            parameter_code: Option<String>,
            block_id: Option<Uuid>,
            block_code: String,
            is_control: bool,
            session_code: Option<String>,
            days_after_treatment: Option<i32>,
            data_count: Option<i32>,
            avg_value: Option<f64>,
            std_dev: Option<f64>,
            min_value: Option<f64>,
            max_value: Option<f64>,
        }

        let data_summary = sqlx::query_as::<_, DataSummaryRow>(
            r#"
            SELECT 
                mp.id as parameter_id,
                mp.name as parameter_name,
                mp.code as parameter_code,
                eb.id as block_id,
                eb.block_code,
                eb.is_control,
                ms.session_code,
                ms.days_after_treatment,
                COUNT(md.id)::int as data_count,
                AVG(md.numeric_value)::float8 as avg_value,
                STDDEV(md.numeric_value)::float8 as std_dev,
                MIN(md.numeric_value)::float8 as min_value,
                MAX(md.numeric_value)::float8 as max_value
            FROM monitoring_data md
            JOIN experimental_units eu ON md.unit_id = eu.id
            JOIN experimental_blocks eb ON eu.block_id = eb.id
            JOIN monitoring_parameters mp ON md.parameter_id = mp.id
            JOIN monitoring_sessions ms ON md.session_id = ms.id
            WHERE eb.project_id = $1
            GROUP BY mp.id, mp.name, mp.code, eb.id, eb.block_code, eb.is_control, 
                     ms.session_code, ms.days_after_treatment
            ORDER BY mp.code, ms.days_after_treatment, eb.block_code
            "#
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let data_summaries: Vec<DataSummary> = data_summary
            .iter()
            .map(|row| DataSummary {
                parameter_id: row.parameter_id.unwrap_or_default(),
                parameter_name: row.parameter_name.clone().unwrap_or_default(),
                parameter_code: row.parameter_code.clone().unwrap_or_default(),
                block_id: row.block_id.unwrap_or_default(),
                block_code: row.block_code.clone(),
                is_control: row.is_control,
                session_code: row.session_code.clone().unwrap_or_default(),
                days_after_treatment: row.days_after_treatment,
                n: row.data_count.unwrap_or(0) as usize,
                mean: row.avg_value.unwrap_or(0.0),
                std_dev: row.std_dev.unwrap_or(0.0),
                min: row.min_value.unwrap_or(0.0),
                max: row.max_value.unwrap_or(0.0),
            })
            .collect();

        Ok(ProjectReportData {
            project,
            formulas,
            blocks,
            sessions,
            parameters,
            data_summary: data_summaries,
        })
    }

    async fn generate_executive_summary(
        &self,
        data: &ProjectReportData,
        ai_insights: Option<&str>,
    ) -> Result<(ReportContent, Option<String>), AppError> {
        let mut sections = Vec::new();

        // Project overview section
        sections.push(ReportContentSection {
            title: "Project Overview".to_string(),
            content: format!(
                "**Project Code:** {}\n\n**Title:** {}\n\n**Status:** {:?}\n\n**Duration:** {} to {}\n\n**Crop:** {} ({})\n\n**Location:** {}",
                data.project.code,
                data.project.title,
                data.project.status,
                data.project.start_date.map(|d| d.to_string()).unwrap_or("TBD".to_string()),
                data.project.end_date.map(|d| d.to_string()).unwrap_or("TBD".to_string()),
                data.project.crop_type.as_deref().unwrap_or("Not specified"),
                data.project.crop_variety.as_deref().unwrap_or("Unknown"),
                data.project.location_name.as_deref().unwrap_or("Not specified")
            ),
            tables: vec![],
            charts: vec![],
        });

        // Experiment design section
        let treatment_table = TableData {
            title: "Treatment Summary".to_string(),
            headers: vec![
                "Block".to_string(),
                "Treatment".to_string(),
                "Type".to_string(),
                "Status".to_string(),
            ],
            rows: data
                .blocks
                .iter()
                .map(|b| {
                    let formula = data.formulas.iter().find(|f| Some(f.id) == b.formula_id);
                    vec![
                        b.block_code.clone(),
                        formula.map(|f| f.name.clone()).unwrap_or("N/A".to_string()),
                        if b.is_control { "Control" } else { "Treatment" }.to_string(),
                        formula.map(|f| format!("{:?}", f.status)).unwrap_or("N/A".to_string()),
                    ]
                })
                .collect(),
        };

        sections.push(ReportContentSection {
            title: "Experiment Design".to_string(),
            content: format!(
                "**Design:** {:?}\n\n**Replications:** {}\n\n**Treatments:** {}\n\n**Plot Size:** {}",
                data.project.experiment_design.as_ref().map(|d| format!("{:?}", d)).unwrap_or("Not specified".to_string()),
                data.project.replications.unwrap_or(0),
                data.project.treatments_count.unwrap_or(0),
                data.project.plot_size.as_deref().unwrap_or("Not specified")
            ),
            tables: vec![treatment_table],
            charts: vec![],
        });

        // Key findings section
        let mut findings = Vec::new();
        
        // Calculate control vs treatment comparison for main parameters
        for param in &data.parameters {
            let param_data: Vec<_> = data
                .data_summary
                .iter()
                .filter(|d| d.parameter_id == param.id)
                .collect();

            if param_data.is_empty() {
                continue;
            }

            let control_mean = param_data
                .iter()
                .filter(|d| d.is_control)
                .map(|d| d.mean)
                .sum::<f64>()
                / param_data.iter().filter(|d| d.is_control).count().max(1) as f64;

            for treatment in param_data.iter().filter(|d| !d.is_control) {
                let diff = treatment.mean - control_mean;
                let percent = if control_mean != 0.0 {
                    (diff / control_mean) * 100.0
                } else {
                    0.0
                };

                if percent.abs() > 10.0 {
                    findings.push(format!(
                        "- **{}** ({}): {:.1}% {} vs control (Mean: {:.2} vs {:.2})",
                        param.name,
                        treatment.block_code,
                        percent.abs(),
                        if percent > 0.0 { "increase" } else { "decrease" },
                        treatment.mean,
                        control_mean
                    ));
                }
            }
        }

        let findings_text = if findings.is_empty() {
            "No significant differences observed (>10% vs control).".to_string()
        } else {
            findings.join("\n\n")
        };

        sections.push(ReportContentSection {
            title: "Key Findings".to_string(),
            content: findings_text,
            tables: vec![],
            charts: vec![],
        });

        // AI insights section if available
        if let Some(insights) = ai_insights {
            sections.push(ReportContentSection {
                title: "AI-Generated Insights".to_string(),
                content: insights.to_string(),
                tables: vec![],
                charts: vec![],
            });
        }

        // Recommendations section
        sections.push(ReportContentSection {
            title: "Recommendations".to_string(),
            content: "Based on preliminary data analysis, further monitoring and statistical validation are recommended before final conclusions.".to_string(),
            tables: vec![],
            charts: vec![],
        });

        let content = ReportContent {
            title: format!("Executive Summary - {}", data.project.title),
            generated_at: Utc::now(),
            report_type: "Executive Summary".to_string(),
            sections,
        };

        // Generate PDF (simplified - actual PDF generation would be more complex)
        let pdf_path = self.generate_pdf(&content, &data.project.code).await?;

        Ok((content, Some(pdf_path)))
    }

    async fn generate_full_report(
        &self,
        data: &ProjectReportData,
        _requested_sections: &[ReportSection],
        ai_insights: Option<&str>,
    ) -> Result<(ReportContent, Option<String>), AppError> {
        let mut sections = Vec::new();

        // 1. Title Page Info
        sections.push(ReportContentSection {
            title: "Project Information".to_string(),
            content: format!(
                r#"**Project Code:** {}
**Title:** {}
**Organization:** Centra Biotech Indonesia
**Status:** {:?}
**Report Date:** {}

## Background
{}

## Objectives
{}

## Hypothesis
{}"#,
                data.project.code,
                data.project.title,
                data.project.status,
                Utc::now().format("%Y-%m-%d"),
                data.project.background.as_deref().unwrap_or("Not provided"),
                data.project.objectives.as_deref().unwrap_or("Not provided"),
                data.project.hypothesis.as_deref().unwrap_or("Not provided")
            ),
            tables: vec![],
            charts: vec![],
        });

        // 2. Methodology
        sections.push(ReportContentSection {
            title: "Materials and Methods".to_string(),
            content: format!(
                r#"## Experiment Design
- **Design Type:** {:?}
- **Replications:** {}
- **Treatments:** {}
- **Plot Size:** {}

## Location
- **Site:** {}
- **Type:** {}
- **Address:** {}

## Crop Details
- **Crop:** {}
- **Variety:** {}
- **Growth Stage:** {}

## Methodology
{}"#,
                data.project.experiment_design,
                data.project.replications.unwrap_or(0),
                data.project.treatments_count.unwrap_or(0),
                data.project.plot_size.as_deref().unwrap_or("Not specified"),
                data.project.location_name.as_deref().unwrap_or("Not specified"),
                data.project.location_type.as_deref().unwrap_or("Not specified"),
                data.project.location_address.as_deref().unwrap_or("Not specified"),
                data.project.crop_type.as_deref().unwrap_or("Not specified"),
                data.project.crop_variety.as_deref().unwrap_or("Not specified"),
                data.project.growth_stage.as_deref().unwrap_or("Not specified"),
                data.project.methodology.as_deref().unwrap_or("Not provided")
            ),
            tables: vec![],
            charts: vec![],
        });

        // 3. Treatment Details
        let formula_table = TableData {
            title: "Formula Specifications".to_string(),
            headers: vec![
                "Code".to_string(),
                "Name".to_string(),
                "Version".to_string(),
                "Application Rate".to_string(),
                "QC Status".to_string(),
                "Cost/Unit".to_string(),
            ],
            rows: data
                .formulas
                .iter()
                .map(|f| {
                    vec![
                        f.code.clone(),
                        f.name.clone(),
                        f.version.clone(),
                        f.application_rate.clone().unwrap_or("N/A".to_string()),
                        format!("{:?}", f.status),
                        f.cost_per_unit
                            .map(|c| format!("Rp {:.0}", c))
                            .unwrap_or("N/A".to_string()),
                    ]
                })
                .collect(),
        };

        sections.push(ReportContentSection {
            title: "Treatment Specifications".to_string(),
            content: "The following formulations were tested in this experiment:".to_string(),
            tables: vec![formula_table],
            charts: vec![],
        });

        // 4. Results - Data Summary Tables per Parameter
        for param in &data.parameters {
            let param_data: Vec<_> = data
                .data_summary
                .iter()
                .filter(|d| d.parameter_id == param.id)
                .collect();

            if param_data.is_empty() {
                continue;
            }

            // Group by session
            let mut sessions_data: std::collections::HashMap<String, Vec<&DataSummary>> =
                std::collections::HashMap::new();
            for d in &param_data {
                sessions_data
                    .entry(d.session_code.clone())
                    .or_insert_with(Vec::new)
                    .push(d);
            }

            let mut all_tables = Vec::new();
            for (session, session_data) in sessions_data {
                let table = TableData {
                    title: format!("{} - {}", param.name, session),
                    headers: vec![
                        "Treatment".to_string(),
                        "Type".to_string(),
                        "N".to_string(),
                        "Mean".to_string(),
                        "Std Dev".to_string(),
                        "Min".to_string(),
                        "Max".to_string(),
                    ],
                    rows: session_data
                        .iter()
                        .map(|d| {
                            vec![
                                d.block_code.clone(),
                                if d.is_control { "Control" } else { "Treatment" }.to_string(),
                                d.n.to_string(),
                                format!("{:.2}", d.mean),
                                format!("{:.2}", d.std_dev),
                                format!("{:.2}", d.min),
                                format!("{:.2}", d.max),
                            ]
                        })
                        .collect(),
                };
                all_tables.push(table);
            }

            sections.push(ReportContentSection {
                title: format!("Results: {}", param.name),
                content: format!(
                    "Descriptive statistics for {} measurements across treatments.",
                    param.name
                ),
                tables: all_tables,
                charts: vec![],
            });
        }

        // 5. Statistical Analysis
        // Perform ANOVA for each parameter
        let mut anova_content = String::new();
        for param in &data.parameters {
            let param_values: Vec<_> = data
                .data_summary
                .iter()
                .filter(|d| d.parameter_id == param.id)
                .collect();

            if param_values.len() < 2 {
                continue;
            }

            // Group values by block for ANOVA
            let mut groups: std::collections::HashMap<String, Vec<f64>> =
                std::collections::HashMap::new();
            for d in &param_values {
                groups
                    .entry(d.block_code.clone())
                    .or_insert_with(Vec::new)
                    .push(d.mean);
            }

            if groups.len() >= 2 {
                let group_values: Vec<Vec<f64>> = groups.values().cloned().collect();
                let anova = StatisticalAnalysis::one_way_anova(&group_values);

                anova_content.push_str(&format!(
                    r#"### {} ANOVA

| Source | SS | df | MS | F | P-value |
|--------|----|----|----|----|---------|
| Between | {:.4} | {} | {:.4} | {:.4} | {:.4} |
| Within | {:.4} | {} | {:.4} | - | - |
| Total | {:.4} | {} | - | - | - |

**R² = {:.4}** | **Significant at α=0.05: {}** | **Significant at α=0.01: {}**

"#,
                    param.name,
                    anova.source_between.ss,
                    anova.source_between.df,
                    anova.source_between.ms,
                    anova.source_between.f.unwrap_or(0.0),
                    anova.source_between.p.unwrap_or(1.0),
                    anova.source_within.ss,
                    anova.source_within.df,
                    anova.source_within.ms,
                    anova.source_total.ss,
                    anova.source_total.df,
                    anova.r_squared,
                    if anova.is_significant_05 { "Yes" } else { "No" },
                    if anova.is_significant_01 { "Yes" } else { "No" }
                ));
            }
        }

        sections.push(ReportContentSection {
            title: "Statistical Analysis".to_string(),
            content: if anova_content.is_empty() {
                "Insufficient data for statistical analysis.".to_string()
            } else {
                anova_content
            },
            tables: vec![],
            charts: vec![],
        });

        // 6. AI Insights
        if let Some(insights) = ai_insights {
            sections.push(ReportContentSection {
                title: "AI-Powered Analysis".to_string(),
                content: insights.to_string(),
                tables: vec![],
                charts: vec![],
            });
        }

        // 7. Conclusions
        sections.push(ReportContentSection {
            title: "Conclusions and Recommendations".to_string(),
            content: format!(
                r#"## Expected Outcomes
{}

## Conclusions
Based on the data collected and analyzed, the following conclusions can be drawn:
[To be completed by Principal Researcher]

## Recommendations
[To be completed by Principal Researcher]"#,
                data.project
                    .expected_outcomes
                    .as_deref()
                    .unwrap_or("Not specified")
            ),
            tables: vec![],
            charts: vec![],
        });

        let content = ReportContent {
            title: format!("Full Experiment Report - {}", data.project.title),
            generated_at: Utc::now(),
            report_type: "Full Experiment Report".to_string(),
            sections,
        };

        let pdf_path = self.generate_pdf(&content, &data.project.code).await?;

        Ok((content, Some(pdf_path)))
    }

    async fn generate_statistical_report(
        &self,
        data: &ProjectReportData,
    ) -> Result<(ReportContent, Option<String>), AppError> {
        let mut sections = Vec::new();

        sections.push(ReportContentSection {
            title: "Statistical Analysis Report".to_string(),
            content: format!(
                "**Project:** {} - {}\n\n**Analysis Date:** {}",
                data.project.code,
                data.project.title,
                Utc::now().format("%Y-%m-%d %H:%M")
            ),
            tables: vec![],
            charts: vec![],
        });

        // Descriptive statistics for each parameter
        for param in &data.parameters {
            let param_data: Vec<_> = data
                .data_summary
                .iter()
                .filter(|d| d.parameter_id == param.id)
                .collect();

            if param_data.is_empty() {
                continue;
            }

            let desc_table = TableData {
                title: format!("Descriptive Statistics: {}", param.name),
                headers: vec![
                    "Treatment".to_string(),
                    "N".to_string(),
                    "Mean".to_string(),
                    "SD".to_string(),
                    "SE".to_string(),
                    "CV%".to_string(),
                    "Min".to_string(),
                    "Max".to_string(),
                ],
                rows: param_data
                    .iter()
                    .map(|d| {
                        let se = if d.n > 0 { d.std_dev / (d.n as f64).sqrt() } else { 0.0 };
                        let cv = if d.mean != 0.0 { (d.std_dev / d.mean) * 100.0 } else { 0.0 };
                        vec![
                            format!("{}{}", d.block_code, if d.is_control { " (C)" } else { "" }),
                            d.n.to_string(),
                            format!("{:.3}", d.mean),
                            format!("{:.3}", d.std_dev),
                            format!("{:.3}", se),
                            format!("{:.1}", cv),
                            format!("{:.3}", d.min),
                            format!("{:.3}", d.max),
                        ]
                    })
                    .collect(),
            };

            sections.push(ReportContentSection {
                title: format!("Parameter: {}", param.name),
                content: String::new(),
                tables: vec![desc_table],
                charts: vec![],
            });
        }

        let content = ReportContent {
            title: format!("Statistical Report - {}", data.project.code),
            generated_at: Utc::now(),
            report_type: "Statistical Analysis".to_string(),
            sections,
        };

        Ok((content, None))
    }

    async fn generate_qc_report(
        &self,
        data: &ProjectReportData,
    ) -> Result<(ReportContent, Option<String>), AppError> {
        let mut sections = Vec::new();

        sections.push(ReportContentSection {
            title: "QC Gate Report".to_string(),
            content: format!(
                "**Project:** {}\n\n**Report Date:** {}",
                data.project.code,
                Utc::now().format("%Y-%m-%d")
            ),
            tables: vec![],
            charts: vec![],
        });

        // Formula QC status table
        let qc_table = TableData {
            title: "Formula QC Status".to_string(),
            headers: vec![
                "Formula".to_string(),
                "Version".to_string(),
                "Status".to_string(),
                "Approved By".to_string(),
                "Approved At".to_string(),
                "Notes".to_string(),
            ],
            rows: data
                .formulas
                .iter()
                .map(|f| {
                    vec![
                        format!("{} - {}", f.code, f.name),
                        f.version.clone(),
                        format!("{:?}", f.status),
                        f.qc_approved_by.map(|id| id.to_string()).unwrap_or("-".to_string()),
                        f.qc_approved_at.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or("-".to_string()),
                        f.qc_notes.clone().unwrap_or("-".to_string()),
                    ]
                })
                .collect(),
        };

        sections.push(ReportContentSection {
            title: "Formula QC Summary".to_string(),
            content: "Overview of all formula QC statuses for this project.".to_string(),
            tables: vec![qc_table],
            charts: vec![],
        });

        let content = ReportContent {
            title: format!("QC Report - {}", data.project.code),
            generated_at: Utc::now(),
            report_type: "QC Report".to_string(),
            sections,
        };

        Ok((content, None))
    }

    async fn generate_field_progress(
        &self,
        data: &ProjectReportData,
    ) -> Result<(ReportContent, Option<String>), AppError> {
        let mut sections = Vec::new();

        // Progress overview
        let completed_sessions = data.sessions.iter().filter(|s| s.is_completed).count();
        let total_sessions = data.sessions.len();
        let progress_percent = if total_sessions > 0 {
            (completed_sessions as f64 / total_sessions as f64) * 100.0
        } else {
            0.0
        };

        sections.push(ReportContentSection {
            title: "Field Monitoring Progress".to_string(),
            content: format!(
                r#"**Project:** {} - {}

## Progress Summary
- **Total Monitoring Sessions:** {}
- **Completed:** {}
- **Remaining:** {}
- **Progress:** {:.1}%"#,
                data.project.code,
                data.project.title,
                total_sessions,
                completed_sessions,
                total_sessions - completed_sessions,
                progress_percent
            ),
            tables: vec![],
            charts: vec![],
        });

        // Session schedule table
        let session_table = TableData {
            title: "Monitoring Schedule".to_string(),
            headers: vec![
                "Session".to_string(),
                "Scheduled".to_string(),
                "Actual".to_string(),
                "DAT".to_string(),
                "Status".to_string(),
                "Weather".to_string(),
            ],
            rows: data
                .sessions
                .iter()
                .map(|s| {
                    vec![
                        s.session_code.clone(),
                        s.scheduled_date.to_string(),
                        s.actual_date.map(|d| d.to_string()).unwrap_or("-".to_string()),
                        s.days_after_treatment.map(|d| d.to_string()).unwrap_or("-".to_string()),
                        if s.is_completed { "✓ Complete" } else { "Pending" }.to_string(),
                        s.weather_conditions.clone().unwrap_or("-".to_string()),
                    ]
                })
                .collect(),
        };

        sections.push(ReportContentSection {
            title: "Session Details".to_string(),
            content: String::new(),
            tables: vec![session_table],
            charts: vec![],
        });

        let content = ReportContent {
            title: format!("Field Progress - {}", data.project.code),
            generated_at: Utc::now(),
            report_type: "Field Progress".to_string(),
            sections,
        };

        Ok((content, None))
    }

    async fn generate_pdf(&self, content: &ReportContent, project_code: &str) -> Result<String, AppError> {
        // Create reports directory if it doesn't exist
        let reports_dir = Path::new(&self.storage_path);
        std::fs::create_dir_all(reports_dir)
            .map_err(|e| AppError::FileError(format!("Failed to create reports directory: {}", e)))?;

        let filename = format!(
            "{}_{}.pdf",
            project_code,
            Utc::now().format("%Y%m%d_%H%M%S")
        );
        let filepath = reports_dir.join(&filename);

        // Create PDF document
        let (doc, page1, layer1) = PdfDocument::new(
            &content.title,
            Mm(210.0),
            Mm(297.0),
            "Layer 1",
        );

        let current_layer = doc.get_page(page1).get_layer(layer1);

        // Load font (using built-in fonts for simplicity)
        let font = doc
            .add_builtin_font(BuiltinFont::Helvetica)
            .map_err(|e| AppError::InternalError(format!("Failed to load font: {}", e)))?;
        let font_bold = doc
            .add_builtin_font(BuiltinFont::HelveticaBold)
            .map_err(|e| AppError::InternalError(format!("Failed to load bold font: {}", e)))?;

        // Title
        current_layer.use_text(
            &content.title,
            18.0,
            Mm(20.0),
            Mm(270.0),
            &font_bold,
        );

        // Subtitle with date
        current_layer.use_text(
            &format!("Generated: {}", content.generated_at.format("%Y-%m-%d %H:%M")),
            10.0,
            Mm(20.0),
            Mm(260.0),
            &font,
        );

        // Note: Full PDF generation with tables, charts, and multi-page support
        // would require more complex implementation. This is a basic structure.

        // Save PDF
        doc.save(&mut BufWriter::new(
            File::create(&filepath)
                .map_err(|e| AppError::FileError(format!("Failed to create PDF file: {}", e)))?,
        ))
        .map_err(|e| AppError::InternalError(format!("Failed to save PDF: {}", e)))?;

        Ok(filepath.to_string_lossy().to_string())
    }
}

// ==============================================================================
// REPORT DATA STRUCTURES
// ==============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReportType {
    ExecutiveSummary,
    FullExperiment,
    StatisticalAnalysis,
    QCReport,
    FieldProgress,
}

impl ToString for ReportType {
    fn to_string(&self) -> String {
        match self {
            ReportType::ExecutiveSummary => "Executive Summary".to_string(),
            ReportType::FullExperiment => "Full Experiment Report".to_string(),
            ReportType::StatisticalAnalysis => "Statistical Analysis".to_string(),
            ReportType::QCReport => "QC Report".to_string(),
            ReportType::FieldProgress => "Field Progress".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReportSection {
    Overview,
    Methodology,
    Treatments,
    Results,
    Statistics,
    AIInsights,
    Conclusions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportContent {
    pub title: String,
    pub generated_at: chrono::DateTime<Utc>,
    pub report_type: String,
    pub sections: Vec<ReportContentSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportContentSection {
    pub title: String,
    pub content: String,
    pub tables: Vec<TableData>,
    pub charts: Vec<ChartData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableData {
    pub title: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartData {
    pub title: String,
    pub chart_type: String, // bar, line, scatter
    pub data: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ProjectReportData {
    pub project: Project,
    pub formulas: Vec<Formula>,
    pub blocks: Vec<ExperimentalBlock>,
    pub sessions: Vec<MonitoringSession>,
    pub parameters: Vec<MonitoringParameter>,
    pub data_summary: Vec<DataSummary>,
}

#[derive(Debug, Clone)]
pub struct DataSummary {
    pub parameter_id: Uuid,
    pub parameter_name: String,
    pub parameter_code: String,
    pub block_id: Uuid,
    pub block_code: String,
    pub is_control: bool,
    pub session_code: String,
    pub days_after_treatment: Option<i32>,
    pub n: usize,
    pub mean: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedReport {
    pub id: Uuid,
    pub project_id: Uuid,
    pub report_type: ReportType,
    pub title: String,
    pub content: ReportContent,
    pub pdf_path: Option<String>,
    pub generated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GeneratedReportRecord {
    pub id: Uuid,
    pub project_id: Uuid,
    pub report_type: Option<String>,
    pub content_json: Option<serde_json::Value>,
    pub pdf_path: Option<String>,
    pub generated_at: Option<chrono::DateTime<Utc>>,
    pub generated_by: Option<Uuid>,
    pub is_latest: bool,
}
