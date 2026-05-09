//! Tenant Management
//!
//! Comprehensive tenant lifecycle management including CRUD operations,
//! provisioning, usage tracking, and administrative operations.
//!
//! # Features
//!
//! - 📝 **CRUD Operations** - Create, read, update, delete tenants
//! - 🚀 **Provisioning** - Automated resource setup for new tenants
//! - 🔄 **Lifecycle Management** - Activate, suspend, terminate tenants

#![allow(dead_code)]
#![allow(clippy::derivable_impls)]
#![allow(clippy::collapsible_if)]
//! - 📊 **Usage Tracking** - Monitor tenant resource usage
//! - ⚙️ **Configuration** - Per-tenant settings and limits
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use armature_tenancy::management::*;
//!
//! // Create tenant manager with your store
//! let manager = TenantManager::new(store, provisioner);
//!
//! // Create a new tenant
//! let request = CreateTenantRequest::new("acme-corp")
//!     .with_display_name("Acme Corporation")
//!     .with_plan(TenantPlan::Professional);
//!
//! let tenant = manager.create(request).await?;
//!
//! // Manage tenant lifecycle
//! manager.suspend(&tenant.id, "Payment overdue").await?;
//! manager.activate(&tenant.id).await?;
//!
//! // Track usage
//! let usage = manager.get_usage(&tenant.id).await?;
//! ```

use crate::TenantError;
use crate::tenant::Tenant;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Tenant status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantStatus {
    /// Tenant is being provisioned
    Provisioning,
    /// Tenant is active and operational
    Active,
    /// Tenant is temporarily suspended
    Suspended,
    /// Tenant is being terminated
    Terminating,
    /// Tenant has been terminated
    Terminated,
}

impl Default for TenantStatus {
    fn default() -> Self {
        Self::Provisioning
    }
}

impl std::fmt::Display for TenantStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provisioning => write!(f, "provisioning"),
            Self::Active => write!(f, "active"),
            Self::Suspended => write!(f, "suspended"),
            Self::Terminating => write!(f, "terminating"),
            Self::Terminated => write!(f, "terminated"),
        }
    }
}

/// Tenant plan/tier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TenantPlan {
    /// Free tier with limited resources
    #[default]
    Free,
    /// Starter plan for small teams
    Starter,
    /// Professional plan for growing businesses
    Professional,
    /// Enterprise plan with unlimited resources
    Enterprise,
    /// Custom plan with negotiated limits
    Custom,
}

impl std::fmt::Display for TenantPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Free => write!(f, "free"),
            Self::Starter => write!(f, "starter"),
            Self::Professional => write!(f, "professional"),
            Self::Enterprise => write!(f, "enterprise"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

/// Tenant limits based on plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantLimits {
    /// Maximum number of users
    pub max_users: Option<u32>,
    /// Maximum storage in bytes
    pub max_storage_bytes: Option<u64>,
    /// Maximum API requests per month
    pub max_api_requests: Option<u64>,
    /// Maximum concurrent connections
    pub max_connections: Option<u32>,
    /// Custom limits
    pub custom: HashMap<String, serde_json::Value>,
}

impl Default for TenantLimits {
    fn default() -> Self {
        Self {
            max_users: Some(5),
            max_storage_bytes: Some(1024 * 1024 * 1024), // 1GB
            max_api_requests: Some(10_000),
            max_connections: Some(10),
            custom: HashMap::new(),
        }
    }
}

impl TenantLimits {
    /// Create limits for a specific plan
    pub fn for_plan(plan: TenantPlan) -> Self {
        match plan {
            TenantPlan::Free => Self::default(),
            TenantPlan::Starter => Self {
                max_users: Some(25),
                max_storage_bytes: Some(10 * 1024 * 1024 * 1024), // 10GB
                max_api_requests: Some(100_000),
                max_connections: Some(50),
                custom: HashMap::new(),
            },
            TenantPlan::Professional => Self {
                max_users: Some(100),
                max_storage_bytes: Some(100 * 1024 * 1024 * 1024), // 100GB
                max_api_requests: Some(1_000_000),
                max_connections: Some(200),
                custom: HashMap::new(),
            },
            TenantPlan::Enterprise | TenantPlan::Custom => Self {
                max_users: None,         // Unlimited
                max_storage_bytes: None, // Unlimited
                max_api_requests: None,  // Unlimited
                max_connections: None,   // Unlimited
                custom: HashMap::new(),
            },
        }
    }

    /// Set custom limit
    pub fn with_custom(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.custom.insert(key.into(), value);
        self
    }
}

/// Current usage metrics for a tenant
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TenantUsage {
    /// Number of active users
    pub users: u32,
    /// Storage used in bytes
    pub storage_bytes: u64,
    /// API requests this period
    pub api_requests: u64,
    /// Current active connections
    pub active_connections: u32,
    /// Custom usage metrics
    pub custom: HashMap<String, serde_json::Value>,
    /// Last updated timestamp
    pub last_updated: Option<DateTime<Utc>>,
}

impl TenantUsage {
    /// Check if usage exceeds limits
    pub fn exceeds_limits(&self, limits: &TenantLimits) -> Vec<String> {
        let mut violations = Vec::new();

        if let Some(max) = limits.max_users {
            if self.users > max {
                violations.push(format!("Users: {} / {}", self.users, max));
            }
        }

        if let Some(max) = limits.max_storage_bytes {
            if self.storage_bytes > max {
                violations.push(format!("Storage: {} / {} bytes", self.storage_bytes, max));
            }
        }

        if let Some(max) = limits.max_api_requests {
            if self.api_requests > max {
                violations.push(format!("API Requests: {} / {}", self.api_requests, max));
            }
        }

        if let Some(max) = limits.max_connections {
            if self.active_connections > max {
                violations.push(format!(
                    "Connections: {} / {}",
                    self.active_connections, max
                ));
            }
        }

        violations
    }
}

/// Request to create a new tenant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTenantRequest {
    /// Unique slug/identifier (URL-safe)
    pub slug: String,
    /// Display name
    pub display_name: Option<String>,
    /// Custom domain (optional)
    pub domain: Option<String>,
    /// Plan/tier
    pub plan: TenantPlan,
    /// Owner user ID
    pub owner_id: Option<String>,
    /// Initial metadata
    pub metadata: HashMap<String, String>,
    /// Custom limits (overrides plan defaults)
    pub custom_limits: Option<TenantLimits>,
}

impl CreateTenantRequest {
    /// Create a new tenant request
    pub fn new(slug: impl Into<String>) -> Self {
        Self {
            slug: slug.into(),
            display_name: None,
            domain: None,
            plan: TenantPlan::Free,
            owner_id: None,
            metadata: HashMap::new(),
            custom_limits: None,
        }
    }

    /// Set display name
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// Set custom domain
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Set plan
    pub fn with_plan(mut self, plan: TenantPlan) -> Self {
        self.plan = plan;
        self
    }

    /// Set owner
    pub fn with_owner(mut self, owner_id: impl Into<String>) -> Self {
        self.owner_id = Some(owner_id.into());
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Set custom limits
    pub fn with_limits(mut self, limits: TenantLimits) -> Self {
        self.custom_limits = Some(limits);
        self
    }
}

/// Request to update a tenant
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateTenantRequest {
    /// New display name
    pub display_name: Option<String>,
    /// New domain
    pub domain: Option<Option<String>>,
    /// New plan
    pub plan: Option<TenantPlan>,
    /// Metadata to add/update
    pub metadata: Option<HashMap<String, String>>,
    /// New limits
    pub limits: Option<TenantLimits>,
}

impl UpdateTenantRequest {
    /// Create empty update request
    pub fn new() -> Self {
        Self::default()
    }

    /// Set display name
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// Set domain
    pub fn with_domain(mut self, domain: Option<String>) -> Self {
        self.domain = Some(domain);
        self
    }

    /// Set plan
    pub fn with_plan(mut self, plan: TenantPlan) -> Self {
        self.plan = Some(plan);
        self
    }

    /// Set metadata
    pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Set limits
    pub fn with_limits(mut self, limits: TenantLimits) -> Self {
        self.limits = Some(limits);
        self
    }
}

/// Extended tenant with management information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedTenant {
    /// Core tenant information
    pub tenant: Tenant,
    /// Current status
    pub status: TenantStatus,
    /// Plan/tier
    pub plan: TenantPlan,
    /// Resource limits
    pub limits: TenantLimits,
    /// Current usage
    pub usage: TenantUsage,
    /// Owner user ID
    pub owner_id: Option<String>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
    /// Suspension reason (if suspended)
    pub suspension_reason: Option<String>,
}

impl ManagedTenant {
    /// Create from tenant with defaults
    pub fn from_tenant(tenant: Tenant, plan: TenantPlan) -> Self {
        let now = Utc::now();
        Self {
            tenant,
            status: TenantStatus::Provisioning,
            plan,
            limits: TenantLimits::for_plan(plan),
            usage: TenantUsage::default(),
            owner_id: None,
            created_at: now,
            updated_at: now,
            suspension_reason: None,
        }
    }

    /// Check if tenant is operational
    pub fn is_operational(&self) -> bool {
        self.status == TenantStatus::Active
    }

    /// Check if tenant can accept requests
    pub fn can_accept_requests(&self) -> bool {
        matches!(
            self.status,
            TenantStatus::Active | TenantStatus::Provisioning
        )
    }

    /// Get limit violations
    pub fn get_violations(&self) -> Vec<String> {
        self.usage.exceeds_limits(&self.limits)
    }
}

/// Tenant provisioner trait
///
/// Implement this to set up tenant resources (databases, schemas, etc.).
#[async_trait]
pub trait TenantProvisioner: Send + Sync {
    /// Provision resources for a new tenant
    async fn provision(&self, tenant: &ManagedTenant) -> Result<(), TenantError>;

    /// Deprovision resources when tenant is terminated
    async fn deprovision(&self, tenant: &ManagedTenant) -> Result<(), TenantError>;

    /// Suspend tenant resources (optional, default no-op)
    async fn suspend(&self, _tenant: &ManagedTenant) -> Result<(), TenantError> {
        Ok(())
    }

    /// Resume tenant resources (optional, default no-op)
    async fn resume(&self, _tenant: &ManagedTenant) -> Result<(), TenantError> {
        Ok(())
    }
}

/// No-op provisioner for testing or manual provisioning
#[derive(Debug, Clone, Default)]
pub struct NoOpProvisioner;

#[async_trait]
impl TenantProvisioner for NoOpProvisioner {
    async fn provision(&self, _tenant: &ManagedTenant) -> Result<(), TenantError> {
        Ok(())
    }

    async fn deprovision(&self, _tenant: &ManagedTenant) -> Result<(), TenantError> {
        Ok(())
    }
}

/// Managed tenant store trait
///
/// Extend your TenantStore with management operations.
#[async_trait]
pub trait ManagedTenantStore: Send + Sync {
    /// Create a new managed tenant
    async fn create(&self, tenant: &ManagedTenant) -> Result<(), TenantError>;

    /// Get managed tenant by ID
    async fn get(&self, id: &str) -> Result<Option<ManagedTenant>, TenantError>;

    /// Get managed tenant by slug
    async fn get_by_slug(&self, slug: &str) -> Result<Option<ManagedTenant>, TenantError>;

    /// Update managed tenant
    async fn update(&self, tenant: &ManagedTenant) -> Result<(), TenantError>;

    /// Delete managed tenant
    async fn delete(&self, id: &str) -> Result<(), TenantError>;

    /// List all tenants with optional filters
    async fn list(&self, filter: &TenantFilter) -> Result<Vec<ManagedTenant>, TenantError>;

    /// Count tenants matching filter
    async fn count(&self, filter: &TenantFilter) -> Result<u64, TenantError>;

    /// Update usage metrics
    async fn update_usage(&self, id: &str, usage: &TenantUsage) -> Result<(), TenantError>;
}

/// Filter for listing tenants
#[derive(Debug, Clone, Default)]
pub struct TenantFilter {
    /// Filter by status
    pub status: Option<TenantStatus>,
    /// Filter by plan
    pub plan: Option<TenantPlan>,
    /// Filter by owner
    pub owner_id: Option<String>,
    /// Search by name/slug
    pub search: Option<String>,
    /// Pagination offset
    pub offset: u64,
    /// Pagination limit
    pub limit: u64,
}

impl TenantFilter {
    /// Create new filter with defaults
    pub fn new() -> Self {
        Self {
            limit: 50,
            ..Default::default()
        }
    }

    /// Filter by status
    pub fn with_status(mut self, status: TenantStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// Filter by plan
    pub fn with_plan(mut self, plan: TenantPlan) -> Self {
        self.plan = Some(plan);
        self
    }

    /// Filter by owner
    pub fn with_owner(mut self, owner_id: impl Into<String>) -> Self {
        self.owner_id = Some(owner_id.into());
        self
    }

    /// Search by name/slug
    pub fn with_search(mut self, search: impl Into<String>) -> Self {
        self.search = Some(search.into());
        self
    }

    /// Set pagination
    pub fn with_pagination(mut self, offset: u64, limit: u64) -> Self {
        self.offset = offset;
        self.limit = limit;
        self
    }
}

/// Tenant manager
///
/// High-level API for managing tenant lifecycle.
pub struct TenantManager {
    store: Arc<dyn ManagedTenantStore>,
    provisioner: Arc<dyn TenantProvisioner>,
}

impl TenantManager {
    /// Create a new tenant manager
    pub fn new(
        store: Arc<dyn ManagedTenantStore>,
        provisioner: Arc<dyn TenantProvisioner>,
    ) -> Self {
        Self { store, provisioner }
    }

    /// Create a new tenant with no-op provisioner
    pub fn with_store(store: Arc<dyn ManagedTenantStore>) -> Self {
        Self {
            store,
            provisioner: Arc::new(NoOpProvisioner),
        }
    }

    /// Create a new tenant
    pub async fn create(&self, request: CreateTenantRequest) -> Result<ManagedTenant, TenantError> {
        // Check if slug is available
        if self.store.get_by_slug(&request.slug).await?.is_some() {
            return Err(TenantError::Invalid(format!(
                "Tenant slug '{}' already exists",
                request.slug
            )));
        }

        // Generate ID
        let id = uuid::Uuid::new_v4().to_string();

        // Create core tenant
        let mut tenant = Tenant::new(&id, &request.slug);
        if let Some(domain) = &request.domain {
            tenant = tenant.with_domain(domain);
        }
        for (key, value) in &request.metadata {
            tenant = tenant.with_metadata(key, value);
        }

        // Create managed tenant
        let mut managed = ManagedTenant::from_tenant(tenant, request.plan);
        managed.owner_id = request.owner_id;
        if let Some(limits) = request.custom_limits {
            managed.limits = limits;
        }

        // Persist tenant
        self.store.create(&managed).await?;

        // Provision resources
        match self.provisioner.provision(&managed).await {
            Ok(()) => {
                // Update status to active
                managed.status = TenantStatus::Active;
                managed.updated_at = Utc::now();
                self.store.update(&managed).await?;
            }
            Err(e) => {
                // Provisioning failed - mark as terminated and clean up
                managed.status = TenantStatus::Terminated;
                managed.suspension_reason = Some(format!("Provisioning failed: {}", e));
                self.store.update(&managed).await?;
                return Err(e);
            }
        }

        Ok(managed)
    }

    /// Get tenant by ID
    pub async fn get(&self, id: &str) -> Result<Option<ManagedTenant>, TenantError> {
        self.store.get(id).await
    }

    /// Get tenant by slug
    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<ManagedTenant>, TenantError> {
        self.store.get_by_slug(slug).await
    }

    /// Update tenant
    pub async fn update(
        &self,
        id: &str,
        request: UpdateTenantRequest,
    ) -> Result<ManagedTenant, TenantError> {
        let mut managed = self
            .store
            .get(id)
            .await?
            .ok_or_else(|| TenantError::NotFound(id.to_string()))?;

        // Apply updates
        if let Some(name) = request.display_name {
            managed.tenant.name = name;
        }
        if let Some(domain) = request.domain {
            managed.tenant.domain = domain;
        }
        if let Some(plan) = request.plan {
            managed.plan = plan;
            // Update limits if not custom
            if request.limits.is_none() {
                managed.limits = TenantLimits::for_plan(plan);
            }
        }
        if let Some(metadata) = request.metadata {
            managed.tenant.metadata.extend(metadata);
        }
        if let Some(limits) = request.limits {
            managed.limits = limits;
        }
        managed.updated_at = Utc::now();

        self.store.update(&managed).await?;
        Ok(managed)
    }

    /// List tenants
    pub async fn list(&self, filter: &TenantFilter) -> Result<Vec<ManagedTenant>, TenantError> {
        self.store.list(filter).await
    }

    /// Count tenants
    pub async fn count(&self, filter: &TenantFilter) -> Result<u64, TenantError> {
        self.store.count(filter).await
    }

    /// Suspend a tenant
    pub async fn suspend(&self, id: &str, reason: &str) -> Result<ManagedTenant, TenantError> {
        let mut managed = self
            .store
            .get(id)
            .await?
            .ok_or_else(|| TenantError::NotFound(id.to_string()))?;

        if managed.status == TenantStatus::Terminated {
            return Err(TenantError::Invalid(
                "Cannot suspend terminated tenant".to_string(),
            ));
        }

        // Suspend resources
        self.provisioner.suspend(&managed).await?;

        // Update status
        managed.status = TenantStatus::Suspended;
        managed.suspension_reason = Some(reason.to_string());
        managed.tenant.active = false;
        managed.updated_at = Utc::now();

        self.store.update(&managed).await?;
        Ok(managed)
    }

    /// Activate a suspended tenant
    pub async fn activate(&self, id: &str) -> Result<ManagedTenant, TenantError> {
        let mut managed = self
            .store
            .get(id)
            .await?
            .ok_or_else(|| TenantError::NotFound(id.to_string()))?;

        if managed.status == TenantStatus::Terminated {
            return Err(TenantError::Invalid(
                "Cannot activate terminated tenant".to_string(),
            ));
        }

        // Resume resources
        self.provisioner.resume(&managed).await?;

        // Update status
        managed.status = TenantStatus::Active;
        managed.suspension_reason = None;
        managed.tenant.active = true;
        managed.updated_at = Utc::now();

        self.store.update(&managed).await?;
        Ok(managed)
    }

    /// Terminate a tenant (soft delete)
    pub async fn terminate(&self, id: &str, reason: &str) -> Result<ManagedTenant, TenantError> {
        let mut managed = self
            .store
            .get(id)
            .await?
            .ok_or_else(|| TenantError::NotFound(id.to_string()))?;

        if managed.status == TenantStatus::Terminated {
            return Err(TenantError::Invalid(
                "Tenant already terminated".to_string(),
            ));
        }

        // Mark as terminating
        managed.status = TenantStatus::Terminating;
        managed.updated_at = Utc::now();
        self.store.update(&managed).await?;

        // Deprovision resources
        self.provisioner.deprovision(&managed).await?;

        // Mark as terminated
        managed.status = TenantStatus::Terminated;
        managed.suspension_reason = Some(reason.to_string());
        managed.tenant.active = false;
        managed.updated_at = Utc::now();

        self.store.update(&managed).await?;
        Ok(managed)
    }

    /// Permanently delete a tenant (hard delete)
    pub async fn delete(&self, id: &str) -> Result<(), TenantError> {
        let managed = self.store.get(id).await?;

        if let Some(tenant) = managed {
            // Must be terminated first
            if tenant.status != TenantStatus::Terminated {
                return Err(TenantError::Invalid(
                    "Tenant must be terminated before deletion".to_string(),
                ));
            }

            self.store.delete(id).await?;
        }

        Ok(())
    }

    /// Get usage metrics
    pub async fn get_usage(&self, id: &str) -> Result<TenantUsage, TenantError> {
        let managed = self
            .store
            .get(id)
            .await?
            .ok_or_else(|| TenantError::NotFound(id.to_string()))?;

        Ok(managed.usage)
    }

    /// Update usage metrics
    pub async fn update_usage(&self, id: &str, usage: TenantUsage) -> Result<(), TenantError> {
        self.store.update_usage(id, &usage).await
    }

    /// Check if tenant is within limits
    pub async fn check_limits(&self, id: &str) -> Result<Vec<String>, TenantError> {
        let managed = self
            .store
            .get(id)
            .await?
            .ok_or_else(|| TenantError::NotFound(id.to_string()))?;

        Ok(managed.get_violations())
    }
}

/// In-memory tenant store for testing
#[derive(Debug, Default)]
pub struct InMemoryManagedTenantStore {
    tenants: parking_lot::RwLock<HashMap<String, ManagedTenant>>,
}

impl InMemoryManagedTenantStore {
    /// Create new in-memory store
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ManagedTenantStore for InMemoryManagedTenantStore {
    async fn create(&self, tenant: &ManagedTenant) -> Result<(), TenantError> {
        let mut tenants = self.tenants.write();
        if tenants.contains_key(&tenant.tenant.id) {
            return Err(TenantError::Invalid(format!(
                "Tenant {} already exists",
                tenant.tenant.id
            )));
        }
        tenants.insert(tenant.tenant.id.clone(), tenant.clone());
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<ManagedTenant>, TenantError> {
        Ok(self.tenants.read().get(id).cloned())
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<ManagedTenant>, TenantError> {
        Ok(self
            .tenants
            .read()
            .values()
            .find(|t| t.tenant.name == slug)
            .cloned())
    }

    async fn update(&self, tenant: &ManagedTenant) -> Result<(), TenantError> {
        let mut tenants = self.tenants.write();
        if !tenants.contains_key(&tenant.tenant.id) {
            return Err(TenantError::NotFound(tenant.tenant.id.clone()));
        }
        tenants.insert(tenant.tenant.id.clone(), tenant.clone());
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), TenantError> {
        self.tenants.write().remove(id);
        Ok(())
    }

    async fn list(&self, filter: &TenantFilter) -> Result<Vec<ManagedTenant>, TenantError> {
        let tenants = self.tenants.read();
        let mut results: Vec<_> = tenants
            .values()
            .filter(|t| {
                if let Some(status) = filter.status {
                    if t.status != status {
                        return false;
                    }
                }
                if let Some(plan) = filter.plan {
                    if t.plan != plan {
                        return false;
                    }
                }
                if let Some(ref owner) = filter.owner_id {
                    if t.owner_id.as_ref() != Some(owner) {
                        return false;
                    }
                }
                if let Some(ref search) = filter.search {
                    if !t.tenant.name.contains(search) && !t.tenant.id.contains(search) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        // Sort by created_at descending
        results.sort_by_key(|r| std::cmp::Reverse(r.created_at));

        // Apply pagination
        let start = filter.offset as usize;
        let end = (filter.offset + filter.limit) as usize;
        Ok(results.into_iter().skip(start).take(end - start).collect())
    }

    async fn count(&self, filter: &TenantFilter) -> Result<u64, TenantError> {
        let list = self.list(filter).await?;
        Ok(list.len() as u64)
    }

    async fn update_usage(&self, id: &str, usage: &TenantUsage) -> Result<(), TenantError> {
        let mut tenants = self.tenants.write();
        if let Some(tenant) = tenants.get_mut(id) {
            tenant.usage = usage.clone();
            tenant.usage.last_updated = Some(Utc::now());
            Ok(())
        } else {
            Err(TenantError::NotFound(id.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_tenant() {
        let store = Arc::new(InMemoryManagedTenantStore::new());
        let manager = TenantManager::with_store(store);

        let request = CreateTenantRequest::new("acme-corp")
            .with_display_name("Acme Corporation")
            .with_plan(TenantPlan::Professional);

        let tenant = manager.create(request).await.unwrap();
        assert_eq!(tenant.tenant.name, "acme-corp");
        assert_eq!(tenant.plan, TenantPlan::Professional);
        assert_eq!(tenant.status, TenantStatus::Active);
    }

    #[tokio::test]
    async fn test_suspend_activate() {
        let store = Arc::new(InMemoryManagedTenantStore::new());
        let manager = TenantManager::with_store(store);

        let tenant = manager
            .create(CreateTenantRequest::new("test-tenant"))
            .await
            .unwrap();

        // Suspend
        let suspended = manager
            .suspend(&tenant.tenant.id, "Payment overdue")
            .await
            .unwrap();
        assert_eq!(suspended.status, TenantStatus::Suspended);
        assert!(!suspended.tenant.active);

        // Activate
        let activated = manager.activate(&tenant.tenant.id).await.unwrap();
        assert_eq!(activated.status, TenantStatus::Active);
        assert!(activated.tenant.active);
    }

    #[tokio::test]
    async fn test_limits() {
        let limits = TenantLimits::for_plan(TenantPlan::Free);
        let usage = TenantUsage {
            users: 10, // Exceeds limit of 5
            ..Default::default()
        };

        let violations = usage.exceeds_limits(&limits);
        assert!(!violations.is_empty());
    }

    #[tokio::test]
    async fn test_list_filter() {
        let store = Arc::new(InMemoryManagedTenantStore::new());
        let manager = TenantManager::with_store(store);

        // Create tenants with different plans
        manager
            .create(CreateTenantRequest::new("free-tenant").with_plan(TenantPlan::Free))
            .await
            .unwrap();
        manager
            .create(CreateTenantRequest::new("pro-tenant").with_plan(TenantPlan::Professional))
            .await
            .unwrap();

        // Filter by plan
        let filter = TenantFilter::new().with_plan(TenantPlan::Professional);
        let results = manager.list(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tenant.name, "pro-tenant");
    }
}
