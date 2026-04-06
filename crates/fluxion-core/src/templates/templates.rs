// ============================================================
// fluxion-core — Built-in Project Templates
//
// Collection of predefined project templates for common
// game types and use cases.
// ============================================================

use super::{TemplateMetadata, TemplateCategory, TemplateDifficulty, TemplateSize, TemplateOptions};
use crate::project::ProjectConfig;
use std::path::Path;

/// Trait for project templates
pub trait ProjectTemplate: Send + Sync {
    /// Unique template identifier
    fn template_id(&self) -> &str;
    
    /// Template metadata
    fn metadata(&self) -> TemplateMetadata;
    
    /// Create project from this template
    fn create_project(&self, options: &TemplateOptions) -> Result<ProjectConfig, String>;
    
    /// Create initial directory structure
    fn create_directories(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String>;
    
    /// Create initial files and assets
    fn create_files(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String>;
    
    /// Create initial scene(s)
    fn create_scenes(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String>;
}

// ============================================================================
// Empty Templates
// ============================================================================

pub struct Empty3DTemplate;

impl ProjectTemplate for Empty3DTemplate {
    fn template_id(&self) -> &str {
        "empty_3d"
    }
    
    fn metadata(&self) -> TemplateMetadata {
        TemplateMetadata {
            name: "Empty 3D Project".to_string(),
            description: "Blank 3D project with basic scene setup".to_string(),
            long_description: "Start with an empty 3D scene containing a camera, light, and ground plane. Perfect for custom projects.".to_string(),
            category: TemplateCategory::Empty,
            thumbnail: Some("templates/thumbnails/empty_3d.png".to_string()),
            tags: vec!["3d".to_string(), "empty".to_string(), "basic".to_string()],
            difficulty: TemplateDifficulty::Beginner,
            size: TemplateSize::Small,
        }
    }
    
    fn create_project(&self, options: &TemplateOptions) -> Result<ProjectConfig, String> {
        let mut settings = crate::project::ProjectSettings::default();
        settings.build.window_title = options.name.clone();
        
        Ok(ProjectConfig {
            name: options.name.clone(),
            version: "1.0.0".to_string(),
            engine: "FluxionRS".to_string(),
            schema: 1,
            default_scene: format!("Assets/Scenes/{}.scene", options.name),
            settings,
        })
    }
    
    fn create_directories(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String> {
        let dirs = [
            "Assets",
            "Assets/Scenes",
            "Assets/Materials",
            "Assets/Models",
            "Assets/Textures",
            "Assets/Audio",
            "Assets/Scripts",
            "Assets/Prefabs",
        ];
        
        for dir in dirs {
            std::fs::create_dir_all(base_path.join(dir))
                .map_err(|e| format!("Failed to create directory {}: {}", dir, e))?;
        }
        
        Ok(())
    }
    
    fn create_files(&self, _base_path: &Path, _options: &TemplateOptions) -> Result<(), String> {
        // No additional files for empty template
        Ok(())
    }
    
    fn create_scenes(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String> {
        let scene_content = r#"{
  "entities": [
    {
      "id": 1,
      "name": "Main Camera",
      "components": {
        "Transform": {
          "position": [0, 2, -5],
          "rotation": [0, 0, 0, 1],
          "scale": [1, 1, 1]
        },
        "Camera": {
          "projection_mode": "Perspective",
          "fov": 60.0,
          "near": 0.1,
          "far": 1000.0
        }
      }
    },
    {
      "id": 2,
      "name": "Directional Light",
      "components": {
        "Transform": {
          "position": [0, 10, 0],
          "rotation": [0.7071, 0, -0.7071, 0],
          "scale": [1, 1, 1]
        },
        "Light": {
          "light_type": "Directional",
          "color": [1, 1, 1],
          "intensity": 1.0,
          "shadows": true
        }
      }
    },
    {
      "id": 3,
      "name": "Ground",
      "components": {
        "Transform": {
          "position": [0, -1, 0],
          "rotation": [0, 0, 0, 1],
          "scale": [10, 1, 10]
        },
        "MeshRenderer": {
          "mesh_path": "Models/PrimitiveCube.glb",
          "material_path": "Materials/Ground.fluxmat"
        }
      }
    }
  ]
}"#;
        
        let scene_path = base_path.join("Assets/Scenes").join(format!("{}.scene", options.name));
        std::fs::write(&scene_path, scene_content)
            .map_err(|e| format!("Failed to write scene file: {}", e))?;
        
        Ok(())
    }
}

pub struct Empty2DTemplate;

impl ProjectTemplate for Empty2DTemplate {
    fn template_id(&self) -> &str {
        "empty_2d"
    }
    
    fn metadata(&self) -> TemplateMetadata {
        TemplateMetadata {
            name: "Empty 2D Project".to_string(),
            description: "Blank 2D project with orthographic camera setup".to_string(),
            long_description: "Start with an empty 2D scene with orthographic camera and basic lighting. Ideal for 2D games.".to_string(),
            category: TemplateCategory::Empty,
            thumbnail: Some("templates/thumbnails/empty_2d.png".to_string()),
            tags: vec!["2d".to_string(), "empty".to_string(), "basic".to_string()],
            difficulty: TemplateDifficulty::Beginner,
            size: TemplateSize::Small,
        }
    }
    
    fn create_project(&self, options: &TemplateOptions) -> Result<ProjectConfig, String> {
        let mut settings = crate::project::ProjectSettings::default();
        settings.build.window_title = options.name.clone();
        
        Ok(ProjectConfig {
            name: options.name.clone(),
            version: "1.0.0".to_string(),
            engine: "FluxionRS".to_string(),
            schema: 1,
            default_scene: format!("Assets/Scenes/{}.scene", options.name),
            settings,
        })
    }
    
    fn create_directories(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String> {
        let dirs = [
            "Assets",
            "Assets/Scenes",
            "Assets/Materials",
            "Assets/Sprites",
            "Assets/Textures",
            "Assets/Audio",
            "Assets/Scripts",
            "Assets/Prefabs",
            "Assets/Tilemaps",
        ];
        
        for dir in dirs {
            std::fs::create_dir_all(base_path.join(dir))
                .map_err(|e| format!("Failed to create directory {}: {}", dir, e))?;
        }
        
        Ok(())
    }
    
    fn create_files(&self, _base_path: &Path, _options: &TemplateOptions) -> Result<(), String> {
        Ok(())
    }
    
    fn create_scenes(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String> {
        let scene_content = r#"{
  "entities": [
    {
      "id": 1,
      "name": "Main Camera",
      "components": {
        "Transform": {
          "position": [0, 0, -10],
          "rotation": [0, 0, 0, 1],
          "scale": [1, 1, 1]
        },
        "Camera": {
          "projection_mode": "Orthographic",
          "fov": 5.0,
          "near": 0.1,
          "far": 1000.0
        }
      }
    }
  ]
}"#;
        
        let scene_path = base_path.join("Assets/Scenes").join(format!("{}.scene", options.name));
        std::fs::write(&scene_path, scene_content)
            .map_err(|e| format!("Failed to write scene file: {}", e))?;
        
        Ok(())
    }
}

// ============================================================================
// 3D Game Templates
// ============================================================================

pub struct Basic3DTemplate;

impl ProjectTemplate for Basic3DTemplate {
    fn template_id(&self) -> &str {
        "basic_3d"
    }
    
    fn metadata(&self) -> TemplateMetadata {
        TemplateMetadata {
            name: "Basic 3D Game".to_string(),
            description: "3D project with player controller and interactive objects".to_string(),
            long_description: "A complete 3D game setup with character controller, collectibles, and basic gameplay mechanics.".to_string(),
            category: TemplateCategory::ThreeD,
            thumbnail: Some("templates/thumbnails/basic_3d.png".to_string()),
            tags: vec!["3d".to_string(), "character".to_string(), "gameplay".to_string()],
            difficulty: TemplateDifficulty::Beginner,
            size: TemplateSize::Medium,
        }
    }
    
    fn create_project(&self, options: &TemplateOptions) -> Result<ProjectConfig, String> {
        let mut settings = crate::project::ProjectSettings::default();
        settings.build.window_title = options.name.clone();
        
        Ok(ProjectConfig {
            name: options.name.clone(),
            version: "1.0.0".to_string(),
            engine: "FluxionRS".to_string(),
            schema: 1,
            default_scene: format!("Assets/Scenes/{}.scene", options.name),
            settings,
        })
    }
    
    fn create_directories(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String> {
        let dirs = [
            "Assets",
            "Assets/Scenes",
            "Assets/Materials",
            "Assets/Models",
            "Assets/Textures",
            "Assets/Audio",
            "Assets/Scripts",
            "Assets/Prefabs",
            "Assets/Animations",
        ];
        
        for dir in dirs {
            std::fs::create_dir_all(base_path.join(dir))
                .map_err(|e| format!("Failed to create directory {}: {}", dir, e))?;
        }
        
        Ok(())
    }
    
    fn create_files(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String> {
        // Create player controller script
        let player_script = r#"pub fn start() {
    // Initialize player
}

pub fn update(dt) {
    let id = fluxion::script::self_entity();
    
    // Get input
    let forward = fluxion::input::get_axis("Vertical");
    let right = fluxion::input::get_axis("Horizontal");
    let jump = fluxion::input::get_key_down("Space");
    
    // Movement
    let speed = 5.0;
    let move_vec = [right * speed, 0.0, forward * speed];
    
    let current_pos = fluxion::entity::get_position(id);
    let new_pos = [
        current_pos[0] + move_vec[0] * dt,
        current_pos[1],
        current_pos[2] + move_vec[2] * dt
    ];
    
    fluxion::entity::set_position(id, new_pos);
    
    // Jump
    if jump {
        let current_vel = fluxion::entity::get_velocity(id);
        fluxion::entity::set_velocity(id, [current_vel[0], 5.0, current_vel[2]]);
    }
}
"#;
        
        let script_path = base_path.join("Assets/Scripts").join("player_controller.rn");
        std::fs::write(&script_path, player_script)
            .map_err(|e| format!("Failed to write player script: {}", e))?;
        
        Ok(())
    }
    
    fn create_scenes(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String> {
        let scene_content = r#"{
  "entities": [
    {
      "id": 1,
      "name": "Main Camera",
      "components": {
        "Transform": {
          "position": [0, 3, -8],
          "rotation": [0.216, 0, 0, 0.976],
          "scale": [1, 1, 1]
        },
        "Camera": {
          "projection_mode": "Perspective",
          "fov": 60.0,
          "near": 0.1,
          "far": 1000.0
        }
      }
    },
    {
      "id": 2,
      "name": "Directional Light",
      "components": {
        "Transform": {
          "position": [0, 10, 0],
          "rotation": [0.7071, 0, -0.7071, 0],
          "scale": [1, 1, 1]
        },
        "Light": {
          "light_type": "Directional",
          "color": [1, 1, 1],
          "intensity": 1.0,
          "shadows": true
        }
      }
    },
    {
      "id": 3,
      "name": "Player",
      "components": {
        "Transform": {
          "position": [0, 1, 0],
          "rotation": [0, 0, 0, 1],
          "scale": [1, 1, 1]
        },
        "MeshRenderer": {
          "mesh_path": "Models/PlayerCapsule.glb",
          "material_path": "Materials/Player.fluxmat"
        },
        "RigidBody": {
          "mass": 1.0,
          "use_gravity": true,
          "is_kinematic": false,
          "freeze_position": [false, false, false],
          "freeze_rotation": [true, true, true]
        },
        "Collider": {
          "shape": "Capsule",
          "radius": 0.5,
          "height": 2.0,
          "center": [0, 0, 0]
        },
        "ScriptBundle": {
          "scripts": [
            {
              "name": "PlayerController",
              "path": "Scripts/player_controller.rn",
              "enabled": true
            }
          ]
        }
      }
    },
    {
      "id": 4,
      "name": "Ground",
      "components": {
        "Transform": {
          "position": [0, -1, 0],
          "rotation": [0, 0, 0, 1],
          "scale": [20, 1, 20]
        },
        "MeshRenderer": {
          "mesh_path": "Models/PrimitiveCube.glb",
          "material_path": "Materials/Ground.fluxmat"
        },
        "RigidBody": {
          "mass": 0.0,
          "use_gravity": true,
          "is_kinematic": true,
          "freeze_position": [true, true, true],
          "freeze_rotation": [true, true, true]
        },
        "Collider": {
          "shape": "Box",
          "size": [1, 1, 1],
          "center": [0, 0, 0]
        }
      }
    }
  ]
}"#;
        
        let scene_path = base_path.join("Assets/Scenes").join(format!("{}.scene", options.name));
        std::fs::write(&scene_path, scene_content)
            .map_err(|e| format!("Failed to write scene file: {}", e))?;
        
        Ok(())
    }
}

// Placeholder templates for other types - these would be implemented similarly
pub struct FPSTemplate;
pub struct ThirdPersonTemplate;
pub struct VehicleTemplate;
pub struct PlatformerTemplate;
pub struct TopDownTemplate;
pub struct PuzzleTemplate;
pub struct VRBasicTemplate;
pub struct VRInteractionTemplate;
pub struct MobileGameTemplate;
pub struct BasicScriptingTemplate;
pub struct PhysicsDemoTemplate;

// Implement basic trait for placeholder templates
macro_rules! impl_placeholder_template {
    ($struct_name:ident, $id:expr, $name:expr, $description:expr, $category:expr) => {
        impl ProjectTemplate for $struct_name {
            fn template_id(&self) -> &str { $id }
            
            fn metadata(&self) -> TemplateMetadata {
                TemplateMetadata {
                    name: $name.to_string(),
                    description: $description.to_string(),
                    long_description: $description.to_string(),
                    category: $category,
                    thumbnail: None,
                    tags: vec![],
                    difficulty: TemplateDifficulty::Beginner,
                    size: TemplateSize::Medium,
                }
            }
            
            fn create_project(&self, options: &TemplateOptions) -> Result<ProjectConfig, String> {
                let mut settings = crate::project::ProjectSettings::default();
                settings.build.window_title = options.name.clone();
                
                Ok(ProjectConfig {
                    name: options.name.clone(),
                    version: "1.0.0".to_string(),
                    engine: "FluxionRS".to_string(),
                    schema: 1,
                    default_scene: format!("Assets/Scenes/{}.scene", options.name),
                    settings,
                })
            }
            
            fn create_directories(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String> {
                Empty3DTemplate.create_directories(base_path, options)
            }
            
            fn create_files(&self, _base_path: &Path, _options: &TemplateOptions) -> Result<(), String> {
                Ok(())
            }
            
            fn create_scenes(&self, base_path: &Path, options: &TemplateOptions) -> Result<(), String> {
                Empty3DTemplate.create_scenes(base_path, options)
            }
        }
    };
}

impl_placeholder_template!(FPSTemplate, "fps", "FPS Game", "First-person shooter template", TemplateCategory::ThreeD);
impl_placeholder_template!(ThirdPersonTemplate, "third_person", "Third-Person Game", "Third-person adventure template", TemplateCategory::ThreeD);
impl_placeholder_template!(VehicleTemplate, "vehicle", "Vehicle Game", "Vehicle simulation template", TemplateCategory::ThreeD);
impl_placeholder_template!(PlatformerTemplate, "platformer", "Platformer Game", "2D platformer template", TemplateCategory::TwoD);
impl_placeholder_template!(TopDownTemplate, "topdown", "Top-Down Game", "Top-down game template", TemplateCategory::TwoD);
impl_placeholder_template!(PuzzleTemplate, "puzzle", "Puzzle Game", "Puzzle game template", TemplateCategory::TwoD);
impl_placeholder_template!(VRBasicTemplate, "vr_basic", "VR Basic", "Basic VR template", TemplateCategory::VR);
impl_placeholder_template!(VRInteractionTemplate, "vr_interaction", "VR Interaction", "VR interaction template", TemplateCategory::VR);
impl_placeholder_template!(MobileGameTemplate, "mobile", "Mobile Game", "Mobile game template", TemplateCategory::Mobile);
impl_placeholder_template!(BasicScriptingTemplate, "scripting", "Scripting Demo", "Scripting demonstration", TemplateCategory::Educational);
impl_placeholder_template!(PhysicsDemoTemplate, "physics", "Physics Demo", "Physics demonstration", TemplateCategory::Educational);
