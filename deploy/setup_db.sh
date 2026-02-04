#!/bin/bash
# Setup database for CENTRABIO R&D NEXUS

sudo -u postgres psql -p 5433 << 'EOF'
-- Create user
CREATE USER centrabio WITH PASSWORD 'CentraBio2025SecurePass' CREATEDB;

-- Create database
CREATE DATABASE centrabio_nexus OWNER centrabio;
EOF

sudo -u postgres psql -p 5433 -d centrabio_nexus << 'EOF'
-- Create extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Grant privileges
GRANT ALL PRIVILEGES ON DATABASE centrabio_nexus TO centrabio;
EOF

echo "Database setup complete!"
