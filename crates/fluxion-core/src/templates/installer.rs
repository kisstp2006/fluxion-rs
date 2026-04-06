// ============================================================
// fluxion-core — Template Installer
//
// Handles the actual installation of templates into
// project directories with progress tracking.
// ============================================================

use super::{ProjectTemplate, TemplateOptions};
use crate::project::ProjectConfig;
use std::path::Path;
use std::time::{Duration, Instant};

/// Progress information for template installation
#[derive(Debug, Clone)]
pub struct InstallProgress {
    /// Current step (0-based)
    pub current_step: usize,
    /// Total number of steps
    pub total_steps: usize,
    /// Name of current step
    pub step_name: String,
    /// Progress percentage (0-100)
    pub percentage: f32,
    /// Time elapsed since start
    pub elapsed: Duration,
}

/// Template installer with progress tracking
pub struct TemplateInstaller {
    template_id: String,
    options: TemplateOptions,
    start_time: Instant,
    current_step: usize,
    total_steps: usize,
}

impl TemplateInstaller {
    /// Create a new installer for the given template
    pub fn new(template_id: String, options: TemplateOptions) -> Result<Self, String> {
        let registry = super::registry::get_template_registry();
        if !registry.get_metadata(&template_id).is_some() {
            return Err(format!("Template '{}' not found", template_id));
        }
        
        Ok(Self {
            template_id,
            options,
            start_time: Instant::now(),
            current_step: 0,
            total_steps: 4, // project, directories, files, scenes
        })
    }
    
    /// Get current installation progress
    pub fn progress(&self) -> InstallProgress {
        let percentage = (self.current_step as f32 / self.total_steps as f32) * 100.0;
        let step_names = ["Creating project config", "Creating directories", "Creating files", "Creating scenes"];
        let step_name = if self.current_step < step_names.len() {
            step_names[self.current_step].to_string()
        } else {
            "Complete".to_string()
        };
        
        InstallProgress {
            current_step: self.current_step,
            total_steps: self.total_steps,
            step_name,
            percentage,
            elapsed: self.start_time.elapsed(),
        }
    }
    
    /// Install the template (blocking)
    pub fn install(mut self) -> Result<(), String> {
        let registry = super::registry::get_template_registry();
        let template = registry.get_template(&self.template_id)
            .ok_or_else(|| format!("Template '{}' not found", self.template_id))?;
        
        // Step 1: Create project configuration
        let project_config = template.create_project(&self.options)?;
        self.save_project_config(&project_config)?;
        self.current_step += 1;
        
        // Step 2: Create directory structure
        let base_path = Path::new(&self.options.directory);
        template.create_directories(base_path, &self.options)?;
        self.current_step += 1;
        
        // Step 3: Create files
        template.create_files(base_path, &self.options)?;
        self.current_step += 1;
        
        // Step 4: Create scenes
        template.create_scenes(base_path, &self.options)?;
        self.current_step += 1;
        
        Ok(())
    }
    
    /// Save project configuration to .fluxproj file
    fn save_project_config(&self, config: &crate::project::ProjectConfig) -> Result<(), String> {
        let config_path = Path::new(&self.options.directory).join(".fluxproj");
        let config_json = serde_json::to_string_pretty(config)
            .map_err(|e| format!("Failed to serialize project config: {}", e))?;
        
        std::fs::write(&config_path, config_json)
            .map_err(|e| format!("Failed to write project config: {}", e))?;
        
        Ok(())
    }
}

/// Quick install function for simple use cases
pub fn install_template(template_id: &str, name: &str, directory: &str) -> Result<(), String> {
    let options = TemplateOptions {
        name: name.to_string(),
        directory: directory.to_string(),
        custom_options: std::collections::HashMap::new(),
    };
    
    let installer = TemplateInstaller::new(template_id.to_string(), options)?;
    installer.install()
}

/// Validate a template installation
pub fn validate_installation(directory: &str) -> Result<Vec<String>, String> {
    let mut issues = Vec::new();
    let base_path = Path::new(directory);
    
    // Check .fluxproj exists
    let proj_file = base_path.join(".fluxproj");
    if !proj_file.exists() {
        issues.push("Missing .fluxproj file".to_string());
    }
    
    // Check Assets directory
    let assets_dir = base_path.join("Assets");
    if !assets_dir.exists() {
        issues.push("Missing Assets directory".to_string());
    }
    
    // Check standard subdirectories
    let required_dirs = ["Scenes", "Materials", "Scripts"];
    for dir in required_dirs {
        let dir_path = assets_dir.join(dir);
        if !dir_path.exists() {
            issues.push(format!("Missing Assets/{} directory", dir));
        }
    }
    
    if issues.is_empty() {
        Ok(issues)
    } else {
        Err(format!("Installation validation failed: {}", issues.join(", ")))
    }
}
