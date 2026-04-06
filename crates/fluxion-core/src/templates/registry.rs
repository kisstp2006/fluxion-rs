// ============================================================
// fluxion-core — Template Registry
//
// Central registry for all available project templates.
// Handles template discovery, metadata, and instantiation.
// ============================================================

use super::{ProjectTemplate, TemplateMetadata, TemplateCategory};
use std::collections::HashMap;
use std::sync::Arc;

/// Global template registry
pub struct TemplateRegistry {
    templates: HashMap<String, Arc<dyn ProjectTemplate>>,
    metadata: HashMap<String, TemplateMetadata>,
}

impl TemplateRegistry {
    /// Create a new template registry with built-in templates
    pub fn new() -> Self {
        let mut registry = Self {
            templates: HashMap::new(),
            metadata: HashMap::new(),
        };
        
        // Register built-in templates
        registry.register_builtin_templates();
        registry
    }
    
    /// Register a template with the registry
    pub fn register<T: ProjectTemplate + 'static>(&mut self, template: T) {
        let id = template.template_id().to_string();
        let metadata = template.metadata();
        
        self.templates.insert(id.clone(), Arc::new(template));
        self.metadata.insert(id, metadata);
    }
    
    /// Get all templates in a category
    pub fn get_by_category(&self, category: TemplateCategory) -> Vec<&TemplateMetadata> {
        self.metadata
            .values()
            .filter(|m| m.category == category)
            .collect()
    }
    
    /// Get all templates
    pub fn get_all(&self) -> Vec<&TemplateMetadata> {
        self.metadata.values().collect()
    }
    
    /// Get template by ID
    pub fn get_template(&self, id: &str) -> Option<Arc<dyn ProjectTemplate>> {
        self.templates.get(id).cloned()
    }
    
    /// Get template metadata by ID
    pub fn get_metadata(&self, id: &str) -> Option<&TemplateMetadata> {
        self.metadata.get(id)
    }
    
    /// Search templates by query
    pub fn search(&self, query: &str) -> Vec<&TemplateMetadata> {
        let query = query.to_lowercase();
        self.metadata
            .values()
            .filter(|m| {
                m.name.to_lowercase().contains(&query)
                    || m.description.to_lowercase().contains(&query)
                    || m.tags.iter().any(|t| t.to_lowercase().contains(&query))
            })
            .collect()
    }
    
    /// Register all built-in templates
    fn register_builtin_templates(&mut self) {
        use super::templates::*;
        
        // Empty templates
        self.register(Empty3DTemplate);
        self.register(Empty2DTemplate);
        
        // 3D templates
        self.register(Basic3DTemplate);
        self.register(FPSTemplate);
        self.register(ThirdPersonTemplate);
        self.register(VehicleTemplate);
        
        // 2D templates
        self.register(PlatformerTemplate);
        self.register(TopDownTemplate);
        self.register(PuzzleTemplate);
        
        // VR templates
        self.register(VRBasicTemplate);
        self.register(VRInteractionTemplate);
        
        // Mobile templates
        self.register(MobileGameTemplate);
        
        // Educational templates
        self.register(BasicScriptingTemplate);
        self.register(PhysicsDemoTemplate);
    }
}

impl Default for TemplateRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global template registry instance
static mut GLOBAL_REGISTRY: Option<TemplateRegistry> = None;
static REGISTRY_INIT: std::sync::Once = std::sync::Once::new();

/// Get the global template registry
pub fn get_template_registry() -> &'static TemplateRegistry {
    unsafe {
        REGISTRY_INIT.call_once(|| {
            GLOBAL_REGISTRY = Some(TemplateRegistry::new());
        });
        GLOBAL_REGISTRY.as_ref().unwrap()
    }
}

/// Get all template IDs
pub fn get_template_ids() -> Vec<String> {
    get_template_registry().metadata.keys().cloned().collect()
}

/// Get template metadata by ID
pub fn get_template_metadata(id: &str) -> Option<&TemplateMetadata> {
    get_template_registry().get_metadata(id)
}
