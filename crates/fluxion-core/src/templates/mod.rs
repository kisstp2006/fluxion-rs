// ============================================================
// fluxion-core — Project Template System
//
// Unity-inspired project templates with predefined assets,
// scenes, and configurations for common game types.
// ============================================================

pub mod registry;
pub mod templates;
pub mod installer;

pub use registry::TemplateRegistry;
pub use templates::*;
pub use installer::TemplateInstaller;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Template metadata for display in project chooser
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMetadata {
    /// Human-readable template name
    pub name: String,
    /// Short description (1-2 sentences)
    pub description: String,
    /// Longer description with features
    pub long_description: String,
    /// Category for organization
    pub category: TemplateCategory,
    /// Relative path to thumbnail image (optional)
    pub thumbnail: Option<String>,
    /// Tags for filtering/searching
    pub tags: Vec<String>,
    /// Estimated difficulty level
    pub difficulty: TemplateDifficulty,
    /// Estimated project size (small/medium/large)
    pub size: TemplateSize,
}

/// Template categories for organization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateCategory {
    /// Empty or minimal projects
    Empty,
    /// 3D games and applications
    ThreeD,
    /// 2D games and applications
    TwoD,
    /// Virtual Reality projects
    VR,
    /// Mobile-optimized projects
    Mobile,
    /// Educational or tutorial projects
    Educational,
}

/// Difficulty level for templates
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateDifficulty {
    Beginner,
    Intermediate,
    Advanced,
}

/// Project size estimation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateSize {
    Small,
    Medium,
    Large,
}

/// Template installation options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateOptions {
    /// Project name
    pub name: String,
    /// Target directory
    pub directory: String,
    /// Custom template-specific options
    pub custom_options: HashMap<String, serde_json::Value>,
}

impl Default for TemplateOptions {
    fn default() -> Self {
        Self {
            name: "NewProject".to_string(),
            directory: ".".to_string(),
            custom_options: HashMap::new(),
        }
    }
}
