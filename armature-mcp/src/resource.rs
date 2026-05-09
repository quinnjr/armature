//! MCP Resource definitions and registry
//!
//! Resources are data that can be exposed to language models through the MCP protocol.
//! They are identified by URIs and can contain text or binary content.

use crate::error::{McpError, Result};
use crate::types::{ResourceContent, ResourceDefinition};
use async_trait::async_trait;
use std::any::TypeId;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Handler function type for MCP resource reads
pub type ResourceHandlerFn =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<ResourceContent>> + Send>> + Send + Sync>;

/// An MCP resource entry registered at compile time
pub struct McpResourceEntry {
    /// URI of the resource
    pub uri: &'static str,
    /// Human-readable name
    pub name: &'static str,
    /// Optional description
    pub description: Option<&'static str>,
    /// MIME type of the resource
    pub mime_type: Option<&'static str>,
    /// Handler to read the resource content
    pub handler: ResourceHandlerFn,
    /// Type ID of the struct this resource belongs to
    pub owner_type_id: TypeId,
}

inventory::collect!(McpResourceEntry);

impl McpResourceEntry {
    /// Create a new resource entry
    pub fn new<T: 'static>(
        uri: &'static str,
        name: &'static str,
        description: Option<&'static str>,
        mime_type: Option<&'static str>,
        handler: ResourceHandlerFn,
    ) -> Self {
        Self {
            uri,
            name,
            description,
            mime_type,
            handler,
            owner_type_id: TypeId::of::<T>(),
        }
    }

    /// Convert to a ResourceDefinition for the protocol
    pub fn to_definition(&self) -> ResourceDefinition {
        ResourceDefinition {
            uri: self.uri.to_string(),
            name: self.name.to_string(),
            description: self.description.map(|s| s.to_string()),
            mime_type: self.mime_type.map(|s| s.to_string()),
        }
    }

    /// Read the resource content
    pub async fn read(&self) -> Result<ResourceContent> {
        (self.handler)().await
    }
}

/// Trait for types that provide MCP resources
#[async_trait]
pub trait McpResourceProvider: Send + Sync {
    /// Get resource definitions provided by this type
    fn resources(&self) -> Vec<ResourceDefinition>;

    /// Read a resource by URI
    async fn read_resource(&self, uri: &str) -> Result<ResourceContent>;
}

/// Registry for MCP resources collected at compile time
#[derive(Default)]
pub struct McpResourceRegistry {
    resources: HashMap<String, &'static McpResourceEntry>,
}

impl McpResourceRegistry {
    /// Create a new registry and collect all registered resources
    pub fn new() -> Self {
        let mut resources = HashMap::new();

        for entry in inventory::iter::<McpResourceEntry> {
            resources.insert(entry.uri.to_string(), entry);
        }

        Self { resources }
    }

    /// Get all registered resource definitions
    pub fn list_resources(&self) -> Vec<ResourceDefinition> {
        self.resources
            .values()
            .map(|entry| entry.to_definition())
            .collect()
    }

    /// Get a resource by URI
    pub fn get_resource(&self, uri: &str) -> Option<&'static McpResourceEntry> {
        self.resources.get(uri).copied()
    }

    /// Read a resource by URI
    pub async fn read_resource(&self, uri: &str) -> Result<ResourceContent> {
        let entry = self
            .resources
            .get(uri)
            .ok_or_else(|| McpError::ResourceNotFound(uri.to_string()))?;

        entry.read().await
    }

    /// Check if a resource exists
    pub fn has_resource(&self, uri: &str) -> bool {
        self.resources.contains_key(uri)
    }

    /// Get the number of registered resources
    pub fn len(&self) -> usize {
        self.resources.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }
}

/// Macro to register an MCP resource at compile time
#[macro_export]
macro_rules! register_mcp_resource {
    ($owner:ty, $uri:expr, $name:expr, $description:expr, $mime_type:expr, $handler:expr) => {
        $crate::inventory::submit! {
            $crate::resource::McpResourceEntry::new::<$owner>(
                $uri,
                $name,
                Some($description),
                Some($mime_type),
                std::sync::Arc::new(move || {
                    Box::pin($handler())
                }),
            )
        }
    };
    ($owner:ty, $uri:expr, $name:expr, $handler:expr) => {
        $crate::inventory::submit! {
            $crate::resource::McpResourceEntry::new::<$owner>(
                $uri,
                $name,
                None,
                None,
                std::sync::Arc::new(move || {
                    Box::pin($handler())
                }),
            )
        }
    };
}
