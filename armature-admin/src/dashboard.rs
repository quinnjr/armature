//! Dashboard views for admin

use crate::{AdminInstance, QuickAction, StatCard};
use serde::{Deserialize, Serialize};

/// Dashboard view data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardView {
    /// Page title
    pub title: String,
    /// Statistics cards
    pub stats: Vec<StatCard>,
    /// Quick actions
    pub quick_actions: Vec<QuickAction>,
    /// Model summaries
    pub model_summaries: Vec<ModelSummary>,
}

impl DashboardView {
    /// Create a new dashboard view
    pub fn new(admin: &AdminInstance) -> Self {
        let model_summaries = admin
            .models()
            .iter()
            .map(|m| ModelSummary {
                name: m.name.clone(),
                verbose_name: m.verbose_name.clone(),
                icon: m.icon.clone(),
                count: 0, // Would be populated from database
                recent_count: 0,
                url: format!("{}/{}", admin.config.base_path, m.name),
            })
            .collect();

        Self {
            title: admin.config.title.clone(),
            stats: vec![StatCard {
                title: "Total Records".to_string(),
                value: "0".to_string(),
                change: None,
                icon: Some("database".to_string()),
                color: None,
                link: None,
            }],
            quick_actions: admin
                .models()
                .iter()
                .filter(|m| m.can_add)
                .take(4)
                .map(|m| QuickAction {
                    label: format!("Add {}", m.verbose_name_singular),
                    url: format!("{}/{}/add", admin.config.base_path, m.name),
                    icon: Some("plus".to_string()),
                    css_class: None,
                })
                .collect(),
            model_summaries,
        }
    }

    /// Set statistics
    pub fn with_stats(mut self, stats: Vec<StatCard>) -> Self {
        self.stats = stats;
        self
    }
}

/// Model summary for dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSummary {
    /// Model name
    pub name: String,
    /// Verbose name
    pub verbose_name: String,
    /// Icon
    pub icon: Option<String>,
    /// Total record count
    pub count: usize,
    /// Recent record count
    pub recent_count: usize,
    /// URL to list view
    pub url: String,
}
