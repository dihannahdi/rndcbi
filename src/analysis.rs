use crate::config::Settings;
use crate::errors::AppError;
use crate::models::*;
use crate::services::ProjectService;
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
    },
    Client,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use statrs::distribution::{ContinuousCDF, FisherSnedecor, StudentsT};
use statrs::statistics::{Data, Distribution, Max, Min, OrderStatistics};
use std::collections::HashMap;
use uuid::Uuid;

// ==============================================================================
// STATISTICAL ANALYSIS SERVICE
// ==============================================================================

pub struct StatisticalAnalysis;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptiveStats {
    pub n: usize,
    pub mean: f64,
    pub std_dev: f64,
    pub std_error: f64,
    pub min: f64,
    pub max: f64,
    pub median: f64,
    pub q1: f64,
    pub q3: f64,
    pub variance: f64,
    pub cv: f64, // Coefficient of variation
}

impl StatisticalAnalysis {
    pub fn descriptive(values: &[f64]) -> DescriptiveStats {
        let n = values.len();
        if n == 0 {
            return DescriptiveStats {
                n: 0,
                mean: 0.0,
                std_dev: 0.0,
                std_error: 0.0,
                min: 0.0,
                max: 0.0,
                median: 0.0,
                q1: 0.0,
                q3: 0.0,
                variance: 0.0,
                cv: 0.0,
            };
        }

        let mut data = Data::new(values.to_vec());
        let mean = data.mean().unwrap_or(0.0);
        let variance = data.variance().unwrap_or(0.0);
        let std_dev = variance.sqrt();
        let std_error = std_dev / (n as f64).sqrt();

        DescriptiveStats {
            n,
            mean,
            std_dev,
            std_error,
            min: data.min(),
            max: data.max(),
            median: data.median(),
            q1: data.lower_quartile(),
            q3: data.upper_quartile(),
            variance,
            cv: if mean != 0.0 { (std_dev / mean) * 100.0 } else { 0.0 },
        }
    }

    /// One-way ANOVA for treatment comparison
    pub fn one_way_anova(groups: &[Vec<f64>]) -> AnovaResult {
        let k = groups.len(); // Number of groups
        let mut n_total = 0;
        let mut grand_sum = 0.0;
        let mut group_means = Vec::new();
        let mut group_sizes = Vec::new();

        // Calculate group means and grand mean
        for group in groups {
            let n = group.len();
            let sum: f64 = group.iter().sum();
            let mean = if n > 0 { sum / n as f64 } else { 0.0 };
            group_means.push(mean);
            group_sizes.push(n);
            grand_sum += sum;
            n_total += n;
        }

        let grand_mean = if n_total > 0 { grand_sum / n_total as f64 } else { 0.0 };

        // Calculate Sum of Squares Between (SSB)
        let mut ssb = 0.0;
        for (i, group) in groups.iter().enumerate() {
            let n = group.len() as f64;
            ssb += n * (group_means[i] - grand_mean).powi(2);
        }

        // Calculate Sum of Squares Within (SSW)
        let mut ssw = 0.0;
        for (i, group) in groups.iter().enumerate() {
            for &value in group {
                ssw += (value - group_means[i]).powi(2);
            }
        }

        // Calculate Total Sum of Squares (SST)
        let sst = ssb + ssw;

        // Degrees of freedom
        let df_between = (k - 1) as f64;
        let df_within = (n_total - k) as f64;
        let df_total = (n_total - 1) as f64;

        // Mean squares
        let msb = if df_between > 0.0 { ssb / df_between } else { 0.0 };
        let msw = if df_within > 0.0 { ssw / df_within } else { 0.0 };

        // F-statistic
        let f_statistic = if msw > 0.0 { msb / msw } else { 0.0 };

        // P-value from F-distribution
        let p_value = if df_between > 0.0 && df_within > 0.0 {
            let f_dist = FisherSnedecor::new(df_between, df_within).unwrap();
            1.0 - f_dist.cdf(f_statistic)
        } else {
            1.0
        };

        // Coefficient of determination (R²)
        let r_squared = if sst > 0.0 { ssb / sst } else { 0.0 };

        AnovaResult {
            source_between: AnovaSource {
                ss: ssb,
                df: df_between as i32,
                ms: msb,
                f: Some(f_statistic),
                p: Some(p_value),
            },
            source_within: AnovaSource {
                ss: ssw,
                df: df_within as i32,
                ms: msw,
                f: None,
                p: None,
            },
            source_total: AnovaSource {
                ss: sst,
                df: df_total as i32,
                ms: 0.0,
                f: None,
                p: None,
            },
            r_squared,
            is_significant_05: p_value < 0.05,
            is_significant_01: p_value < 0.01,
            group_means,
            group_sizes,
            grand_mean,
        }
    }

    /// Two-way ANOVA for factorial experiments (simplified)
    pub fn two_way_anova(
        data: &HashMap<(usize, usize), Vec<f64>>,
        factor_a_levels: usize,
        factor_b_levels: usize,
    ) -> TwoWayAnovaResult {
        let mut grand_sum = 0.0;
        let mut n_total = 0;
        let mut factor_a_sums: Vec<f64> = vec![0.0; factor_a_levels];
        let mut factor_b_sums: Vec<f64> = vec![0.0; factor_b_levels];
        let mut factor_a_n: Vec<usize> = vec![0; factor_a_levels];
        let mut factor_b_n: Vec<usize> = vec![0; factor_b_levels];
        let mut cell_means: HashMap<(usize, usize), f64> = HashMap::new();

        // Calculate sums
        for ((a, b), values) in data {
            let sum: f64 = values.iter().sum();
            let n = values.len();
            let mean = if n > 0 { sum / n as f64 } else { 0.0 };
            
            cell_means.insert((*a, *b), mean);
            grand_sum += sum;
            n_total += n;
            factor_a_sums[*a] += sum;
            factor_b_sums[*b] += sum;
            factor_a_n[*a] += n;
            factor_b_n[*b] += n;
        }

        let grand_mean = if n_total > 0 { grand_sum / n_total as f64 } else { 0.0 };

        // Calculate factor means
        let factor_a_means: Vec<f64> = factor_a_sums
            .iter()
            .zip(factor_a_n.iter())
            .map(|(sum, n)| if *n > 0 { sum / *n as f64 } else { 0.0 })
            .collect();

        let factor_b_means: Vec<f64> = factor_b_sums
            .iter()
            .zip(factor_b_n.iter())
            .map(|(sum, n)| if *n > 0 { sum / *n as f64 } else { 0.0 })
            .collect();

        // Calculate SS for Factor A
        let n_per_a: usize = factor_a_n.iter().sum::<usize>() / factor_a_levels;
        let ss_a: f64 = factor_a_means
            .iter()
            .map(|mean| n_per_a as f64 * (mean - grand_mean).powi(2))
            .sum();

        // Calculate SS for Factor B
        let n_per_b: usize = factor_b_n.iter().sum::<usize>() / factor_b_levels;
        let ss_b: f64 = factor_b_means
            .iter()
            .map(|mean| n_per_b as f64 * (mean - grand_mean).powi(2))
            .sum();

        // Calculate Total SS and Error SS
        let mut ss_total = 0.0;
        for ((_, _), values) in data {
            for value in values {
                ss_total += (value - grand_mean).powi(2);
            }
        }

        // Calculate interaction SS (simplified - cell means method)
        let r = data.values().next().map(|v| v.len()).unwrap_or(1); // replications
        let mut ss_ab = 0.0;
        for a in 0..factor_a_levels {
            for b in 0..factor_b_levels {
                if let Some(&cell_mean) = cell_means.get(&(a, b)) {
                    let expected = factor_a_means[a] + factor_b_means[b] - grand_mean;
                    ss_ab += r as f64 * (cell_mean - expected).powi(2);
                }
            }
        }

        let ss_error = ss_total - ss_a - ss_b - ss_ab;

        // Degrees of freedom
        let df_a = (factor_a_levels - 1) as f64;
        let df_b = (factor_b_levels - 1) as f64;
        let df_ab = df_a * df_b;
        let df_error = (n_total - factor_a_levels * factor_b_levels) as f64;
        let df_total = (n_total - 1) as f64;

        // Mean squares
        let ms_a = if df_a > 0.0 { ss_a / df_a } else { 0.0 };
        let ms_b = if df_b > 0.0 { ss_b / df_b } else { 0.0 };
        let ms_ab = if df_ab > 0.0 { ss_ab / df_ab } else { 0.0 };
        let ms_error = if df_error > 0.0 { ss_error / df_error } else { 0.0 };

        // F-statistics
        let f_a = if ms_error > 0.0 { ms_a / ms_error } else { 0.0 };
        let f_b = if ms_error > 0.0 { ms_b / ms_error } else { 0.0 };
        let f_ab = if ms_error > 0.0 { ms_ab / ms_error } else { 0.0 };

        // P-values
        let p_a = Self::f_p_value(f_a, df_a, df_error);
        let p_b = Self::f_p_value(f_b, df_b, df_error);
        let p_ab = Self::f_p_value(f_ab, df_ab, df_error);

        TwoWayAnovaResult {
            factor_a: AnovaSource {
                ss: ss_a,
                df: df_a as i32,
                ms: ms_a,
                f: Some(f_a),
                p: Some(p_a),
            },
            factor_b: AnovaSource {
                ss: ss_b,
                df: df_b as i32,
                ms: ms_b,
                f: Some(f_b),
                p: Some(p_b),
            },
            interaction: AnovaSource {
                ss: ss_ab,
                df: df_ab as i32,
                ms: ms_ab,
                f: Some(f_ab),
                p: Some(p_ab),
            },
            error: AnovaSource {
                ss: ss_error,
                df: df_error as i32,
                ms: ms_error,
                f: None,
                p: None,
            },
            total: AnovaSource {
                ss: ss_total,
                df: df_total as i32,
                ms: 0.0,
                f: None,
                p: None,
            },
            factor_a_means,
            factor_b_means,
            cell_means: cell_means.into_iter().collect(),
        }
    }

    fn f_p_value(f: f64, df1: f64, df2: f64) -> f64 {
        if df1 > 0.0 && df2 > 0.0 {
            if let Ok(f_dist) = FisherSnedecor::new(df1, df2) {
                return 1.0 - f_dist.cdf(f);
            }
        }
        1.0
    }

    /// T-test for comparing two groups
    pub fn t_test(group1: &[f64], group2: &[f64], paired: bool) -> TTestResult {
        if paired && group1.len() != group2.len() {
            return TTestResult {
                t_statistic: 0.0,
                p_value: 1.0,
                df: 0.0,
                mean_difference: 0.0,
                ci_lower: 0.0,
                ci_upper: 0.0,
                is_significant: false,
            };
        }

        let (t_stat, df, mean_diff) = if paired {
            // Paired t-test
            let differences: Vec<f64> = group1
                .iter()
                .zip(group2.iter())
                .map(|(a, b)| a - b)
                .collect();
            let n = differences.len() as f64;
            let mean_d = differences.iter().sum::<f64>() / n;
            let var_d: f64 = differences
                .iter()
                .map(|d| (d - mean_d).powi(2))
                .sum::<f64>()
                / (n - 1.0);
            let se_d = (var_d / n).sqrt();
            let t = if se_d > 0.0 { mean_d / se_d } else { 0.0 };
            (t, n - 1.0, mean_d)
        } else {
            // Independent samples t-test (Welch's)
            let n1 = group1.len() as f64;
            let n2 = group2.len() as f64;
            let mean1 = group1.iter().sum::<f64>() / n1;
            let mean2 = group2.iter().sum::<f64>() / n2;
            let var1: f64 = group1.iter().map(|x| (x - mean1).powi(2)).sum::<f64>() / (n1 - 1.0);
            let var2: f64 = group2.iter().map(|x| (x - mean2).powi(2)).sum::<f64>() / (n2 - 1.0);

            let se = ((var1 / n1) + (var2 / n2)).sqrt();
            let t = if se > 0.0 { (mean1 - mean2) / se } else { 0.0 };

            // Welch-Satterthwaite degrees of freedom
            let num = ((var1 / n1) + (var2 / n2)).powi(2);
            let denom = ((var1 / n1).powi(2) / (n1 - 1.0)) + ((var2 / n2).powi(2) / (n2 - 1.0));
            let df = if denom > 0.0 { num / denom } else { n1 + n2 - 2.0 };

            (t, df, mean1 - mean2)
        };

        // P-value (two-tailed)
        let p_value = if df > 0.0 {
            if let Ok(t_dist) = StudentsT::new(0.0, 1.0, df) {
                2.0 * (1.0 - t_dist.cdf(t_stat.abs()))
            } else {
                1.0
            }
        } else {
            1.0
        };

        // 95% CI
        let t_crit = if df > 0.0 {
            StudentsT::new(0.0, 1.0, df).map(|d| d.inverse_cdf(0.975)).unwrap_or(1.96)
        } else {
            1.96
        };

        let se = if t_stat != 0.0 { mean_diff / t_stat } else { 0.0 };
        let ci_lower = mean_diff - t_crit * se;
        let ci_upper = mean_diff + t_crit * se;

        TTestResult {
            t_statistic: t_stat,
            p_value,
            df,
            mean_difference: mean_diff,
            ci_lower,
            ci_upper,
            is_significant: p_value < 0.05,
        }
    }

    /// LSD (Least Significant Difference) post-hoc test
    pub fn lsd_test(groups: &[Vec<f64>], mse: f64, df_error: f64) -> Vec<LSDComparison> {
        let mut comparisons = Vec::new();
        let k = groups.len();

        for i in 0..k {
            for j in (i + 1)..k {
                let n1 = groups[i].len() as f64;
                let n2 = groups[j].len() as f64;
                let mean1 = groups[i].iter().sum::<f64>() / n1;
                let mean2 = groups[j].iter().sum::<f64>() / n2;
                let mean_diff = mean1 - mean2;

                let se = (mse * (1.0 / n1 + 1.0 / n2)).sqrt();
                let t = if se > 0.0 { mean_diff / se } else { 0.0 };

                let p_value = if df_error > 0.0 {
                    StudentsT::new(0.0, 1.0, df_error)
                        .map(|d| 2.0 * (1.0 - d.cdf(t.abs())))
                        .unwrap_or(1.0)
                } else {
                    1.0
                };

                let t_crit = StudentsT::new(0.0, 1.0, df_error)
                    .map(|d| d.inverse_cdf(0.975))
                    .unwrap_or(1.96);
                let lsd = t_crit * se;

                comparisons.push(LSDComparison {
                    group_i: i,
                    group_j: j,
                    mean_difference: mean_diff,
                    std_error: se,
                    t_statistic: t,
                    p_value,
                    lsd,
                    is_significant: mean_diff.abs() > lsd,
                });
            }
        }

        comparisons
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnovaResult {
    pub source_between: AnovaSource,
    pub source_within: AnovaSource,
    pub source_total: AnovaSource,
    pub r_squared: f64,
    pub is_significant_05: bool,
    pub is_significant_01: bool,
    pub group_means: Vec<f64>,
    pub group_sizes: Vec<usize>,
    pub grand_mean: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwoWayAnovaResult {
    pub factor_a: AnovaSource,
    pub factor_b: AnovaSource,
    pub interaction: AnovaSource,
    pub error: AnovaSource,
    pub total: AnovaSource,
    pub factor_a_means: Vec<f64>,
    pub factor_b_means: Vec<f64>,
    pub cell_means: Vec<((usize, usize), f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnovaSource {
    pub ss: f64,
    pub df: i32,
    pub ms: f64,
    pub f: Option<f64>,
    pub p: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TTestResult {
    pub t_statistic: f64,
    pub p_value: f64,
    pub df: f64,
    pub mean_difference: f64,
    pub ci_lower: f64,
    pub ci_upper: f64,
    pub is_significant: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LSDComparison {
    pub group_i: usize,
    pub group_j: usize,
    pub mean_difference: f64,
    pub std_error: f64,
    pub t_statistic: f64,
    pub p_value: f64,
    pub lsd: f64,
    pub is_significant: bool,
}

// ==============================================================================
// AI ANALYSIS SERVICE
// ==============================================================================

pub struct AIAnalysisService {
    client: Client<OpenAIConfig>,
    model: String,
}

impl AIAnalysisService {
    pub fn new(settings: &Settings) -> Option<Self> {
        if settings.openai.api_key.is_empty() {
            return None;
        }

        let config = OpenAIConfig::new().with_api_key(&settings.openai.api_key);
        let client = Client::with_config(config);

        Some(Self {
            client,
            model: settings.openai.model.clone(),
        })
    }

    pub async fn analyze_experiment_data(
        &self,
        pool: &PgPool,
        project_id: Uuid,
        analysis_type: &str,
    ) -> Result<AIAnalysisResult, AppError> {
        // Fetch project data
        let project = ProjectService::get_by_id(pool, project_id).await?;

        // Fetch monitoring data summary
        let data_summary = self.fetch_monitoring_summary(pool, project_id).await?;

        // Build prompt based on analysis type
        let prompt = self.build_analysis_prompt(&project, &data_summary, analysis_type);

        // Call OpenAI API
        let response = self.call_openai(&prompt).await?;

        Ok(AIAnalysisResult {
            analysis_type: analysis_type.to_string(),
            insights: response.insights,
            recommendations: response.recommendations,
            statistical_interpretation: response.statistical_interpretation,
            confidence_level: response.confidence_level,
            limitations: response.limitations,
            next_steps: response.next_steps,
        })
    }

    async fn fetch_monitoring_summary(
        &self,
        pool: &PgPool,
        project_id: Uuid,
    ) -> Result<MonitoringSummary, AppError> {
        // Fetch parameter statistics
        let stats: Vec<(Option<String>, Option<String>, String, bool, Option<i64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>)> = sqlx::query_as(
            r#"
            SELECT 
                mp.name as parameter_name,
                mp.code as parameter_code,
                eb.block_code,
                eb.is_control,
                COUNT(md.id) as data_count,
                AVG(md.numeric_value)::float8 as avg_value,
                STDDEV(md.numeric_value)::float8 as std_dev,
                MIN(md.numeric_value)::float8 as min_value,
                MAX(md.numeric_value)::float8 as max_value
            FROM monitoring_data md
            JOIN experimental_units eu ON md.unit_id = eu.id
            JOIN experimental_blocks eb ON eu.block_id = eb.id
            JOIN monitoring_parameters mp ON md.parameter_id = mp.id
            WHERE eb.project_id = $1
            GROUP BY mp.name, mp.code, eb.block_code, eb.is_control
            ORDER BY mp.code, eb.block_code
            "#
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let treatment_stats: Vec<TreatmentStats> = stats
            .iter()
            .map(|row| TreatmentStats {
                parameter_name: row.0.clone().unwrap_or_default(),
                parameter_code: row.1.clone().unwrap_or_default(),
                block_code: row.2.clone(),
                is_control: row.3,
                n: row.4.unwrap_or(0) as usize,
                mean: row.5.unwrap_or(0.0),
                std_dev: row.6.unwrap_or(0.0),
                min: row.7.unwrap_or(0.0),
                max: row.8.unwrap_or(0.0),
            })
            .collect();

        Ok(MonitoringSummary { treatment_stats })
    }

    fn build_analysis_prompt(
        &self,
        project: &Project,
        summary: &MonitoringSummary,
        analysis_type: &str,
    ) -> String {
        let stats_text = summary
            .treatment_stats
            .iter()
            .map(|s| {
                format!(
                    "- {} ({}): Block {}{}, n={}, Mean={:.2}±{:.2}, Range=[{:.2}-{:.2}]",
                    s.parameter_name,
                    s.parameter_code,
                    s.block_code,
                    if s.is_control { " (CONTROL)" } else { "" },
                    s.n,
                    s.mean,
                    s.std_dev,
                    s.min,
                    s.max
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"You are an expert agricultural research scientist analyzing experiment data for Centra Biotech Indonesia R&D.

PROJECT: {}
TITLE: {}
HYPOTHESIS: {}
METHODOLOGY: {}
CROP: {} ({})
EXPERIMENT DESIGN: {:?}
REPLICATIONS: {}

DATA SUMMARY:
{}

ANALYSIS TYPE: {}

Please provide:
1. **Key Insights**: What do the data tell us? Are there significant patterns?
2. **Statistical Interpretation**: Interpret the means, variability, and any comparisons
3. **Treatment Effects**: Which treatments show promise compared to control?
4. **Recommendations**: Practical recommendations for R&D team
5. **Limitations**: Any data quality issues or limitations to consider
6. **Next Steps**: Suggested next steps for the experiment

Respond in JSON format with keys: insights, statistical_interpretation, recommendations, confidence_level (high/medium/low), limitations, next_steps"#,
            project.code,
            project.title,
            project.hypothesis.as_deref().unwrap_or("Not specified"),
            project.methodology.as_deref().unwrap_or("Not specified"),
            project.crop_type.as_deref().unwrap_or("Not specified"),
            project.crop_variety.as_deref().unwrap_or("Unknown variety"),
            project.experiment_design,
            project.replications.unwrap_or(3),
            stats_text,
            analysis_type
        )
    }

    async fn call_openai(&self, prompt: &str) -> Result<AIResponse, AppError> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages([
                ChatCompletionRequestMessage::System(
                    ChatCompletionRequestSystemMessageArgs::default()
                        .content("You are an expert agricultural data scientist specializing in biostimulant and fertilizer research. Provide analysis in clear, scientific language suitable for R&D reports.")
                        .build()
                        .map_err(|e| AppError::AIError(e.to_string()))?
                ),
                ChatCompletionRequestMessage::User(
                    ChatCompletionRequestUserMessageArgs::default()
                        .content(prompt)
                        .build()
                        .map_err(|e| AppError::AIError(e.to_string()))?
                ),
            ])
            .temperature(0.3)
            .max_tokens(2000_u32)
            .build()
            .map_err(|e| AppError::AIError(e.to_string()))?;

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| AppError::AIError(e.to_string()))?;

        let content = response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| AppError::AIError("No response from AI".to_string()))?;

        // Parse JSON response
        let parsed: AIResponse = serde_json::from_str(&content).unwrap_or_else(|_| AIResponse {
            insights: content.clone(),
            statistical_interpretation: String::new(),
            recommendations: String::new(),
            confidence_level: "medium".to_string(),
            limitations: String::new(),
            next_steps: String::new(),
        });

        Ok(parsed)
    }

    pub async fn generate_report_insights(
        &self,
        pool: &PgPool,
        project_id: Uuid,
    ) -> Result<String, AppError> {
        let project = ProjectService::get_by_id(pool, project_id).await?;
        let summary = self.fetch_monitoring_summary(pool, project_id).await?;

        let prompt = format!(
            r#"Generate a professional executive summary for the R&D experiment report:

PROJECT: {} - {}
STATUS: {:?}
CROP: {} ({})

DATA COLLECTED:
{}

Write a 2-3 paragraph executive summary suitable for management review, highlighting:
1. Key findings
2. Treatment efficacy
3. Business implications
4. Recommendations"#,
            project.code,
            project.title,
            project.status,
            project.crop_type.as_deref().unwrap_or("Unknown"),
            project.crop_variety.as_deref().unwrap_or("Unknown"),
            summary
                .treatment_stats
                .iter()
                .take(10)
                .map(|s| format!(
                    "  {} ({}): Mean={:.2}",
                    s.parameter_name, s.block_code, s.mean
                ))
                .collect::<Vec<_>>()
                .join("\n")
        );

        let response = self.call_openai(&prompt).await?;
        Ok(response.insights)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringSummary {
    pub treatment_stats: Vec<TreatmentStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreatmentStats {
    pub parameter_name: String,
    pub parameter_code: String,
    pub block_code: String,
    pub is_control: bool,
    pub n: usize,
    pub mean: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIResponse {
    pub insights: String,
    pub statistical_interpretation: String,
    pub recommendations: String,
    pub confidence_level: String,
    pub limitations: String,
    pub next_steps: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIAnalysisResult {
    pub analysis_type: String,
    pub insights: String,
    pub recommendations: String,
    pub statistical_interpretation: String,
    pub confidence_level: String,
    pub limitations: String,
    pub next_steps: String,
}

// ==============================================================================
// COST-BENEFIT ANALYSIS
// ==============================================================================

pub struct CostBenefitAnalysis;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBenefitResult {
    pub treatment_costs: Vec<TreatmentCost>,
    pub yield_comparison: Vec<YieldComparison>,
    pub roi_analysis: Vec<ROIAnalysis>,
    pub break_even_analysis: BreakEvenAnalysis,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreatmentCost {
    pub treatment_name: String,
    pub formula_cost: Decimal,
    pub application_cost: Decimal,
    pub total_cost_per_ha: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YieldComparison {
    pub treatment_name: String,
    pub yield_per_ha: f64,
    pub yield_increase_vs_control: f64,
    pub yield_increase_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIAnalysis {
    pub treatment_name: String,
    pub additional_cost: Decimal,
    pub additional_revenue: Decimal,
    pub net_benefit: Decimal,
    pub roi_percent: f64,
    pub is_profitable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakEvenAnalysis {
    pub break_even_yield_increase: f64,
    pub break_even_price: Decimal,
    pub current_margin_of_safety: f64,
}

impl CostBenefitAnalysis {
    pub async fn analyze(
        pool: &PgPool,
        project_id: Uuid,
        crop_price_per_kg: Decimal,
    ) -> Result<CostBenefitResult, AppError> {
        // Fetch formula costs by block
        #[derive(sqlx::FromRow)]
        struct FormulaCostRow {
            block_code: String,
            is_control: bool,
            formula_name: Option<String>,
            calculated_cost: Option<Decimal>,
            application_rate: Option<String>,
        }

        let formula_costs: Vec<FormulaCostRow> = sqlx::query_as(
            r#"
            SELECT 
                eb.block_code,
                eb.is_control,
                f.name as formula_name,
                f.calculated_cost,
                f.application_rate
            FROM experimental_blocks eb
            LEFT JOIN formulas f ON eb.formula_id = f.id
            WHERE eb.project_id = $1
            ORDER BY eb.block_code
            "#
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Fetch yield data (assuming 'yield' parameter exists)
        #[derive(sqlx::FromRow)]
        struct YieldRow {
            block_code: String,
            is_control: bool,
            avg_yield: Option<f64>,
        }

        let yields: Vec<YieldRow> = sqlx::query_as(
            r#"
            SELECT 
                eb.block_code,
                eb.is_control,
                AVG(md.numeric_value)::float8 as avg_yield
            FROM monitoring_data md
            JOIN experimental_units eu ON md.unit_id = eu.id
            JOIN experimental_blocks eb ON eu.block_id = eb.id
            JOIN monitoring_parameters mp ON md.parameter_id = mp.id
            WHERE eb.project_id = $1 
            AND mp.parameter_type = 'yield'
            GROUP BY eb.block_code, eb.is_control
            "#
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Find control yield
        let control_yield = yields
            .iter()
            .find(|y| y.is_control)
            .and_then(|y| y.avg_yield)
            .unwrap_or(0.0);

        // Calculate treatment costs
        let treatment_costs: Vec<TreatmentCost> = formula_costs
            .iter()
            .map(|fc| {
                let formula_cost = fc.calculated_cost.unwrap_or_default();
                // Assuming application cost is 20% of formula cost
                let application_cost = formula_cost * Decimal::from_str_exact("0.2").unwrap_or_default();
                TreatmentCost {
                    treatment_name: fc.formula_name.clone().unwrap_or_else(|| fc.block_code.clone()),
                    formula_cost,
                    application_cost,
                    total_cost_per_ha: formula_cost + application_cost,
                }
            })
            .collect();

        // Calculate yield comparisons
        let yield_comparison: Vec<YieldComparison> = yields
            .iter()
            .map(|y| {
                let yield_val = y.avg_yield.unwrap_or(0.0);
                let increase = yield_val - control_yield;
                let percent = if control_yield > 0.0 {
                    (increase / control_yield) * 100.0
                } else {
                    0.0
                };
                YieldComparison {
                    treatment_name: y.block_code.clone(),
                    yield_per_ha: yield_val,
                    yield_increase_vs_control: increase,
                    yield_increase_percent: percent,
                }
            })
            .collect();

        // Calculate ROI
        let control_cost = treatment_costs
            .iter()
            .find(|tc| formula_costs.iter().any(|fc| fc.block_code == tc.treatment_name && fc.is_control))
            .map(|tc| tc.total_cost_per_ha)
            .unwrap_or_default();

        let roi_analysis: Vec<ROIAnalysis> = treatment_costs
            .iter()
            .zip(yield_comparison.iter())
            .filter(|(_, yc)| yc.yield_increase_vs_control != 0.0)
            .map(|(tc, yc)| {
                let additional_cost = tc.total_cost_per_ha - control_cost;
                let additional_revenue = crop_price_per_kg * Decimal::try_from(yc.yield_increase_vs_control).unwrap_or_default();
                let net_benefit = additional_revenue - additional_cost;
                let roi = if additional_cost > Decimal::ZERO {
                    ((net_benefit / additional_cost) * Decimal::from(100)).to_string().parse::<f64>().unwrap_or(0.0)
                } else {
                    0.0
                };
                ROIAnalysis {
                    treatment_name: tc.treatment_name.clone(),
                    additional_cost,
                    additional_revenue,
                    net_benefit,
                    roi_percent: roi,
                    is_profitable: net_benefit > Decimal::ZERO,
                }
            })
            .collect();

        // Simple break-even analysis
        let avg_additional_cost: Decimal = if !roi_analysis.is_empty() {
            roi_analysis.iter().map(|r| r.additional_cost).sum::<Decimal>() / Decimal::from(roi_analysis.len())
        } else {
            Decimal::ZERO
        };

        let break_even_yield = if crop_price_per_kg > Decimal::ZERO {
            avg_additional_cost / crop_price_per_kg
        } else {
            Decimal::ZERO
        };

        let break_even_analysis = BreakEvenAnalysis {
            break_even_yield_increase: break_even_yield.to_string().parse().unwrap_or(0.0),
            break_even_price: avg_additional_cost,
            current_margin_of_safety: 0.0, // Simplified
        };

        // Generate recommendation
        let profitable_treatments: Vec<_> = roi_analysis
            .iter()
            .filter(|r| r.is_profitable)
            .collect();

        let recommendation = if profitable_treatments.is_empty() {
            "No treatments showed positive ROI. Consider reviewing formulations or application rates.".to_string()
        } else {
            let best = profitable_treatments
                .iter()
                .max_by(|a, b| a.roi_percent.partial_cmp(&b.roi_percent).unwrap())
                .unwrap();
            format!(
                "Recommended: {} with ROI of {:.1}% and net benefit of Rp {:.0}/ha",
                best.treatment_name, best.roi_percent, best.net_benefit
            )
        };

        Ok(CostBenefitResult {
            treatment_costs,
            yield_comparison,
            roi_analysis,
            break_even_analysis,
            recommendation,
        })
    }
}

// Helper for Decimal parsing
trait DecimalExt {
    fn from_str_exact(s: &str) -> Option<Decimal>;
}

impl DecimalExt for Decimal {
    fn from_str_exact(s: &str) -> Option<Decimal> {
        s.parse().ok()
    }
}

// ==============================================================================
// RAG AI CHAT SERVICE - Research Assistant with Database Context
// ==============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Main message/query from user - supports both 'message' and 'query' fields
    #[serde(alias = "query")]
    pub message: String,
    pub project_id: Option<Uuid>,
    pub context_types: Option<Vec<String>>, // "projects", "formulas", "experiments", "results", "history"
    pub max_context_items: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Main response text - serialized as both 'answer' and 'response' for frontend compatibility
    pub answer: String,
    /// Alias for answer field for frontend compatibility
    #[serde(rename = "response")]
    pub response_alias: String,
    pub sources: Vec<ContextSource>,
    pub suggested_queries: Vec<String>,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSource {
    pub source_type: String,
    pub source_id: String,
    pub title: String,
    pub snippet: String,
    pub relevance_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryLogEntry {
    pub id: Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub action: String,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub user_name: String,
    pub details: Option<String>,
}

pub struct RAGChatService {
    client: Client<OpenAIConfig>,
    model: String,
}

impl RAGChatService {
    pub fn new(settings: &Settings) -> Option<Self> {
        if settings.openai.api_key.is_empty() {
            return None;
        }

        let config = OpenAIConfig::new().with_api_key(&settings.openai.api_key);
        let client = Client::with_config(config);

        Some(Self {
            client,
            model: settings.openai.model.clone(),
        })
    }

    /// Main RAG chat endpoint - answers questions using database context
    pub async fn chat(
        &self,
        pool: &PgPool,
        request: &ChatRequest,
    ) -> Result<ChatResponse, AppError> {
        // 1. Gather relevant context from database based on query
        let context = self.gather_context(pool, request).await?;
        
        // 2. Build prompt with context
        let system_prompt = self.build_system_prompt();
        let user_prompt = self.build_user_prompt(&request.message, &context);
        
        // 3. Call OpenAI
        let response = self.call_openai_chat(&system_prompt, &user_prompt).await?;
        
        // 4. Extract sources for attribution
        let sources = context.iter().take(5).map(|c| c.clone()).collect();
        
        // 5. Generate suggested follow-up queries
        let suggested_queries = self.generate_suggestions(&request.message, &context);
        
        Ok(ChatResponse {
            answer: response.clone(),
            response_alias: response,
            sources,
            suggested_queries,
            confidence: self.estimate_confidence(&context),
        })
    }

    async fn gather_context(
        &self,
        pool: &PgPool,
        request: &ChatRequest,
    ) -> Result<Vec<ContextSource>, AppError> {
        let mut context_sources = Vec::new();
        let max_items = request.max_context_items.unwrap_or(10);
        let query_lower = request.message.to_lowercase();
        
        let context_types = request.context_types.clone().unwrap_or_else(|| {
            vec!["projects".to_string(), "formulas".to_string(), "experiments".to_string()]
        });

        // Search projects
        if context_types.contains(&"projects".to_string()) {
            let projects = self.search_projects(pool, &query_lower, request.project_id).await?;
            context_sources.extend(projects);
        }

        // Search formulas
        if context_types.contains(&"formulas".to_string()) {
            let formulas = self.search_formulas(pool, &query_lower).await?;
            context_sources.extend(formulas);
        }

        // Search experiment data
        if context_types.contains(&"experiments".to_string()) || context_types.contains(&"results".to_string()) {
            let experiments = self.search_experiments(pool, &query_lower, request.project_id).await?;
            context_sources.extend(experiments);
        }

        // Search audit history
        if context_types.contains(&"history".to_string()) {
            let history = self.search_history(pool, &query_lower).await?;
            context_sources.extend(history);
        }

        // Sort by relevance and limit
        context_sources.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap_or(std::cmp::Ordering::Equal));
        context_sources.truncate(max_items);

        Ok(context_sources)
    }

    async fn search_projects(
        &self,
        pool: &PgPool,
        query: &str,
        specific_project: Option<Uuid>,
    ) -> Result<Vec<ContextSource>, AppError> {
        let rows: Vec<(Uuid, String, String, String, Option<String>, Option<String>, Option<String>)> = if let Some(pid) = specific_project {
            sqlx::query_as(
                r#"SELECT id, code, title, status::text, hypothesis, methodology, crop_type
                   FROM projects WHERE id = $1"#
            )
            .bind(pid)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
        } else {
            sqlx::query_as(
                r#"SELECT id, code, title, status::text, hypothesis, methodology, crop_type
                   FROM projects 
                   WHERE LOWER(title) LIKE $1 
                      OR LOWER(code) LIKE $1 
                      OR LOWER(hypothesis) LIKE $1
                      OR LOWER(crop_type) LIKE $1
                   ORDER BY created_at DESC
                   LIMIT 5"#
            )
            .bind(format!("%{}%", query))
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
        };

        Ok(rows.iter().map(|row| {
            let snippet = format!(
                "Project: {} - {} [{}]\nHypothesis: {}\nMethodology: {}\nCrop: {}",
                row.1, row.2, row.3,
                row.4.as_deref().unwrap_or("N/A"),
                row.5.as_deref().unwrap_or("N/A"),
                row.6.as_deref().unwrap_or("N/A")
            );
            let relevance = self.calculate_relevance(query, &snippet);
            ContextSource {
                source_type: "project".to_string(),
                source_id: row.0.to_string(),
                title: format!("{} - {}", row.1, row.2),
                snippet,
                relevance_score: relevance,
            }
        }).collect())
    }

    async fn search_formulas(
        &self,
        pool: &PgPool,
        query: &str,
    ) -> Result<Vec<ContextSource>, AppError> {
        let rows: Vec<(Uuid, String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
            r#"SELECT id, code, name, status::text, description, application_rate
               FROM formulas 
               WHERE LOWER(name) LIKE $1 
                  OR LOWER(code) LIKE $1 
                  OR LOWER(description) LIKE $1
               ORDER BY created_at DESC
               LIMIT 5"#
        )
        .bind(format!("%{}%", query))
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(rows.iter().map(|row| {
            let snippet = format!(
                "Formula: {} - {} [{}]\nDescription: {}\nApplication Rate: {}",
                row.1, row.2, row.3,
                row.4.as_deref().unwrap_or("N/A"),
                row.5.as_deref().unwrap_or("N/A")
            );
            let relevance = self.calculate_relevance(query, &snippet);
            ContextSource {
                source_type: "formula".to_string(),
                source_id: row.0.to_string(),
                title: format!("{} - {}", row.1, row.2),
                snippet,
                relevance_score: relevance,
            }
        }).collect())
    }

    async fn search_experiments(
        &self,
        pool: &PgPool,
        query: &str,
        project_id: Option<Uuid>,
    ) -> Result<Vec<ContextSource>, AppError> {
        // Search monitoring data with aggregates
        let base_query = if project_id.is_some() {
            r#"SELECT 
                p.id as project_id,
                p.code as project_code,
                mp.name as parameter_name,
                COUNT(md.id) as data_count,
                AVG(md.numeric_value)::float8 as avg_value,
                STDDEV(md.numeric_value)::float8 as std_dev
               FROM monitoring_data md
               JOIN experimental_units eu ON md.unit_id = eu.id
               JOIN experimental_blocks eb ON eu.block_id = eb.id
               JOIN projects p ON eb.project_id = p.id
               JOIN monitoring_parameters mp ON md.parameter_id = mp.id
               WHERE p.id = $1
               GROUP BY p.id, p.code, mp.name
               LIMIT 10"#
        } else {
            r#"SELECT 
                p.id as project_id,
                p.code as project_code,
                mp.name as parameter_name,
                COUNT(md.id) as data_count,
                AVG(md.numeric_value)::float8 as avg_value,
                STDDEV(md.numeric_value)::float8 as std_dev
               FROM monitoring_data md
               JOIN experimental_units eu ON md.unit_id = eu.id
               JOIN experimental_blocks eb ON eu.block_id = eb.id
               JOIN projects p ON eb.project_id = p.id
               JOIN monitoring_parameters mp ON md.parameter_id = mp.id
               WHERE LOWER(mp.name) LIKE $1 OR LOWER(p.code) LIKE $1
               GROUP BY p.id, p.code, mp.name
               LIMIT 10"#
        };

        let rows: Vec<(Uuid, String, Option<String>, Option<i64>, Option<f64>, Option<f64>)> = if let Some(pid) = project_id {
            sqlx::query_as(base_query)
                .bind(pid)
                .fetch_all(pool)
                .await
                .map_err(|e| AppError::Database(e.to_string()))?
        } else {
            sqlx::query_as(base_query)
                .bind(format!("%{}%", query))
                .fetch_all(pool)
                .await
                .map_err(|e| AppError::Database(e.to_string()))?
        };

        Ok(rows.iter().map(|row| {
            let snippet = format!(
                "Project {} - Parameter: {}\nData Points: {}, Mean: {:.2} ± {:.2}",
                row.1,
                row.2.as_deref().unwrap_or("Unknown"),
                row.3.unwrap_or(0),
                row.4.unwrap_or(0.0),
                row.5.unwrap_or(0.0)
            );
            let relevance = self.calculate_relevance(query, &snippet);
            ContextSource {
                source_type: "experiment_data".to_string(),
                source_id: row.0.to_string(),
                title: format!("{} - {}", row.1, row.2.as_deref().unwrap_or("Data")),
                snippet,
                relevance_score: relevance,
            }
        }).collect())
    }

    async fn search_history(
        &self,
        pool: &PgPool,
        query: &str,
    ) -> Result<Vec<ContextSource>, AppError> {
        let rows: Vec<(Uuid, chrono::DateTime<chrono::Utc>, String, String, Option<Uuid>, Option<serde_json::Value>)> = sqlx::query_as(
            r#"SELECT al.id, al.created_at, al.action, al.entity_type, al.entity_id, al.changes
               FROM audit_logs al
               WHERE LOWER(al.action) LIKE $1 
                  OR LOWER(al.entity_type) LIKE $1
               ORDER BY al.created_at DESC
               LIMIT 10"#
        )
        .bind(format!("%{}%", query))
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(rows.iter().map(|row| {
            let changes_str = row.5.as_ref()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "N/A".to_string());
            let snippet = format!(
                "Action: {} on {} at {}\nChanges: {}",
                row.2, row.3, row.1.format("%Y-%m-%d %H:%M"),
                if changes_str.len() > 200 { &changes_str[..200] } else { &changes_str }
            );
            let relevance = self.calculate_relevance(query, &snippet);
            ContextSource {
                source_type: "audit_log".to_string(),
                source_id: row.0.to_string(),
                title: format!("{} - {}", row.2, row.3),
                snippet,
                relevance_score: relevance,
            }
        }).collect())
    }

    fn calculate_relevance(&self, query: &str, text: &str) -> f32 {
        let query_words: Vec<&str> = query.split_whitespace().collect();
        let text_lower = text.to_lowercase();
        
        let mut matches = 0;
        for word in &query_words {
            if text_lower.contains(*word) {
                matches += 1;
            }
        }
        
        if query_words.is_empty() {
            return 0.5;
        }
        
        matches as f32 / query_words.len() as f32
    }

    fn build_system_prompt(&self) -> String {
        r#"You are the CentraBio R&D NEXUS AI Research Assistant - an expert in agricultural biotechnology, biostimulants, and fertilizer research. You help researchers at Centra Biotech Indonesia analyze experiments, formulations, and research data.

Your capabilities:
1. Answer questions about ongoing research projects and experiments
2. Explain statistical results and their implications
3. Compare formula performances across experiments
4. Provide recommendations based on data analysis
5. Help with research methodology questions
6. Summarize historical research activities

Always base your answers on the provided context from the database. If the context doesn't contain enough information, say so clearly. Cite specific projects, formulas, or data points when relevant.

Response format:
- Be concise but thorough
- Use scientific terminology appropriate for R&D professionals
- Include specific numbers and statistics when available
- Suggest follow-up analyses when appropriate
- Highlight any data quality concerns"#.to_string()
    }

    fn build_user_prompt(&self, query: &str, context: &[ContextSource]) -> String {
        let context_text = context.iter()
            .map(|c| format!("[{}] {}\n{}", c.source_type.to_uppercase(), c.title, c.snippet))
            .collect::<Vec<_>>()
            .join("\n\n");

        format!(
            r#"## DATABASE CONTEXT:
{}

## USER QUESTION:
{}

Please answer based on the context provided. If the context is insufficient, indicate what additional information would be helpful."#,
            context_text,
            query
        )
    }

    async fn call_openai_chat(&self, system: &str, user: &str) -> Result<String, AppError> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages([
                ChatCompletionRequestMessage::System(
                    ChatCompletionRequestSystemMessageArgs::default()
                        .content(system)
                        .build()
                        .map_err(|e| AppError::AIError(e.to_string()))?
                ),
                ChatCompletionRequestMessage::User(
                    ChatCompletionRequestUserMessageArgs::default()
                        .content(user)
                        .build()
                        .map_err(|e| AppError::AIError(e.to_string()))?
                ),
            ])
            .temperature(0.4)
            .max_tokens(1500_u32)
            .build()
            .map_err(|e| AppError::AIError(e.to_string()))?;

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| AppError::AIError(e.to_string()))?;

        response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| AppError::AIError("No response from AI".to_string()))
    }

    fn generate_suggestions(&self, query: &str, context: &[ContextSource]) -> Vec<String> {
        let mut suggestions = Vec::new();
        
        // Based on context types found
        let has_projects = context.iter().any(|c| c.source_type == "project");
        let has_formulas = context.iter().any(|c| c.source_type == "formula");
        let has_data = context.iter().any(|c| c.source_type == "experiment_data");

        if has_projects {
            suggestions.push("What are the key findings from this project?".to_string());
            suggestions.push("Compare results with control group".to_string());
        }
        if has_formulas {
            suggestions.push("What is the cost-benefit analysis of this formula?".to_string());
            suggestions.push("Show application recommendations".to_string());
        }
        if has_data {
            suggestions.push("Perform statistical analysis on this data".to_string());
            suggestions.push("Show trends over time".to_string());
        }
        
        // Generic suggestions
        if suggestions.is_empty() {
            suggestions.push("Show all active projects".to_string());
            suggestions.push("List recent experiment results".to_string());
            suggestions.push("What formulas are pending QC?".to_string());
        }

        suggestions.truncate(4);
        suggestions
    }

    fn estimate_confidence(&self, context: &[ContextSource]) -> String {
        if context.is_empty() {
            return "low".to_string();
        }
        
        let avg_relevance: f32 = context.iter().map(|c| c.relevance_score).sum::<f32>() / context.len() as f32;
        
        if avg_relevance > 0.7 && context.len() >= 3 {
            "high".to_string()
        } else if avg_relevance > 0.4 || context.len() >= 2 {
            "medium".to_string()
        } else {
            "low".to_string()
        }
    }

    /// Export history logs with various filters
    pub async fn export_history_logs(
        pool: &PgPool,
        start_date: Option<chrono::DateTime<chrono::Utc>>,
        end_date: Option<chrono::DateTime<chrono::Utc>>,
        entity_type: Option<&str>,
        action_filter: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<HistoryLogEntry>, AppError> {
        let mut query_parts = vec!["SELECT al.id, al.created_at, al.action, al.entity_type, al.entity_id, u.full_name, al.changes::text FROM audit_logs al LEFT JOIN users u ON al.performed_by = u.id WHERE 1=1"];
        let mut conditions = Vec::new();

        if start_date.is_some() {
            conditions.push("al.created_at >= $1");
        }
        if end_date.is_some() {
            conditions.push("al.created_at <= $2");
        }
        if entity_type.is_some() {
            conditions.push("al.entity_type = $3");
        }
        if action_filter.is_some() {
            conditions.push("LOWER(al.action) LIKE $4");
        }

        // Build query dynamically
        let limit_val = limit.unwrap_or(1000);
        
        // Use a simpler approach - fetch all and filter in Rust for flexibility
        let rows: Vec<(Uuid, chrono::DateTime<chrono::Utc>, String, String, Option<Uuid>, Option<String>, Option<String>)> = sqlx::query_as(
            r#"SELECT al.id, al.created_at, al.action, al.entity_type, al.entity_id, 
                      COALESCE(u.full_name, 'System') as user_name, al.changes::text
               FROM audit_logs al 
               LEFT JOIN users u ON al.performed_by = u.id
               ORDER BY al.created_at DESC
               LIMIT $1"#
        )
        .bind(limit_val)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let mut entries: Vec<HistoryLogEntry> = rows.iter()
            .filter(|row| {
                let mut include = true;
                if let Some(start) = start_date {
                    include = include && row.1 >= start;
                }
                if let Some(end) = end_date {
                    include = include && row.1 <= end;
                }
                if let Some(et) = entity_type {
                    include = include && row.3 == et;
                }
                if let Some(af) = action_filter {
                    include = include && row.2.to_lowercase().contains(&af.to_lowercase());
                }
                include
            })
            .map(|row| HistoryLogEntry {
                id: row.0,
                timestamp: row.1,
                action: row.2.clone(),
                entity_type: row.3.clone(),
                entity_id: row.4,
                user_name: row.5.clone().unwrap_or_else(|| "System".to_string()),
                details: row.6.clone(),
            })
            .collect();

        Ok(entries)
    }
}

// ==============================================================================
// HISTORY EXPORT SERVICE
// ==============================================================================

pub struct HistoryExportService;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportRequest {
    pub start_date: Option<String>, // ISO 8601 format
    pub end_date: Option<String>,
    pub entity_types: Option<Vec<String>>,
    pub actions: Option<Vec<String>>,
    pub format: String, // "json", "csv"
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub format: String,
    pub total_records: usize,
    pub data: serde_json::Value,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

impl HistoryExportService {
    pub async fn export(
        pool: &PgPool,
        request: &ExportRequest,
    ) -> Result<ExportResult, AppError> {
        let start_date = request.start_date.as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        
        let end_date = request.end_date.as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let entity_type = request.entity_types.as_ref()
            .and_then(|v| v.first())
            .map(|s| s.as_str());
        
        let action_filter = request.actions.as_ref()
            .and_then(|v| v.first())
            .map(|s| s.as_str());

        let logs = RAGChatService::export_history_logs(
            pool,
            start_date,
            end_date,
            entity_type,
            action_filter,
            request.limit,
        ).await?;

        let total_records = logs.len();

        let data = match request.format.as_str() {
            "csv" => {
                // Build CSV string
                let mut csv = String::from("id,timestamp,action,entity_type,entity_id,user_name,details\n");
                for log in &logs {
                    csv.push_str(&format!(
                        "{},{},{},{},{},{},{}\n",
                        log.id,
                        log.timestamp.to_rfc3339(),
                        log.action.replace(",", ";"),
                        log.entity_type.replace(",", ";"),
                        log.entity_id.map(|id| id.to_string()).unwrap_or_default(),
                        log.user_name.replace(",", ";"),
                        log.details.as_ref().map(|d| d.replace(",", ";").replace("\n", " ")).unwrap_or_default()
                    ));
                }
                serde_json::json!({ "csv": csv })
            }
            _ => {
                // JSON format (default)
                serde_json::to_value(&logs).unwrap_or(serde_json::json!([]))
            }
        };

        Ok(ExportResult {
            format: request.format.clone(),
            total_records,
            data,
            generated_at: chrono::Utc::now(),
        })
    }

    /// Export project-specific history
    pub async fn export_project_history(
        pool: &PgPool,
        project_id: Uuid,
        format: &str,
    ) -> Result<ExportResult, AppError> {
        let rows: Vec<(Uuid, chrono::DateTime<chrono::Utc>, String, String, Option<String>, Option<String>)> = sqlx::query_as(
            r#"SELECT al.id, al.created_at, al.action, al.entity_type, 
                      COALESCE(u.full_name, 'System') as user_name, al.changes::text
               FROM audit_logs al 
               LEFT JOIN users u ON al.performed_by = u.id
               WHERE al.entity_id = $1 
                  OR (al.entity_type = 'project' AND al.changes::text LIKE $2)
               ORDER BY al.created_at DESC
               LIMIT 500"#
        )
        .bind(project_id)
        .bind(format!("%{}%", project_id))
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let logs: Vec<HistoryLogEntry> = rows.iter().map(|row| HistoryLogEntry {
            id: row.0,
            timestamp: row.1,
            action: row.2.clone(),
            entity_type: row.3.clone(),
            entity_id: Some(project_id),
            user_name: row.4.clone().unwrap_or_else(|| "System".to_string()),
            details: row.5.clone(),
        }).collect();

        let total_records = logs.len();

        let data = match format {
            "csv" => {
                let mut csv = String::from("id,timestamp,action,entity_type,user_name,details\n");
                for log in &logs {
                    csv.push_str(&format!(
                        "{},{},{},{},{},{}\n",
                        log.id,
                        log.timestamp.to_rfc3339(),
                        log.action.replace(",", ";"),
                        log.entity_type.replace(",", ";"),
                        log.user_name.replace(",", ";"),
                        log.details.as_ref().map(|d| d.replace(",", ";").replace("\n", " ")).unwrap_or_default()
                    ));
                }
                serde_json::json!({ "csv": csv })
            }
            _ => serde_json::to_value(&logs).unwrap_or(serde_json::json!([]))
        };

        Ok(ExportResult {
            format: format.to_string(),
            total_records,
            data,
            generated_at: chrono::Utc::now(),
        })
    }
}
