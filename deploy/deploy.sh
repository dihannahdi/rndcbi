#!/bin/bash
# ============================================================================
# CENTRABIO R&D NEXUS - Deployment Script
# ============================================================================
# This script deploys the application to a production server.
# Run from local machine: ./deploy.sh
# 
# Prerequisites:
# - SSH access to server configured (ssh hostinger)
# - Rust toolchain installed on server
# - PostgreSQL running on server
# - Nginx installed on server
# ============================================================================

set -euo pipefail

# Configuration
REMOTE_HOST="hostinger"
REMOTE_USER="ubuntu"
DEPLOY_DIR="/opt/centrabio-nexus"
SERVICE_NAME="centrabio-nexus"
BACKUP_DIR="/opt/backups/centrabio-nexus"
RUST_TARGET="x86_64-unknown-linux-gnu"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
log_warning() { echo -e "${YELLOW}[WARNING]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

# Check prerequisites
check_prerequisites() {
    log_info "Checking prerequisites..."
    
    # Check if cargo is installed
    if ! command -v cargo &> /dev/null; then
        log_error "Cargo not found. Please install Rust toolchain."
    fi
    
    # Check SSH connection
    if ! ssh -o ConnectTimeout=5 "$REMOTE_HOST" exit 2>/dev/null; then
        log_error "Cannot connect to $REMOTE_HOST. Check SSH configuration."
    fi
    
    log_success "Prerequisites check passed"
}

# Build the application
build_release() {
    log_info "Building release binary..."
    
    # Build for production
    cargo build --release --target "$RUST_TARGET"
    
    if [ ! -f "target/$RUST_TARGET/release/$SERVICE_NAME" ]; then
        log_error "Build failed. Binary not found."
    fi
    
    log_success "Build completed successfully"
}

# Prepare deployment package
prepare_package() {
    log_info "Preparing deployment package..."
    
    # Create temp directory
    TEMP_DIR=$(mktemp -d)
    trap "rm -rf $TEMP_DIR" EXIT
    
    # Copy binary
    cp "target/$RUST_TARGET/release/$SERVICE_NAME" "$TEMP_DIR/"
    
    # Copy static files
    cp -r static "$TEMP_DIR/"
    
    # Copy migrations
    cp -r migrations "$TEMP_DIR/"
    
    # Copy deployment files
    cp deploy/centrabio-nexus.service "$TEMP_DIR/"
    cp deploy/nginx.conf "$TEMP_DIR/"
    
    # Copy .env.example as template
    cp .env.example "$TEMP_DIR/.env.template"
    
    # Create tarball
    tar -czf deployment.tar.gz -C "$TEMP_DIR" .
    
    log_success "Deployment package created"
}

# Deploy to server
deploy_to_server() {
    log_info "Deploying to server..."
    
    # Upload package
    scp deployment.tar.gz "$REMOTE_HOST:/tmp/"
    
    # Execute remote deployment
    ssh "$REMOTE_HOST" << 'REMOTE_SCRIPT'
        set -e
        
        DEPLOY_DIR="/opt/centrabio-nexus"
        BACKUP_DIR="/opt/backups/centrabio-nexus"
        SERVICE_NAME="centrabio-nexus"
        
        echo "[REMOTE] Creating directories..."
        sudo mkdir -p "$DEPLOY_DIR"
        sudo mkdir -p "$BACKUP_DIR"
        sudo mkdir -p "$DEPLOY_DIR/uploads"
        sudo mkdir -p "$DEPLOY_DIR/reports"
        sudo mkdir -p /var/log/centrabio-nexus
        
        # Backup current version if exists
        if [ -f "$DEPLOY_DIR/$SERVICE_NAME" ]; then
            echo "[REMOTE] Backing up current version..."
            BACKUP_NAME="backup_$(date +%Y%m%d_%H%M%S).tar.gz"
            sudo tar -czf "$BACKUP_DIR/$BACKUP_NAME" -C "$DEPLOY_DIR" . 2>/dev/null || true
            
            # Keep only last 5 backups
            ls -t "$BACKUP_DIR"/*.tar.gz 2>/dev/null | tail -n +6 | xargs -r sudo rm
        fi
        
        # Stop service if running
        if systemctl is-active --quiet "$SERVICE_NAME"; then
            echo "[REMOTE] Stopping service..."
            sudo systemctl stop "$SERVICE_NAME"
        fi
        
        # Extract new version
        echo "[REMOTE] Extracting new version..."
        sudo tar -xzf /tmp/deployment.tar.gz -C "$DEPLOY_DIR"
        sudo chmod +x "$DEPLOY_DIR/$SERVICE_NAME"
        
        # Update systemd service
        echo "[REMOTE] Updating systemd service..."
        sudo cp "$DEPLOY_DIR/centrabio-nexus.service" /etc/systemd/system/
        sudo systemctl daemon-reload
        
        # Set ownership
        sudo chown -R ubuntu:ubuntu "$DEPLOY_DIR"
        sudo chown -R ubuntu:ubuntu /var/log/centrabio-nexus
        
        # Create .env from template if not exists
        if [ ! -f "$DEPLOY_DIR/.env" ]; then
            echo "[REMOTE] Creating .env from template..."
            sudo cp "$DEPLOY_DIR/.env.template" "$DEPLOY_DIR/.env"
            echo "[REMOTE] IMPORTANT: Edit $DEPLOY_DIR/.env with production values!"
        fi
        
        # Run database migrations
        echo "[REMOTE] Running database migrations..."
        cd "$DEPLOY_DIR"
        # The app will run migrations on startup via SQLx
        
        # Start service
        echo "[REMOTE] Starting service..."
        sudo systemctl start "$SERVICE_NAME"
        sudo systemctl enable "$SERVICE_NAME"
        
        # Wait for service to be ready
        sleep 3
        
        # Health check
        if curl -s http://localhost:8083/health | grep -q "ok"; then
            echo "[REMOTE] Service is healthy!"
        else
            echo "[REMOTE] WARNING: Health check failed. Check logs with: journalctl -u $SERVICE_NAME -f"
        fi
        
        # Cleanup
        rm /tmp/deployment.tar.gz
        
        echo "[REMOTE] Deployment completed!"
REMOTE_SCRIPT
    
    log_success "Deployment completed"
}

# Configure Nginx
configure_nginx() {
    log_info "Configuring Nginx..."
    
    ssh "$REMOTE_HOST" << 'REMOTE_SCRIPT'
        set -e
        
        DEPLOY_DIR="/opt/centrabio-nexus"
        
        # Check if nginx is installed
        if ! command -v nginx &> /dev/null; then
            echo "[REMOTE] Installing Nginx..."
            sudo apt-get update
            sudo apt-get install -y nginx
        fi
        
        # Copy nginx config
        echo "[REMOTE] Installing Nginx configuration..."
        sudo cp "$DEPLOY_DIR/nginx.conf" /etc/nginx/sites-available/centrabio-nexus
        
        # Enable site
        sudo ln -sf /etc/nginx/sites-available/centrabio-nexus /etc/nginx/sites-enabled/
        
        # Test configuration
        sudo nginx -t
        
        # Reload nginx
        sudo systemctl reload nginx
        
        echo "[REMOTE] Nginx configured!"
REMOTE_SCRIPT
    
    log_success "Nginx configured"
}

# Setup SSL with Let's Encrypt
setup_ssl() {
    log_info "Setting up SSL certificate..."
    
    ssh "$REMOTE_HOST" << 'REMOTE_SCRIPT'
        set -e
        
        DOMAIN="nexus.centrabio.id"
        
        # Check if certbot is installed
        if ! command -v certbot &> /dev/null; then
            echo "[REMOTE] Installing Certbot..."
            sudo apt-get update
            sudo apt-get install -y certbot python3-certbot-nginx
        fi
        
        # Create webroot directory
        sudo mkdir -p /var/www/certbot
        
        # Obtain certificate
        echo "[REMOTE] Obtaining SSL certificate for $DOMAIN..."
        sudo certbot --nginx -d "$DOMAIN" --non-interactive --agree-tos --email admin@centrabio.id --redirect
        
        # Setup auto-renewal
        echo "[REMOTE] Setting up auto-renewal..."
        sudo systemctl enable certbot.timer
        sudo systemctl start certbot.timer
        
        echo "[REMOTE] SSL certificate installed!"
REMOTE_SCRIPT
    
    log_success "SSL certificate configured"
}

# Setup PostgreSQL database
setup_database() {
    log_info "Setting up database..."
    
    ssh "$REMOTE_HOST" << 'REMOTE_SCRIPT'
        set -e
        
        DB_NAME="centrabio_nexus"
        DB_USER="centrabio"
        
        echo "[REMOTE] Creating database and user..."
        
        # Create user and database
        sudo -u postgres psql -p 5433 << EOF
            -- Create user if not exists
            DO \$\$
            BEGIN
                IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = '$DB_USER') THEN
                    CREATE USER $DB_USER WITH ENCRYPTED PASSWORD 'CHANGE_THIS_PASSWORD';
                END IF;
            END
            \$\$;
            
            -- Create database if not exists
            SELECT 'CREATE DATABASE $DB_NAME OWNER $DB_USER'
            WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = '$DB_NAME')\gexec
            
            -- Grant privileges
            GRANT ALL PRIVILEGES ON DATABASE $DB_NAME TO $DB_USER;
            
            -- Connect to database and create extensions
            \c $DB_NAME
            CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
            CREATE EXTENSION IF NOT EXISTS "pgcrypto";
EOF
        
        echo "[REMOTE] Database setup completed!"
        echo "[REMOTE] IMPORTANT: Update the database password in /opt/centrabio-nexus/.env"
REMOTE_SCRIPT
    
    log_success "Database setup completed"
}

# View logs
view_logs() {
    log_info "Showing recent logs..."
    ssh "$REMOTE_HOST" "sudo journalctl -u $SERVICE_NAME -n 100 --no-pager"
}

# Rollback to previous version
rollback() {
    log_info "Rolling back to previous version..."
    
    ssh "$REMOTE_HOST" << 'REMOTE_SCRIPT'
        set -e
        
        DEPLOY_DIR="/opt/centrabio-nexus"
        BACKUP_DIR="/opt/backups/centrabio-nexus"
        SERVICE_NAME="centrabio-nexus"
        
        # Find latest backup
        LATEST_BACKUP=$(ls -t "$BACKUP_DIR"/*.tar.gz 2>/dev/null | head -n 1)
        
        if [ -z "$LATEST_BACKUP" ]; then
            echo "[REMOTE] No backup found!"
            exit 1
        fi
        
        echo "[REMOTE] Rolling back to: $LATEST_BACKUP"
        
        # Stop service
        sudo systemctl stop "$SERVICE_NAME"
        
        # Restore backup
        sudo tar -xzf "$LATEST_BACKUP" -C "$DEPLOY_DIR"
        
        # Start service
        sudo systemctl start "$SERVICE_NAME"
        
        echo "[REMOTE] Rollback completed!"
REMOTE_SCRIPT
    
    log_success "Rollback completed"
}

# Show usage
usage() {
    echo "CENTRABIO R&D NEXUS Deployment Script"
    echo ""
    echo "Usage: $0 <command>"
    echo ""
    echo "Commands:"
    echo "  deploy      Full deployment (build + deploy)"
    echo "  build       Build release binary only"
    echo "  upload      Upload and deploy (skip build)"
    echo "  nginx       Configure Nginx only"
    echo "  ssl         Setup SSL certificate"
    echo "  database    Setup PostgreSQL database"
    echo "  logs        View service logs"
    echo "  rollback    Rollback to previous version"
    echo "  status      Check service status"
    echo ""
}

# Check service status
check_status() {
    log_info "Checking service status..."
    ssh "$REMOTE_HOST" << 'REMOTE_SCRIPT'
        echo "=== Service Status ==="
        sudo systemctl status centrabio-nexus --no-pager || true
        
        echo ""
        echo "=== Health Check ==="
        curl -s http://localhost:8083/health || echo "Health check failed"
        
        echo ""
        echo "=== Recent Logs ==="
        sudo journalctl -u centrabio-nexus -n 20 --no-pager
REMOTE_SCRIPT
}

# Main
main() {
    case "${1:-}" in
        deploy)
            check_prerequisites
            build_release
            prepare_package
            deploy_to_server
            ;;
        build)
            check_prerequisites
            build_release
            ;;
        upload)
            check_prerequisites
            prepare_package
            deploy_to_server
            ;;
        nginx)
            configure_nginx
            ;;
        ssl)
            setup_ssl
            ;;
        database)
            setup_database
            ;;
        logs)
            view_logs
            ;;
        rollback)
            rollback
            ;;
        status)
            check_status
            ;;
        *)
            usage
            exit 1
            ;;
    esac
}

main "$@"
