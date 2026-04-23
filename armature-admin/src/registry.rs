//! Model registry for admin

use crate::model::ModelDefinition;
use std::collections::HashMap;

/// Registry of admin models
#[derive(Debug, Default)]
pub struct ModelRegistry {
    /// Registered models by name
    models: HashMap<String, ModelDefinition>,
    /// Model ordering (for sidebar)
    order: Vec<String>,
    /// Model groups
    groups: HashMap<String, Vec<String>>,
}

impl ModelRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
            order: Vec::new(),
            groups: HashMap::new(),
        }
    }

    /// Register a model
    pub fn register(&mut self, model: ModelDefinition) {
        let name = model.name.clone();
        self.order.push(name.clone());
        self.models.insert(name, model);
    }

    /// Register a model in a group
    pub fn register_in_group(&mut self, group: impl Into<String>, model: ModelDefinition) {
        let group = group.into();
        let name = model.name.clone();

        self.models.insert(name.clone(), model);

        if !self.order.contains(&name) {
            self.order.push(name.clone());
        }

        self.groups.entry(group).or_default().push(name);
    }

    /// Get a model by name
    pub fn get(&self, name: &str) -> Option<&ModelDefinition> {
        self.models.get(name)
    }

    /// Get all models
    pub fn all(&self) -> Vec<&ModelDefinition> {
        self.order
            .iter()
            .filter_map(|name| self.models.get(name))
            .collect()
    }

    /// Get model names in order
    pub fn names(&self) -> &[String] {
        &self.order
    }

    /// Get models in a group
    pub fn group(&self, name: &str) -> Vec<&ModelDefinition> {
        self.groups
            .get(name)
            .map(|names| names.iter().filter_map(|n| self.models.get(n)).collect())
            .unwrap_or_default()
    }

    /// Get all groups
    pub fn groups(&self) -> &HashMap<String, Vec<String>> {
        &self.groups
    }

    /// Check if a model exists
    pub fn contains(&self, name: &str) -> bool {
        self.models.contains_key(name)
    }

    /// Get model count
    pub fn count(&self) -> usize {
        self.models.len()
    }

    /// Remove a model
    pub fn unregister(&mut self, name: &str) -> Option<ModelDefinition> {
        self.order.retain(|n| n != name);
        for group in self.groups.values_mut() {
            group.retain(|n| n != name);
        }
        self.models.remove(name)
    }

    /// Get models for sidebar navigation
    pub fn sidebar_items(&self) -> Vec<SidebarItem> {
        // If there are groups, organize by group
        if !self.groups.is_empty() {
            let mut items = Vec::new();
            let mut ungrouped = Vec::new();

            // Add grouped models
            for (group_name, model_names) in &self.groups {
                let models: Vec<_> = model_names
                    .iter()
                    .filter_map(|n| self.models.get(n))
                    .map(|m| SidebarItem::Model {
                        name: m.name.clone(),
                        label: m.verbose_name.clone(),
                        icon: m.icon.clone(),
                    })
                    .collect();

                if !models.is_empty() {
                    items.push(SidebarItem::Group {
                        name: group_name.clone(),
                        items: models,
                    });
                }
            }

            // Add ungrouped models
            for name in &self.order {
                let in_group = self.groups.values().any(|g| g.contains(name));
                if !in_group {
                    if let Some(model) = self.models.get(name) {
                        ungrouped.push(SidebarItem::Model {
                            name: model.name.clone(),
                            label: model.verbose_name.clone(),
                            icon: model.icon.clone(),
                        });
                    }
                }
            }

            items.extend(ungrouped);
            items
        } else {
            // Just list all models
            self.order
                .iter()
                .filter_map(|name| self.models.get(name))
                .map(|m| SidebarItem::Model {
                    name: m.name.clone(),
                    label: m.verbose_name.clone(),
                    icon: m.icon.clone(),
                })
                .collect()
        }
    }
}

/// Sidebar navigation item
#[derive(Debug, Clone)]
pub enum SidebarItem {
    /// Single model
    Model {
        name: String,
        label: String,
        icon: Option<String>,
    },
    /// Group of models
    Group {
        name: String,
        items: Vec<SidebarItem>,
    },
    /// Divider
    Divider,
    /// Custom link
    Link {
        label: String,
        url: String,
        icon: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::{FieldDefinition, FieldType};

    #[test]
    fn test_registry() {
        let mut registry = ModelRegistry::new();

        let user = ModelDefinition::builder("user")
            .id_field()
            .field(FieldDefinition::new("name", FieldType::String))
            .build();

        let post = ModelDefinition::builder("post")
            .id_field()
            .field(FieldDefinition::new("title", FieldType::String))
            .build();

        registry.register(user);
        registry.register(post);

        assert_eq!(registry.count(), 2);
        assert!(registry.contains("user"));
        assert!(registry.contains("post"));
        assert!(!registry.contains("comment"));
    }

    #[test]
    fn test_registry_groups() {
        let mut registry = ModelRegistry::new();

        let user = ModelDefinition::builder("user").id_field().build();

        let role = ModelDefinition::builder("role").id_field().build();

        registry.register_in_group("Auth", user);
        registry.register_in_group("Auth", role);

        let auth_models = registry.group("Auth");
        assert_eq!(auth_models.len(), 2);
    }
}
