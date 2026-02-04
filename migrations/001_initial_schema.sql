-- CENTRABIO R&D NEXUS Database Schema
-- Scientific Decision Support System for Research Management
-- PostgreSQL 16+

-- Enable required extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- ==============================================================================
-- DOMAIN TYPES & ENUMS
-- ==============================================================================

-- User Roles
CREATE TYPE user_role AS ENUM (
    'principal_researcher',   -- PIC Riset - Full Read/Write on their projects
    'qc_analyst',            -- Lab QC - Can change formula status
    'field_officer',         -- Input monitoring data, scan QR
    'rd_manager',            -- View all + Approval access
    'system_admin'           -- System administration
);

-- Project Status
CREATE TYPE project_status AS ENUM (
    'draft',
    'active',
    'on_hold',
    'completed',
    'archived',
    'locked'
);

-- Formula Status (QC Gate)
CREATE TYPE formula_status AS ENUM (
    'draft',
    'pending_qc',
    'qc_in_progress',
    'qc_passed',
    'qc_failed',
    'revision_required',
    'archived'
);

-- Lab Test Status
CREATE TYPE lab_test_status AS ENUM (
    'pending',
    'in_progress',
    'completed',
    'failed',
    'invalid'
);

-- Experimental Design Type
CREATE TYPE experiment_design AS ENUM (
    'rak',              -- Rancangan Acak Kelompok (Randomized Complete Block Design)
    'ral',              -- Rancangan Acak Lengkap (Completely Randomized Design)
    'factorial',        -- Factorial Design
    'split_plot',       -- Split-Plot Design
    'custom'            -- Custom Design
);

-- Monitoring Data Type
CREATE TYPE monitoring_type AS ENUM (
    'height',           -- Tinggi tanaman
    'leaf_count',       -- Jumlah daun
    'stem_diameter',    -- Diameter batang
    'leaf_area',        -- Luas daun
    'chlorophyll',      -- Klorofil/SPAD
    'pest_level',       -- Tingkat serangan hama
    'disease_level',    -- Tingkat serangan penyakit
    'yield',            -- Hasil panen
    'custom'            -- Parameter kustom
);

-- Unit Types
CREATE TYPE measurement_unit AS ENUM (
    'cm', 'mm', 'm',           -- Length
    'g', 'kg', 'mg',           -- Mass
    'ml', 'l', 'ul',           -- Volume
    'ppm', 'percent',          -- Concentration
    'count', 'score',          -- Count/Rating
    'celsius', 'ph',           -- Temperature/pH
    'custom'                    -- Custom unit
);

-- ==============================================================================
-- CORE TABLES
-- ==============================================================================

-- Organizations/Divisions
CREATE TABLE organizations (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name VARCHAR(255) NOT NULL,
    code VARCHAR(50) UNIQUE NOT NULL,
    description TEXT,
    address TEXT,
    contact_email VARCHAR(255),
    contact_phone VARCHAR(50),
    logo_url TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    is_active BOOLEAN DEFAULT TRUE
);

-- Users
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
    email VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    full_name VARCHAR(255) NOT NULL,
    employee_id VARCHAR(50),
    role user_role NOT NULL,
    phone VARCHAR(50),
    avatar_url TEXT,
    department VARCHAR(100),
    position VARCHAR(100),
    
    -- Security
    is_active BOOLEAN DEFAULT TRUE,
    is_email_verified BOOLEAN DEFAULT FALSE,
    email_verification_token UUID,
    password_reset_token UUID,
    password_reset_expires TIMESTAMP WITH TIME ZONE,
    last_login TIMESTAMP WITH TIME ZONE,
    failed_login_attempts INTEGER DEFAULT 0,
    locked_until TIMESTAMP WITH TIME ZONE,
    
    -- Metadata
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    created_by UUID REFERENCES users(id),
    
    CONSTRAINT valid_email CHECK (email ~* '^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$')
);

-- User Sessions (for JWT invalidation)
CREATE TABLE user_sessions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash VARCHAR(255) NOT NULL,
    device_info TEXT,
    ip_address INET,
    user_agent TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL,
    is_valid BOOLEAN DEFAULT TRUE
);

-- Audit Log
CREATE TABLE audit_logs (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID REFERENCES users(id),
    action VARCHAR(100) NOT NULL,
    entity_type VARCHAR(100) NOT NULL,
    entity_id UUID,
    old_values JSONB,
    new_values JSONB,
    ip_address INET,
    user_agent TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- ==============================================================================
-- MODULE 1: PROJECT CHARTER (ABSTRAK)
-- ==============================================================================

-- Projects
CREATE TABLE projects (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id),
    code VARCHAR(50) UNIQUE NOT NULL,
    title VARCHAR(500) NOT NULL,
    
    -- Research Proposal
    background TEXT,
    objectives TEXT,
    hypothesis TEXT,
    methodology TEXT,
    expected_outcomes TEXT,
    
    -- Success Metrics (KPI)
    success_metrics JSONB DEFAULT '[]'::jsonb,
    -- Structure: [{"name": "Growth Rate", "target": ">15%", "unit": "percent", "description": "..."}]
    
    -- Status & Timeline
    status project_status DEFAULT 'draft',
    start_date DATE,
    end_date DATE,
    actual_end_date DATE,
    
    -- Budget
    budget_amount DECIMAL(15, 2),
    budget_currency VARCHAR(3) DEFAULT 'IDR',
    actual_cost DECIMAL(15, 2),
    
    -- Crop/Subject Information
    crop_type VARCHAR(100),
    crop_variety VARCHAR(100),
    growth_stage VARCHAR(100),
    
    -- Location
    location_name VARCHAR(255),
    location_type VARCHAR(50), -- greenhouse, field, lab
    location_coordinates POINT,
    location_address TEXT,
    
    -- Experimental Design
    experiment_design experiment_design,
    replications INTEGER DEFAULT 3,
    treatments_count INTEGER,
    blocks_count INTEGER,
    plot_size VARCHAR(100),
    
    -- Locking System
    is_locked BOOLEAN DEFAULT FALSE,
    locked_at TIMESTAMP WITH TIME ZONE,
    locked_by UUID REFERENCES users(id),
    lock_reason TEXT,
    
    -- Metadata
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    created_by UUID NOT NULL REFERENCES users(id),
    approved_by UUID REFERENCES users(id),
    approved_at TIMESTAMP WITH TIME ZONE
);

-- Project Team Members
CREATE TABLE project_team_members (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role VARCHAR(100) NOT NULL, -- PIC, Assistant, Field Officer, etc.
    responsibilities TEXT,
    assigned_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    assigned_by UUID REFERENCES users(id),
    is_active BOOLEAN DEFAULT TRUE,
    
    UNIQUE(project_id, user_id)
);

-- Project Milestones
CREATE TABLE project_milestones (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    target_date DATE,
    completed_date DATE,
    is_completed BOOLEAN DEFAULT FALSE,
    completion_notes TEXT,
    sort_order INTEGER DEFAULT 0,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Project Documents/Attachments
CREATE TABLE project_attachments (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    file_name VARCHAR(255) NOT NULL,
    file_path TEXT NOT NULL,
    file_size BIGINT,
    mime_type VARCHAR(100),
    file_type VARCHAR(50), -- proposal, report, image, protocol, etc.
    description TEXT,
    uploaded_by UUID NOT NULL REFERENCES users(id),
    uploaded_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- ==============================================================================
-- MODULE 2: LAB FORMULATION & QC GATE
-- ==============================================================================

-- Raw Materials/Ingredients
CREATE TABLE raw_materials (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id),
    code VARCHAR(50) UNIQUE NOT NULL,
    name VARCHAR(255) NOT NULL,
    category VARCHAR(100), -- active ingredient, carrier, adjuvant, etc.
    description TEXT,
    
    -- Stock Information
    stock_quantity DECIMAL(15, 4) DEFAULT 0,
    stock_unit VARCHAR(20),
    minimum_stock DECIMAL(15, 4),
    
    -- Cost Information
    unit_cost DECIMAL(15, 4),
    cost_currency VARCHAR(3) DEFAULT 'IDR',
    
    -- Specifications
    specifications JSONB DEFAULT '{}'::jsonb,
    -- Structure: {"purity": "99%", "origin": "Import", "supplier": "PT XYZ"}
    
    -- Safety Information
    safety_data_sheet TEXT,
    handling_instructions TEXT,
    storage_requirements TEXT,
    
    is_active BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    created_by UUID REFERENCES users(id)
);

-- Formulas
CREATE TABLE formulas (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    code VARCHAR(50) NOT NULL,
    name VARCHAR(255) NOT NULL,
    
    -- Version Control
    version VARCHAR(20) DEFAULT '1.0',
    parent_formula_id UUID REFERENCES formulas(id),
    is_latest_version BOOLEAN DEFAULT TRUE,
    
    -- Status (QC Gate)
    status formula_status DEFAULT 'draft',
    
    -- Description
    description TEXT,
    intended_use TEXT,
    target_crop VARCHAR(100),
    application_method VARCHAR(100),
    application_rate VARCHAR(100),
    
    -- Total Production
    total_volume DECIMAL(15, 4),
    volume_unit VARCHAR(20),
    
    -- Cost Calculation
    calculated_cost DECIMAL(15, 4),
    cost_per_unit DECIMAL(15, 4),
    cost_currency VARCHAR(3) DEFAULT 'IDR',
    
    -- Technical Specifications
    target_ph_min DECIMAL(4, 2),
    target_ph_max DECIMAL(4, 2),
    target_density DECIMAL(8, 4),
    target_viscosity DECIMAL(10, 2),
    
    -- Metadata
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    created_by UUID NOT NULL REFERENCES users(id),
    
    -- QC Information
    qc_approved_by UUID REFERENCES users(id),
    qc_approved_at TIMESTAMP WITH TIME ZONE,
    qc_notes TEXT,
    
    UNIQUE(project_id, code, version)
);

-- Formula Ingredients
CREATE TABLE formula_ingredients (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    formula_id UUID NOT NULL REFERENCES formulas(id) ON DELETE CASCADE,
    raw_material_id UUID NOT NULL REFERENCES raw_materials(id),
    
    -- Quantity
    quantity DECIMAL(15, 6) NOT NULL,
    unit VARCHAR(20) NOT NULL,
    
    -- Percentage
    percentage DECIMAL(8, 4),
    
    -- Function in Formula
    function_role VARCHAR(100), -- active, carrier, stabilizer, etc.
    
    -- Notes
    notes TEXT,
    
    sort_order INTEGER DEFAULT 0,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(formula_id, raw_material_id)
);

-- Lab Tests (QC Tests)
CREATE TABLE lab_tests (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    formula_id UUID NOT NULL REFERENCES formulas(id) ON DELETE CASCADE,
    test_code VARCHAR(50) NOT NULL,
    test_name VARCHAR(255) NOT NULL,
    
    -- Test Configuration
    test_method VARCHAR(255),
    parameter_tested VARCHAR(100),
    
    -- Standards
    standard_min DECIMAL(15, 6),
    standard_max DECIMAL(15, 6),
    standard_unit VARCHAR(50),
    
    -- Results
    result_value DECIMAL(15, 6),
    result_unit VARCHAR(50),
    status lab_test_status DEFAULT 'pending',
    is_passed BOOLEAN,
    
    -- Execution
    tested_by UUID REFERENCES users(id),
    tested_at TIMESTAMP WITH TIME ZONE,
    equipment_used VARCHAR(255),
    
    -- Notes & Evidence
    notes TEXT,
    observations TEXT,
    attachments JSONB DEFAULT '[]'::jsonb,
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Formula Change Log (Audit Trail)
CREATE TABLE formula_versions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    formula_id UUID NOT NULL REFERENCES formulas(id) ON DELETE CASCADE,
    version VARCHAR(20) NOT NULL,
    changes_summary TEXT,
    full_snapshot JSONB NOT NULL, -- Complete formula data at this version
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    created_by UUID NOT NULL REFERENCES users(id),
    
    UNIQUE(formula_id, version)
);

-- ==============================================================================
-- MODULE 3: FIELD MONITORING & DATA ACQUISITION
-- ==============================================================================

-- Experimental Blocks/Plots
CREATE TABLE experimental_blocks (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    
    -- Block Information
    block_code VARCHAR(50) NOT NULL,
    block_name VARCHAR(255),
    
    -- Treatment Assignment
    formula_id UUID REFERENCES formulas(id),
    treatment_description TEXT,
    is_control BOOLEAN DEFAULT FALSE,
    
    -- Location within experimental area
    position_row INTEGER,
    position_column INTEGER,
    coordinates POINT,
    
    -- Physical Details
    area_size DECIMAL(10, 2),
    area_unit VARCHAR(20),
    plant_count INTEGER,
    
    -- QR Code
    qr_code_data TEXT,
    qr_code_generated_at TIMESTAMP WITH TIME ZONE,
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(project_id, block_code)
);

-- Experimental Units (Individual Plants/Polybags)
CREATE TABLE experimental_units (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    block_id UUID NOT NULL REFERENCES experimental_blocks(id) ON DELETE CASCADE,
    
    -- Unit Identification
    unit_code VARCHAR(50) NOT NULL,
    unit_label VARCHAR(100),
    
    -- Position
    position_in_block INTEGER,
    row_number INTEGER,
    column_number INTEGER,
    
    -- QR Code
    qr_code_data TEXT UNIQUE,
    qr_code_url TEXT,
    
    -- Status
    is_active BOOLEAN DEFAULT TRUE,
    excluded_reason TEXT,
    excluded_at TIMESTAMP WITH TIME ZONE,
    
    -- Notes
    notes TEXT,
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(block_id, unit_code)
);

-- Monitoring Parameters (Custom parameters per project)
CREATE TABLE monitoring_parameters (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    
    code VARCHAR(50) NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    
    -- Data Type
    parameter_type monitoring_type,
    data_type VARCHAR(20) DEFAULT 'numeric', -- numeric, text, boolean, rating
    
    -- Unit & Constraints
    unit measurement_unit,
    custom_unit VARCHAR(50),
    min_value DECIMAL(15, 4),
    max_value DECIMAL(15, 4),
    decimal_places INTEGER DEFAULT 2,
    
    -- Validation
    outlier_threshold_percent DECIMAL(5, 2) DEFAULT 200, -- % deviation for outlier detection
    
    -- Display
    sort_order INTEGER DEFAULT 0,
    is_required BOOLEAN DEFAULT TRUE,
    is_active BOOLEAN DEFAULT TRUE,
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(project_id, code)
);

-- Monitoring Sessions
CREATE TABLE monitoring_sessions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    
    session_code VARCHAR(50) NOT NULL,
    session_name VARCHAR(255),
    
    -- Timing
    scheduled_date DATE NOT NULL,
    actual_date DATE,
    days_after_treatment INTEGER, -- DAT (Days After Treatment)
    week_number INTEGER,
    
    -- Status
    is_completed BOOLEAN DEFAULT FALSE,
    completed_at TIMESTAMP WITH TIME ZONE,
    completed_by UUID REFERENCES users(id),
    
    -- Notes
    weather_conditions TEXT,
    general_observations TEXT,
    notes TEXT,
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    UNIQUE(project_id, session_code)
);

-- Monitoring Data (Main data collection)
CREATE TABLE monitoring_data (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    session_id UUID NOT NULL REFERENCES monitoring_sessions(id) ON DELETE CASCADE,
    unit_id UUID NOT NULL REFERENCES experimental_units(id) ON DELETE CASCADE,
    parameter_id UUID NOT NULL REFERENCES monitoring_parameters(id),
    
    -- Value
    numeric_value DECIMAL(15, 4),
    text_value TEXT,
    boolean_value BOOLEAN,
    
    -- Quality Control
    is_outlier BOOLEAN DEFAULT FALSE,
    outlier_reason TEXT,
    is_verified BOOLEAN DEFAULT FALSE,
    verified_by UUID REFERENCES users(id),
    verified_at TIMESTAMP WITH TIME ZONE,
    
    -- Collection Metadata
    collected_by UUID NOT NULL REFERENCES users(id),
    collected_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    -- Device/Location Information
    device_id VARCHAR(100),
    latitude DECIMAL(10, 8),
    longitude DECIMAL(11, 8),
    
    -- Notes
    notes TEXT,
    
    -- Offline Sync
    offline_id UUID, -- ID from offline storage
    synced_at TIMESTAMP WITH TIME ZONE,
    
    UNIQUE(session_id, unit_id, parameter_id)
);

-- Monitoring Photos
CREATE TABLE monitoring_photos (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    monitoring_data_id UUID REFERENCES monitoring_data(id) ON DELETE CASCADE,
    unit_id UUID REFERENCES experimental_units(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES monitoring_sessions(id) ON DELETE CASCADE,
    
    -- File Information
    file_name VARCHAR(255) NOT NULL,
    file_path TEXT NOT NULL,
    file_size BIGINT,
    mime_type VARCHAR(100),
    
    -- Metadata
    latitude DECIMAL(10, 8),
    longitude DECIMAL(11, 8),
    taken_at TIMESTAMP WITH TIME ZONE,
    device_info TEXT,
    
    -- Descriptions
    description TEXT,
    tags JSONB DEFAULT '[]'::jsonb,
    
    uploaded_by UUID NOT NULL REFERENCES users(id),
    uploaded_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    -- Verification
    is_verified BOOLEAN DEFAULT FALSE,
    verified_by UUID REFERENCES users(id)
);

-- ==============================================================================
-- MODULE 4: ANALYSIS & REPORTING
-- ==============================================================================

-- Analysis Results (AI-Generated)
CREATE TABLE analysis_results (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    
    analysis_type VARCHAR(100) NOT NULL, -- anova, comparison, cost_benefit, summary
    analysis_name VARCHAR(255),
    
    -- Input Parameters
    input_parameters JSONB NOT NULL,
    -- Structure depends on analysis_type
    
    -- Results
    results JSONB NOT NULL,
    -- Structure: statistical outputs, p-values, conclusions, etc.
    
    -- AI Insights
    ai_model_used VARCHAR(100),
    ai_prompt TEXT,
    ai_insights TEXT,
    ai_recommendations TEXT,
    
    -- Metadata
    generated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    generated_by UUID NOT NULL REFERENCES users(id),
    
    -- Validity
    is_valid BOOLEAN DEFAULT TRUE,
    invalidated_reason TEXT
);

-- Generated Reports
CREATE TABLE generated_reports (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    
    report_code VARCHAR(50) NOT NULL,
    report_title VARCHAR(500) NOT NULL,
    report_type VARCHAR(100), -- progress, final, interim, custom
    
    -- Content
    content_markdown TEXT,
    content_html TEXT,
    
    -- Sections included
    sections JSONB DEFAULT '[]'::jsonb,
    -- Structure: ["background", "methods", "results", "discussion", "conclusions"]
    
    -- PDF
    pdf_path TEXT,
    pdf_generated_at TIMESTAMP WITH TIME ZONE,
    
    -- Metadata
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    created_by UUID NOT NULL REFERENCES users(id),
    
    -- Approval
    is_approved BOOLEAN DEFAULT FALSE,
    approved_by UUID REFERENCES users(id),
    approved_at TIMESTAMP WITH TIME ZONE
);

-- Report Templates
CREATE TABLE report_templates (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID REFERENCES organizations(id),
    
    name VARCHAR(255) NOT NULL,
    description TEXT,
    template_type VARCHAR(100),
    
    -- Template Content
    header_template TEXT,
    footer_template TEXT,
    section_templates JSONB DEFAULT '{}'::jsonb,
    
    -- Styling
    styles JSONB DEFAULT '{}'::jsonb,
    
    is_default BOOLEAN DEFAULT FALSE,
    is_active BOOLEAN DEFAULT TRUE,
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- ==============================================================================
-- SYSTEM TABLES
-- ==============================================================================

-- System Configuration
CREATE TABLE system_config (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    config_key VARCHAR(100) UNIQUE NOT NULL,
    config_value TEXT,
    config_type VARCHAR(50), -- string, number, boolean, json
    description TEXT,
    is_sensitive BOOLEAN DEFAULT FALSE,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_by UUID REFERENCES users(id)
);

-- Notification Templates
CREATE TABLE notification_templates (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    code VARCHAR(100) UNIQUE NOT NULL,
    name VARCHAR(255) NOT NULL,
    subject_template TEXT,
    body_template TEXT,
    channel VARCHAR(50), -- email, push, in_app
    is_active BOOLEAN DEFAULT TRUE
);

-- User Notifications
CREATE TABLE user_notifications (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    
    title VARCHAR(255) NOT NULL,
    message TEXT NOT NULL,
    notification_type VARCHAR(100),
    
    -- Reference
    entity_type VARCHAR(100),
    entity_id UUID,
    action_url TEXT,
    
    -- Status
    is_read BOOLEAN DEFAULT FALSE,
    read_at TIMESTAMP WITH TIME ZONE,
    
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- File Storage Metadata
CREATE TABLE file_storage (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    
    -- File Information
    original_name VARCHAR(255) NOT NULL,
    stored_name VARCHAR(255) NOT NULL,
    file_path TEXT NOT NULL,
    file_size BIGINT NOT NULL,
    mime_type VARCHAR(100),
    checksum VARCHAR(64), -- SHA-256
    
    -- Association
    entity_type VARCHAR(100),
    entity_id UUID,
    
    -- Metadata
    uploaded_by UUID NOT NULL REFERENCES users(id),
    uploaded_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    
    -- Status
    is_deleted BOOLEAN DEFAULT FALSE,
    deleted_at TIMESTAMP WITH TIME ZONE,
    deleted_by UUID REFERENCES users(id)
);

-- ==============================================================================
-- INDEXES
-- ==============================================================================

-- Users
CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_role ON users(role);
CREATE INDEX idx_users_organization ON users(organization_id);
CREATE INDEX idx_users_is_active ON users(is_active);

-- Projects
CREATE INDEX idx_projects_status ON projects(status);
CREATE INDEX idx_projects_organization ON projects(organization_id);
CREATE INDEX idx_projects_created_by ON projects(created_by);
CREATE INDEX idx_projects_dates ON projects(start_date, end_date);

-- Formulas
CREATE INDEX idx_formulas_project ON formulas(project_id);
CREATE INDEX idx_formulas_status ON formulas(status);
CREATE INDEX idx_formulas_code ON formulas(code);

-- Lab Tests
CREATE INDEX idx_lab_tests_formula ON lab_tests(formula_id);
CREATE INDEX idx_lab_tests_status ON lab_tests(status);

-- Experimental Blocks
CREATE INDEX idx_blocks_project ON experimental_blocks(project_id);
CREATE INDEX idx_blocks_formula ON experimental_blocks(formula_id);

-- Experimental Units
CREATE INDEX idx_units_block ON experimental_units(block_id);
CREATE INDEX idx_units_qr ON experimental_units(qr_code_data);

-- Monitoring Data
CREATE INDEX idx_monitoring_session ON monitoring_data(session_id);
CREATE INDEX idx_monitoring_unit ON monitoring_data(unit_id);
CREATE INDEX idx_monitoring_parameter ON monitoring_data(parameter_id);
CREATE INDEX idx_monitoring_collected_at ON monitoring_data(collected_at);

-- Audit Logs
CREATE INDEX idx_audit_user ON audit_logs(user_id);
CREATE INDEX idx_audit_entity ON audit_logs(entity_type, entity_id);
CREATE INDEX idx_audit_created ON audit_logs(created_at);

-- Notifications
CREATE INDEX idx_notifications_user ON user_notifications(user_id);
CREATE INDEX idx_notifications_unread ON user_notifications(user_id, is_read) WHERE is_read = FALSE;

-- ==============================================================================
-- TRIGGERS
-- ==============================================================================

-- Update timestamp function
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = CURRENT_TIMESTAMP;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Apply to tables
CREATE TRIGGER update_organizations_updated_at BEFORE UPDATE ON organizations
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_users_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_projects_updated_at BEFORE UPDATE ON projects
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_formulas_updated_at BEFORE UPDATE ON formulas
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_raw_materials_updated_at BEFORE UPDATE ON raw_materials
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_lab_tests_updated_at BEFORE UPDATE ON lab_tests
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_experimental_blocks_updated_at BEFORE UPDATE ON experimental_blocks
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Formula version trigger
CREATE OR REPLACE FUNCTION create_formula_version()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.status != NEW.status OR OLD.version != NEW.version THEN
        INSERT INTO formula_versions (formula_id, version, changes_summary, full_snapshot, created_by)
        VALUES (
            NEW.id,
            NEW.version,
            CASE
                WHEN OLD.status != NEW.status THEN 'Status changed from ' || OLD.status || ' to ' || NEW.status
                ELSE 'Version updated'
            END,
            row_to_json(OLD)::jsonb,
            NEW.created_by
        );
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER formula_version_trigger AFTER UPDATE ON formulas
    FOR EACH ROW EXECUTE FUNCTION create_formula_version();

-- ==============================================================================
-- INITIAL DATA
-- ==============================================================================

-- Default Organization
INSERT INTO organizations (id, name, code, description)
VALUES (
    '00000000-0000-0000-0000-000000000001',
    'Centra Biotech Indonesia',
    'CBI',
    'Research and Development Division'
);

-- System Configuration
INSERT INTO system_config (config_key, config_value, config_type, description) VALUES
('jwt_secret', 'CHANGE_ME_IN_PRODUCTION', 'string', 'JWT signing secret'),
('jwt_expiry_hours', '24', 'number', 'JWT token expiry in hours'),
('password_min_length', '8', 'number', 'Minimum password length'),
('max_login_attempts', '5', 'number', 'Maximum failed login attempts before lockout'),
('lockout_duration_minutes', '30', 'number', 'Account lockout duration'),
('outlier_std_deviation', '2', 'number', 'Standard deviations for outlier detection'),
('openai_model', 'gpt-4o', 'string', 'OpenAI model for AI analysis'),
('file_upload_max_size_mb', '50', 'number', 'Maximum file upload size in MB');

-- Default Admin User (password: Admin@123)
INSERT INTO users (
    id, organization_id, email, password_hash, full_name, role, is_active, is_email_verified
) VALUES (
    '00000000-0000-0000-0000-000000000002',
    '00000000-0000-0000-0000-000000000001',
    'admin@centrabiotechindonesia.com',
    '$argon2id$v=19$m=19456,t=2,p=1$PLACEHOLDER_HASH',
    'System Administrator',
    'system_admin',
    TRUE,
    TRUE
);

-- Notification Templates
INSERT INTO notification_templates (code, name, subject_template, body_template, channel) VALUES
('project_created', 'Project Created', 'New Project: {{project_title}}', 'A new project has been created: {{project_title}}', 'in_app'),
('formula_qc_passed', 'Formula QC Passed', 'Formula {{formula_code}} Passed QC', 'Formula {{formula_code}} has passed QC and is ready for field testing.', 'in_app'),
('formula_qc_failed', 'Formula QC Failed', 'Formula {{formula_code}} Failed QC', 'Formula {{formula_code}} has failed QC. Reason: {{reason}}', 'in_app'),
('monitoring_reminder', 'Monitoring Reminder', 'Monitoring Due: {{project_title}}', 'Monitoring session {{session_name}} is scheduled for {{date}}.', 'in_app'),
('project_locked', 'Project Locked', 'Project Locked: {{project_title}}', 'Project {{project_title}} has been locked by {{locked_by}}.', 'in_app');

-- ==============================================================================
-- VIEWS
-- ==============================================================================

-- Project Summary View
CREATE OR REPLACE VIEW vw_project_summary AS
SELECT 
    p.id,
    p.code,
    p.title,
    p.status,
    p.start_date,
    p.end_date,
    p.crop_type,
    p.experiment_design,
    o.name AS organization_name,
    u.full_name AS created_by_name,
    (SELECT COUNT(*) FROM formulas WHERE project_id = p.id) AS formula_count,
    (SELECT COUNT(*) FROM formulas WHERE project_id = p.id AND status = 'qc_passed') AS formulas_qc_passed,
    (SELECT COUNT(*) FROM experimental_blocks WHERE project_id = p.id) AS block_count,
    (SELECT COUNT(*) FROM monitoring_sessions WHERE project_id = p.id) AS session_count,
    (SELECT COUNT(*) FROM monitoring_sessions WHERE project_id = p.id AND is_completed = TRUE) AS sessions_completed
FROM projects p
LEFT JOIN organizations o ON p.organization_id = o.id
LEFT JOIN users u ON p.created_by = u.id;

-- Formula with QC Status View
CREATE OR REPLACE VIEW vw_formula_qc_status AS
SELECT 
    f.id,
    f.code,
    f.name,
    f.version,
    f.status,
    f.project_id,
    p.title AS project_title,
    (SELECT COUNT(*) FROM lab_tests WHERE formula_id = f.id) AS total_tests,
    (SELECT COUNT(*) FROM lab_tests WHERE formula_id = f.id AND is_passed = TRUE) AS passed_tests,
    (SELECT COUNT(*) FROM lab_tests WHERE formula_id = f.id AND is_passed = FALSE) AS failed_tests,
    (SELECT COUNT(*) FROM lab_tests WHERE formula_id = f.id AND status = 'pending') AS pending_tests,
    f.qc_approved_by,
    u.full_name AS qc_approved_by_name,
    f.qc_approved_at
FROM formulas f
LEFT JOIN projects p ON f.project_id = p.id
LEFT JOIN users u ON f.qc_approved_by = u.id;

-- Monitoring Progress View
CREATE OR REPLACE VIEW vw_monitoring_progress AS
SELECT 
    p.id AS project_id,
    p.code AS project_code,
    p.title AS project_title,
    ms.id AS session_id,
    ms.session_code,
    ms.scheduled_date,
    ms.is_completed,
    (SELECT COUNT(*) FROM experimental_units eu 
     JOIN experimental_blocks eb ON eu.block_id = eb.id 
     WHERE eb.project_id = p.id AND eu.is_active = TRUE) AS total_units,
    (SELECT COUNT(DISTINCT md.unit_id) FROM monitoring_data md 
     WHERE md.session_id = ms.id) AS units_measured,
    ROUND(
        (SELECT COUNT(DISTINCT md.unit_id)::numeric FROM monitoring_data md 
         WHERE md.session_id = ms.id) * 100.0 / 
        NULLIF((SELECT COUNT(*) FROM experimental_units eu 
         JOIN experimental_blocks eb ON eu.block_id = eb.id 
         WHERE eb.project_id = p.id AND eu.is_active = TRUE), 0),
        2
    ) AS completion_percentage
FROM projects p
LEFT JOIN monitoring_sessions ms ON ms.project_id = p.id;

-- Grant permissions (adjust as needed for your setup)
-- GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO centrabio_app;
-- GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO centrabio_app;
