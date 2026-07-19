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
use std::sync::{Arc, OnceLock};

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
    /// Lazily-computed, sorted cache of resource URIs. The registry is
    /// populated once at startup from `inventory` and never mutated
    /// afterward, so this cache — built on the first paginated list
    /// call — stays valid for the registry's whole lifetime and avoids
    /// re-sorting the full key set on every `list_resources_page` call.
    sorted_uris: OnceLock<Vec<String>>,
}

impl McpResourceRegistry {
    /// Create a new registry and collect all registered resources
    pub fn new() -> Self {
        let mut resources = HashMap::new();

        for entry in inventory::iter::<McpResourceEntry> {
            resources.insert(entry.uri.to_string(), entry);
        }

        Self {
            resources,
            sorted_uris: OnceLock::new(),
        }
    }

    /// Get all registered resource definitions
    pub fn list_resources(&self) -> Vec<ResourceDefinition> {
        self.resources
            .values()
            .map(|entry| entry.to_definition())
            .collect()
    }

    /// Get a page of registered resource definitions, ordered
    /// deterministically by URI.
    ///
    /// `cursor` is an opaque token: the URI of the last item returned by
    /// the previous page (or `None` to start from the beginning). Returns
    /// the page of resources together with the `next_cursor` to pass in to
    /// fetch the following page, or `None` if this was the last page.
    pub fn list_resources_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> (Vec<ResourceDefinition>, Option<String>) {
        let uris = self.sorted_uris.get_or_init(|| {
            let mut uris: Vec<String> = self.resources.keys().cloned().collect();
            uris.sort();
            uris
        });

        let start = match cursor {
            Some(c) => uris.partition_point(|uri| uri.as_str() <= c),
            None => 0,
        };

        let remaining = &uris[start..];
        let take = remaining.len().min(limit.max(1));
        let page = &remaining[..take];

        let next_cursor = if take < remaining.len() {
            Some(page[take - 1].clone())
        } else {
            None
        };

        let defs = page
            .iter()
            .map(|uri| self.resources[uri.as_str()].to_definition())
            .collect();

        (defs, next_cursor)
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

#[cfg(test)]
mod tests {
    use super::*;

    struct PageTestOwner;

    fn page_test_handler() -> Pin<Box<dyn Future<Output = Result<ResourceContent>> + Send>> {
        Box::pin(async { Ok(ResourceContent::text("test://uri", "test")) })
    }

    /// Build a registry with `count` resources named `res://res_00`,
    /// `res://res_01`, ... so pagination has a deterministically ordered
    /// set to page through.
    fn registry_with_resources(count: usize) -> McpResourceRegistry {
        let mut resources = HashMap::new();
        for i in 0..count {
            let uri: &'static str = Box::leak(format!("res://res_{i:02}").into_boxed_str());
            let entry: &'static McpResourceEntry =
                Box::leak(Box::new(McpResourceEntry::new::<PageTestOwner>(
                    uri,
                    uri,
                    None,
                    None,
                    Arc::new(page_test_handler),
                )));
            resources.insert(uri.to_string(), entry);
        }
        McpResourceRegistry {
            resources,
            sorted_uris: OnceLock::new(),
        }
    }

    #[test]
    fn test_list_resources_page_first_page_has_cursor() {
        let registry = registry_with_resources(10);

        let (page, next_cursor) = registry.list_resources_page(None, 4);

        assert_eq!(page.len(), 4);
        assert_eq!(page[0].uri, "res://res_00");
        assert_eq!(page[3].uri, "res://res_03");
        assert_eq!(next_cursor, Some("res://res_03".to_string()));
    }

    #[test]
    fn test_list_resources_page_cursor_returns_next_page() {
        let registry = registry_with_resources(10);

        let (first, cursor) = registry.list_resources_page(None, 4);
        let cursor = cursor.expect("first page should have a cursor");
        let (second, _) = registry.list_resources_page(Some(&cursor), 4);

        assert_eq!(second.len(), 4);
        assert_eq!(second[0].uri, "res://res_04");
        assert_eq!(second[3].uri, "res://res_07");

        let first_uris: std::collections::HashSet<_> =
            first.iter().map(|r| r.uri.clone()).collect();
        let second_uris: std::collections::HashSet<_> =
            second.iter().map(|r| r.uri.clone()).collect();
        assert!(first_uris.is_disjoint(&second_uris));
    }

    #[test]
    fn test_list_resources_page_final_page_has_no_cursor() {
        let registry = registry_with_resources(10);

        let (page, cursor) = registry.list_resources_page(Some("res://res_07"), 4);

        assert_eq!(page.len(), 2);
        assert_eq!(page[0].uri, "res://res_08");
        assert_eq!(page[1].uri, "res://res_09");
        assert_eq!(cursor, None);
    }

    #[test]
    fn test_list_resources_page_union_equals_full_list_no_dupes() {
        let registry = registry_with_resources(10);

        let mut collected = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let (page, next_cursor) = registry.list_resources_page(cursor.as_deref(), 3);
            collected.extend(page.into_iter().map(|r| r.uri));
            match next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }

        let mut expected: Vec<String> = (0..10).map(|i| format!("res://res_{i:02}")).collect();
        expected.sort();

        assert_eq!(collected, expected);

        let unique: std::collections::HashSet<_> = collected.iter().collect();
        assert_eq!(unique.len(), collected.len());
    }
}
