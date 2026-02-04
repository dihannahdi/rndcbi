use crate::errors::AppError;
use crate::models::{ExperimentalBlock, ExperimentalUnit};
use image::{DynamicImage, Luma, Rgba, RgbaImage};
use qrcode::QrCode;
use sqlx::PgPool;
use std::io::Cursor;
use uuid::Uuid;

// ==============================================================================
// QR CODE SERVICE
// ==============================================================================

pub struct QRCodeService {
    base_url: String,
}

impl QRCodeService {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
        }
    }

    /// Generate QR code for an experimental unit
    pub fn generate_unit_qr(&self, unit: &ExperimentalUnit, block: &ExperimentalBlock) -> Result<QRCodeResult, AppError> {
        let data = QRCodeData {
            entity_type: "experimental_unit".to_string(),
            entity_id: unit.id,
            code: unit.unit_code.clone(),
            label: unit.unit_label.clone(),
            block_code: Some(block.block_code.clone()),
            project_id: Some(block.project_id),
            url: format!("{}/scan/unit/{}", self.base_url, unit.id),
        };

        self.generate_qr_code(&data)
    }

    /// Generate QR code for an experimental block
    pub fn generate_block_qr(&self, block: &ExperimentalBlock) -> Result<QRCodeResult, AppError> {
        let data = QRCodeData {
            entity_type: "experimental_block".to_string(),
            entity_id: block.id,
            code: block.block_code.clone(),
            label: block.block_name.clone(),
            block_code: None,
            project_id: Some(block.project_id),
            url: format!("{}/scan/block/{}", self.base_url, block.id),
        };

        self.generate_qr_code(&data)
    }

    /// Generate batch QR codes for all units in a project
    pub async fn generate_project_qr_codes(
        &self,
        pool: &PgPool,
        project_id: Uuid,
    ) -> Result<Vec<QRCodeResult>, AppError> {
        // Fetch all blocks
        let blocks: Vec<ExperimentalBlock> = sqlx::query_as(
            r#"
            SELECT * FROM experimental_blocks 
            WHERE project_id = $1
            ORDER BY block_code
            "#
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let mut results = Vec::new();

        for block in &blocks {
            // Generate block QR
            let block_qr = self.generate_block_qr(block)?;
            results.push(block_qr);

            // Fetch units for this block
            let units: Vec<ExperimentalUnit> = sqlx::query_as(
                r#"
                SELECT * FROM experimental_units 
                WHERE block_id = $1
                ORDER BY position_in_block
                "#
            )
            .bind(block.id)
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

            for unit in &units {
                let unit_qr = self.generate_unit_qr(unit, block)?;
                results.push(unit_qr);
            }

            // Update block QR data in database
            sqlx::query(
                r#"
                UPDATE experimental_blocks 
                SET qr_code_data = $2, qr_code_generated_at = NOW()
                WHERE id = $1
                "#
            )
            .bind(block.id)
            .bind(results.last().map(|r| &r.data_json))
            .execute(pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        }

        Ok(results)
    }

    fn generate_qr_code(&self, data: &QRCodeData) -> Result<QRCodeResult, AppError> {
        // Create JSON payload
        let json_data = serde_json::to_string(data)
            .map_err(|e| AppError::InternalError(format!("Failed to serialize QR data: {}", e)))?;

        // Generate QR code
        let code = QrCode::new(json_data.as_bytes())
            .map_err(|e| AppError::InternalError(format!("Failed to generate QR code: {}", e)))?;

        // Render to image
        let image = code.render::<Luma<u8>>().build();

        // Convert to RGBA and add label
        let mut rgba_image = DynamicImage::ImageLuma8(image).to_rgba8();
        
        // Add padding and label
        let qr_size = rgba_image.width();
        let padding = 20u32;
        let label_height = 40u32;
        let new_width = qr_size + (padding * 2);
        let new_height = qr_size + (padding * 2) + label_height;

        let mut final_image = RgbaImage::from_pixel(new_width, new_height, Rgba([255, 255, 255, 255]));

        // Copy QR code to center
        for (x, y, pixel) in rgba_image.enumerate_pixels() {
            final_image.put_pixel(x + padding, y + padding, *pixel);
        }

        // Convert to PNG bytes
        let mut png_bytes = Vec::new();
        final_image
            .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .map_err(|e| AppError::InternalError(format!("Failed to encode QR image: {}", e)))?;

        // Encode as base64
        let base64_image = base64::encode(&png_bytes);

        Ok(QRCodeResult {
            entity_type: data.entity_type.clone(),
            entity_id: data.entity_id,
            code: data.code.clone(),
            label: data.label.clone().unwrap_or_else(|| data.code.clone()),
            data_json: json_data,
            data_url: data.url.clone(),
            image_base64: base64_image,
            image_format: "png".to_string(),
        })
    }

    /// Parse QR code data from scanned string
    pub fn parse_qr_data(data: &str) -> Result<QRCodeData, AppError> {
        serde_json::from_str(data)
            .map_err(|e| AppError::Validation(format!("Invalid QR code data: {}", e)))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QRCodeData {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub code: String,
    pub label: Option<String>,
    pub block_code: Option<String>,
    pub project_id: Option<Uuid>,
    pub url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QRCodeResult {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub code: String,
    pub label: String,
    pub data_json: String,
    pub data_url: String,
    pub image_base64: String,
    pub image_format: String,
}

// ==============================================================================
// QR CODE HANDLERS
// ==============================================================================

use actix_web::{web, HttpResponse};

pub async fn generate_block_qr_handler(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    qr_service: web::Data<QRCodeService>,
) -> Result<HttpResponse, AppError> {
    let block_id = path.into_inner();

    let block: ExperimentalBlock = sqlx::query_as("SELECT * FROM experimental_blocks WHERE id = $1")
        .bind(block_id)
        .fetch_optional(pool.get_ref())
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Block not found".to_string()))?;

    let qr_result = qr_service.generate_block_qr(&block)?;

    Ok(HttpResponse::Ok().json(qr_result))
}

pub async fn generate_unit_qr_handler(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    qr_service: web::Data<QRCodeService>,
) -> Result<HttpResponse, AppError> {
    let unit_id = path.into_inner();

    let unit: ExperimentalUnit = sqlx::query_as("SELECT * FROM experimental_units WHERE id = $1")
        .bind(unit_id)
        .fetch_optional(pool.get_ref())
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Unit not found".to_string()))?;

    let block: ExperimentalBlock = sqlx::query_as("SELECT * FROM experimental_blocks WHERE id = $1")
        .bind(unit.block_id)
        .fetch_one(pool.get_ref())
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

    let qr_result = qr_service.generate_unit_qr(&unit, &block)?;

    Ok(HttpResponse::Ok().json(qr_result))
}

pub async fn generate_project_qr_codes_handler(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    qr_service: web::Data<QRCodeService>,
) -> Result<HttpResponse, AppError> {
    let project_id = path.into_inner();

    let results = qr_service
        .generate_project_qr_codes(pool.get_ref(), project_id)
        .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "count": results.len(),
        "qr_codes": results
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct ScanQRRequest {
    pub qr_data: String,
}

pub async fn scan_qr_handler(
    pool: web::Data<PgPool>,
    body: web::Json<ScanQRRequest>,
) -> Result<HttpResponse, AppError> {
    let qr_data = QRCodeService::parse_qr_data(&body.qr_data)?;

    // Return entity details based on type
    match qr_data.entity_type.as_str() {
        "experimental_unit" => {
            let unit: ExperimentalUnit = sqlx::query_as("SELECT * FROM experimental_units WHERE id = $1")
                .bind(qr_data.entity_id)
                .fetch_optional(pool.get_ref())
                .await
                .map_err(|e| AppError::Database(e.to_string()))?
                .ok_or_else(|| AppError::NotFound("Unit not found".to_string()))?;

            let block: ExperimentalBlock = sqlx::query_as("SELECT * FROM experimental_blocks WHERE id = $1")
                .bind(unit.block_id)
                .fetch_one(pool.get_ref())
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            Ok(HttpResponse::Ok().json(serde_json::json!({
                "entity_type": "experimental_unit",
                "unit": unit,
                "block": block
            })))
        }
        "experimental_block" => {
            let block: ExperimentalBlock = sqlx::query_as("SELECT * FROM experimental_blocks WHERE id = $1")
                .bind(qr_data.entity_id)
                .fetch_optional(pool.get_ref())
                .await
                .map_err(|e| AppError::Database(e.to_string()))?
                .ok_or_else(|| AppError::NotFound("Block not found".to_string()))?;

            Ok(HttpResponse::Ok().json(serde_json::json!({
                "entity_type": "experimental_block",
                "block": block
            })))
        }
        _ => Err(AppError::Validation(format!(
            "Unknown entity type: {}",
            qr_data.entity_type
        ))),
    }
}

// ==============================================================================
// GENERATE PRINTABLE QR SHEET
// ==============================================================================

pub async fn generate_qr_print_sheet(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>,
    qr_service: web::Data<QRCodeService>,
) -> Result<HttpResponse, AppError> {
    let project_id = path.into_inner();

    let qr_codes = qr_service
        .generate_project_qr_codes(pool.get_ref(), project_id)
        .await?;

    // Generate HTML for printing
    let mut html = String::from(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>QR Code Print Sheet</title>
    <style>
        @media print {
            .no-print { display: none; }
            .page-break { page-break-after: always; }
        }
        body {
            font-family: Arial, sans-serif;
            margin: 0;
            padding: 20px;
        }
        .qr-grid {
            display: grid;
            grid-template-columns: repeat(4, 1fr);
            gap: 20px;
        }
        .qr-item {
            border: 1px solid #ccc;
            padding: 10px;
            text-align: center;
            break-inside: avoid;
        }
        .qr-image {
            width: 150px;
            height: 150px;
        }
        .qr-label {
            font-weight: bold;
            margin-top: 10px;
            font-size: 12px;
        }
        .qr-code {
            font-size: 10px;
            color: #666;
        }
        .header {
            text-align: center;
            margin-bottom: 20px;
        }
        .print-btn {
            padding: 10px 20px;
            font-size: 16px;
            cursor: pointer;
            margin-bottom: 20px;
        }
    </style>
</head>
<body>
    <div class="header no-print">
        <h1>QR Code Labels</h1>
        <button class="print-btn" onclick="window.print()">Print Labels</button>
    </div>
    <div class="qr-grid">
"#,
    );

    for qr in qr_codes {
        html.push_str(&format!(
            r#"
        <div class="qr-item">
            <img class="qr-image" src="data:image/png;base64,{}" alt="QR Code">
            <div class="qr-label">{}</div>
            <div class="qr-code">{}</div>
        </div>
"#,
            qr.image_base64, qr.label, qr.code
        ));
    }

    html.push_str(
        r#"
    </div>
</body>
</html>
"#,
    );

    Ok(HttpResponse::Ok()
        .content_type("text/html")
        .body(html))
}
