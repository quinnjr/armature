//! Admin configuration

use serde::{Deserialize, Serialize};

/// Admin dashboard configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminConfig {
    /// Dashboard title
    pub title: String,
    /// Base URL path (e.g., "/admin")
    pub base_path: String,
    /// Theme settings
    pub theme: Theme,
    /// Items per page for lists
    pub items_per_page: usize,
    /// Maximum items per page allowed
    pub max_items_per_page: usize,
    /// Require authentication
    pub require_auth: bool,
    /// Enable search globally
    pub enable_search: bool,
    /// Enable export functionality
    pub enable_export: bool,
    /// Date format
    pub date_format: String,
    /// DateTime format
    pub datetime_format: String,
    /// Logo URL
    pub logo_url: Option<String>,
    /// Favicon URL
    pub favicon_url: Option<String>,
    /// Custom CSS
    pub custom_css: Option<String>,
    /// Custom JavaScript
    pub custom_js: Option<String>,
    /// Footer text
    pub footer_text: Option<String>,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            title: "Admin Dashboard".to_string(),
            base_path: "/admin".to_string(),
            theme: Theme::default(),
            items_per_page: 25,
            max_items_per_page: 100,
            require_auth: true,
            enable_search: true,
            enable_export: true,
            date_format: "%Y-%m-%d".to_string(),
            datetime_format: "%Y-%m-%d %H:%M:%S".to_string(),
            logo_url: None,
            favicon_url: None,
            custom_css: None,
            custom_js: None,
            footer_text: None,
        }
    }
}

/// Theme configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    /// Theme name/preset
    pub name: ThemePreset,
    /// Primary color
    pub primary_color: String,
    /// Secondary color
    pub secondary_color: String,
    /// Accent color
    pub accent_color: String,
    /// Background color
    pub background_color: String,
    /// Surface color (cards, etc.)
    pub surface_color: String,
    /// Text color
    pub text_color: String,
    /// Muted text color
    pub text_muted_color: String,
    /// Border color
    pub border_color: String,
    /// Success color
    pub success_color: String,
    /// Warning color
    pub warning_color: String,
    /// Error color
    pub error_color: String,
    /// Sidebar width
    pub sidebar_width: String,
    /// Border radius
    pub border_radius: String,
    /// Font family
    pub font_family: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Dark theme preset
    pub fn dark() -> Self {
        Self {
            name: ThemePreset::Dark,
            primary_color: "#6366f1".to_string(),    // Indigo
            secondary_color: "#8b5cf6".to_string(),  // Violet
            accent_color: "#22d3ee".to_string(),     // Cyan
            background_color: "#0f172a".to_string(), // Slate 900
            surface_color: "#1e293b".to_string(),    // Slate 800
            text_color: "#f8fafc".to_string(),       // Slate 50
            text_muted_color: "#94a3b8".to_string(), // Slate 400
            border_color: "#334155".to_string(),     // Slate 700
            success_color: "#22c55e".to_string(),    // Green
            warning_color: "#f59e0b".to_string(),    // Amber
            error_color: "#ef4444".to_string(),      // Red
            sidebar_width: "260px".to_string(),
            border_radius: "0.5rem".to_string(),
            font_family: "'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
                .to_string(),
        }
    }

    /// Light theme preset
    pub fn light() -> Self {
        Self {
            name: ThemePreset::Light,
            primary_color: "#4f46e5".to_string(),    // Indigo
            secondary_color: "#7c3aed".to_string(),  // Violet
            accent_color: "#0891b2".to_string(),     // Cyan
            background_color: "#f8fafc".to_string(), // Slate 50
            surface_color: "#ffffff".to_string(),    // White
            text_color: "#0f172a".to_string(),       // Slate 900
            text_muted_color: "#64748b".to_string(), // Slate 500
            border_color: "#e2e8f0".to_string(),     // Slate 200
            success_color: "#16a34a".to_string(),    // Green
            warning_color: "#d97706".to_string(),    // Amber
            error_color: "#dc2626".to_string(),      // Red
            sidebar_width: "260px".to_string(),
            border_radius: "0.5rem".to_string(),
            font_family: "'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
                .to_string(),
        }
    }

    /// Corporate blue theme
    pub fn corporate() -> Self {
        Self {
            name: ThemePreset::Corporate,
            primary_color: "#2563eb".to_string(),    // Blue
            secondary_color: "#1d4ed8".to_string(),  // Blue darker
            accent_color: "#0ea5e9".to_string(),     // Sky
            background_color: "#f1f5f9".to_string(), // Slate 100
            surface_color: "#ffffff".to_string(),
            text_color: "#1e293b".to_string(),       // Slate 800
            text_muted_color: "#64748b".to_string(), // Slate 500
            border_color: "#cbd5e1".to_string(),     // Slate 300
            success_color: "#059669".to_string(),    // Emerald
            warning_color: "#ca8a04".to_string(),    // Yellow
            error_color: "#dc2626".to_string(),      // Red
            sidebar_width: "240px".to_string(),
            border_radius: "0.375rem".to_string(),
            font_family:
                "'IBM Plex Sans', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
                    .to_string(),
        }
    }

    /// Generate CSS variables
    pub fn to_css_variables(&self) -> String {
        format!(
            r#":root {{
  --admin-primary: {};
  --admin-secondary: {};
  --admin-accent: {};
  --admin-bg: {};
  --admin-surface: {};
  --admin-text: {};
  --admin-text-muted: {};
  --admin-border: {};
  --admin-success: {};
  --admin-warning: {};
  --admin-error: {};
  --admin-sidebar-width: {};
  --admin-radius: {};
  --admin-font: {};
}}"#,
            self.primary_color,
            self.secondary_color,
            self.accent_color,
            self.background_color,
            self.surface_color,
            self.text_color,
            self.text_muted_color,
            self.border_color,
            self.success_color,
            self.warning_color,
            self.error_color,
            self.sidebar_width,
            self.border_radius,
            self.font_family,
        )
    }
}

/// Theme presets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemePreset {
    #[default]
    Dark,
    Light,
    Corporate,
    Custom,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AdminConfig::default();
        assert_eq!(config.title, "Admin Dashboard");
        assert_eq!(config.base_path, "/admin");
        assert_eq!(config.items_per_page, 25);
    }

    #[test]
    fn test_theme_presets() {
        let dark = Theme::dark();
        assert_eq!(dark.name, ThemePreset::Dark);

        let light = Theme::light();
        assert_eq!(light.name, ThemePreset::Light);
    }

    #[test]
    fn test_css_variables() {
        let theme = Theme::dark();
        let css = theme.to_css_variables();
        assert!(css.contains("--admin-primary:"));
        assert!(css.contains("--admin-bg:"));
    }
}
