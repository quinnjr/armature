-- PostgreSQL Initialization Script for Armature
-- This script runs on first database creation

-- Create extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";
CREATE EXTENSION IF NOT EXISTS "pg_trgm";

-- Create application schema
CREATE SCHEMA IF NOT EXISTS app;

-- Set search path
SET search_path TO app, public;

-- Create audit function
CREATE OR REPLACE FUNCTION app.update_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = CURRENT_TIMESTAMP;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Example users table
CREATE TABLE IF NOT EXISTS app.users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email VARCHAR(255) NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,
    name VARCHAR(255),
    avatar_url VARCHAR(500),
    email_verified BOOLEAN DEFAULT FALSE,
    is_active BOOLEAN DEFAULT TRUE,
    role VARCHAR(50) DEFAULT 'user',
    metadata JSONB DEFAULT '{}',
    last_login_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Create indexes
CREATE INDEX IF NOT EXISTS idx_users_email ON app.users(email);
CREATE INDEX IF NOT EXISTS idx_users_role ON app.users(role);
CREATE INDEX IF NOT EXISTS idx_users_created_at ON app.users(created_at);
CREATE INDEX IF NOT EXISTS idx_users_metadata ON app.users USING GIN(metadata);

-- Add trigger for updated_at
DROP TRIGGER IF EXISTS update_users_updated_at ON app.users;
CREATE TRIGGER update_users_updated_at
    BEFORE UPDATE ON app.users
    FOR EACH ROW
    EXECUTE FUNCTION app.update_updated_at();

-- Sessions table
CREATE TABLE IF NOT EXISTS app.sessions (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES app.users(id) ON DELETE CASCADE,
    token_hash VARCHAR(255) NOT NULL UNIQUE,
    ip_address INET,
    user_agent TEXT,
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON app.sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_token_hash ON app.sessions(token_hash);
CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON app.sessions(expires_at);

-- API keys table
CREATE TABLE IF NOT EXISTS app.api_keys (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES app.users(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    key_hash VARCHAR(255) NOT NULL UNIQUE,
    key_prefix VARCHAR(10) NOT NULL,
    scopes TEXT[] DEFAULT '{}',
    rate_limit INTEGER DEFAULT 1000,
    last_used_at TIMESTAMP WITH TIME ZONE,
    expires_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_api_keys_user_id ON app.api_keys(user_id);
CREATE INDEX IF NOT EXISTS idx_api_keys_key_hash ON app.api_keys(key_hash);

-- Audit log table
CREATE TABLE IF NOT EXISTS app.audit_logs (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID REFERENCES app.users(id),
    action VARCHAR(100) NOT NULL,
    resource_type VARCHAR(100) NOT NULL,
    resource_id UUID,
    old_data JSONB,
    new_data JSONB,
    ip_address INET,
    user_agent TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_user_id ON app.audit_logs(user_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_action ON app.audit_logs(action);
CREATE INDEX IF NOT EXISTS idx_audit_logs_resource ON app.audit_logs(resource_type, resource_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_created_at ON app.audit_logs(created_at);

-- Grant permissions
GRANT USAGE ON SCHEMA app TO armature;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA app TO armature;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA app TO armature;
ALTER DEFAULT PRIVILEGES IN SCHEMA app GRANT ALL ON TABLES TO armature;
ALTER DEFAULT PRIVILEGES IN SCHEMA app GRANT ALL ON SEQUENCES TO armature;

-- Insert default admin user (password: admin123)
INSERT INTO app.users (email, password_hash, name, role, email_verified)
VALUES (
    'admin@example.com',
    '$argon2id$v=19$m=65536,t=3,p=4$c2FsdHNhbHRzYWx0c2FsdA$hash', -- Replace with actual hash
    'Admin User',
    'admin',
    true
) ON CONFLICT (email) DO NOTHING;

COMMENT ON TABLE app.users IS 'Application users';
COMMENT ON TABLE app.sessions IS 'User authentication sessions';
COMMENT ON TABLE app.api_keys IS 'API keys for programmatic access';
COMMENT ON TABLE app.audit_logs IS 'Audit trail for compliance';

