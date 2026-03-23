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
    /// Recent activity
    pub recent_activity: Vec<ActivityItem>,
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
            recent_activity: Vec::new(),
            model_summaries,
        }
    }

    /// Set statistics
    pub fn with_stats(mut self, stats: Vec<StatCard>) -> Self {
        self.stats = stats;
        self
    }

    /// Set recent activity
    pub fn with_activity(mut self, activity: Vec<ActivityItem>) -> Self {
        self.recent_activity = activity;
        self
    }
}

/// Activity item for dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityItem {
    /// Action type
    pub action: ActivityAction,
    /// Model name
    pub model: String,
    /// Record identifier/name
    pub record: String,
    /// Record ID
    pub record_id: String,
    /// User who performed the action
    pub user: Option<String>,
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// URL to the record
    pub url: Option<String>,
}

impl ActivityItem {
    /// Create a new activity item
    pub fn new(
        action: ActivityAction,
        model: impl Into<String>,
        record: impl Into<String>,
    ) -> Self {
        Self {
            action,
            model: model.into(),
            record: record.into(),
            record_id: String::new(),
            user: None,
            timestamp: chrono::Utc::now(),
            url: None,
        }
    }

    /// Set record ID
    pub fn record_id(mut self, id: impl Into<String>) -> Self {
        self.record_id = id.into();
        self
    }

    /// Set user
    pub fn user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Set URL
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Get action description
    pub fn description(&self) -> String {
        match self.action {
            ActivityAction::Create => format!("Created {} \"{}\"", self.model, self.record),
            ActivityAction::Update => format!("Updated {} \"{}\"", self.model, self.record),
            ActivityAction::Delete => format!("Deleted {} \"{}\"", self.model, self.record),
            ActivityAction::View => format!("Viewed {} \"{}\"", self.model, self.record),
        }
    }
}

/// Activity action type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityAction {
    Create,
    Update,
    Delete,
    View,
}

impl ActivityAction {
    /// Get icon for action
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Create => "plus-circle",
            Self::Update => "edit",
            Self::Delete => "trash",
            Self::View => "eye",
        }
    }

    /// Get color for action
    pub fn color(&self) -> &'static str {
        match self {
            Self::Create => "success",
            Self::Update => "warning",
            Self::Delete => "error",
            Self::View => "info",
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_description() {
        let activity = ActivityItem::new(ActivityAction::Create, "User", "Alice");
        assert_eq!(activity.description(), "Created User \"Alice\"");
    }

    #[test]
    fn test_activity_action_icon() {
        assert_eq!(ActivityAction::Create.icon(), "plus-circle");
        assert_eq!(ActivityAction::Delete.icon(), "trash");
    }
}
