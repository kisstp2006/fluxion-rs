// ============================================================
// fluxion-core — ComponentRegistry
//
// A runtime registry that maps component type-name strings to
// factory functions. This replaces the hard-coded match statement
// in deserialize_world.rs and makes the scene/prefab loader
// extensible: any crate (renderer, physics, scripting, editor)
// can register its own component types without touching core.
//
// Usage
// ─────
//   // At startup, build a registry with all known component types:
//   let mut registry = ComponentRegistry::new();
//   registry.register_builtins();                  // Transform, MeshRenderer, Camera, Light, ParticleEmitter
//   registry.register("MyComp", |data, world, id| {
//       let comp: MyComp = serde_json::from_value(data.clone())?;
//       world.add_component(id, comp);
//       Ok(())
//   });
//
//   // Pass to the scene loader:
//   load_scene_into_world(&mut world, &scene, true, &registry)?;
//
// Design notes
// ────────────
// - Factories receive the raw `serde_json::Value` from the scene file
//   and mutably borrow the world so they can call `world.add_component`.
// - Factories return `Result<(), String>` — an Err causes a warning log
//   and the component is skipped (same behaviour as before).
// - The registry is cheap to clone (Arc inside) and is typically built
//   once at startup and shared read-only for the rest of the session.
// ============================================================

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::ecs::component::Component;
use crate::ecs::entity::EntityId;
use crate::ecs::world::ECSWorld;
use crate::reflect::{Reflect, ReflectValue};

/// Function signature for component deserialization factories.
///
/// Receives the raw JSON `data` block, the world, and the target entity.
/// Should call `world.add_component(entity, ...)` before returning.
pub type ComponentFactory = Arc<
    dyn Fn(&Value, &mut ECSWorld, EntityId) -> Result<(), String> + Send + Sync,
>;

/// Returns a cloned snapshot of the component as `Box<dyn Reflect>`.
/// Used by the editor to read and display component fields.
pub type ReflectAccessor = Arc<
    dyn Fn(&ECSWorld, EntityId) -> Option<Box<dyn Reflect>> + Send + Sync,
>;

/// Mutates a single named field on the component in-place (via `get_component_mut`).
/// Used by the editor to apply property panel edits.
pub type ReflectMutator = Arc<
    dyn Fn(&ECSWorld, EntityId, &str, ReflectValue) -> Result<(), String> + Send + Sync,
>;

/// Removes a component from an entity by type-erased dispatch.
/// Registered alongside reflect in `register_reflect`.
pub type ComponentRemover = Arc<
    dyn Fn(&mut ECSWorld, EntityId) + Send + Sync,
>;

/// Registry that maps component type-name strings to factory functions.
///
/// Built once at startup; passed (by reference) to scene / prefab loaders.
#[derive(Clone, Default)]
pub struct ComponentRegistry {
    factories:         HashMap<String, ComponentFactory>,
    reflect_accessors: HashMap<String, ReflectAccessor>,
    reflect_mutators:  HashMap<String, ReflectMutator>,
    removers:          HashMap<String, ComponentRemover>,
    /// Static field descriptors cached at registration time for `.d.ts` generation.
    reflect_fields:    HashMap<String, Vec<crate::reflect::FieldDescriptor>>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a factory for a component type name.
    ///
    /// If a factory is already registered under `type_name`, it is replaced.
    pub fn register<F>(&mut self, type_name: &str, factory: F)
    where
        F: Fn(&Value, &mut ECSWorld, EntityId) -> Result<(), String> + Send + Sync + 'static,
    {
        self.factories.insert(type_name.to_string(), Arc::new(factory));
    }

    /// Returns `true` if a factory is registered for the given type name.
    pub fn has(&self, type_name: &str) -> bool {
        self.factories.contains_key(type_name)
    }

    /// Attempt to deserialize and attach a component.
    ///
    /// Returns `Ok(true)` if a factory was found and succeeded,
    /// `Ok(false)` if the type name is unknown (caller can warn),
    /// `Err(msg)` if the factory returned an error.
    pub fn attach(
        &self,
        type_name: &str,
        data: &Value,
        world: &mut ECSWorld,
        entity: EntityId,
    ) -> Result<bool, String> {
        match self.factories.get(type_name) {
            Some(factory) => {
                factory(data, world, entity)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    // ── Reflect API ───────────────────────────────────────────────────────────

    /// Register reflect accessor + mutator for a component type.
    ///
    /// `T` must implement `Component + Reflect + Clone`.
    /// Call this in addition to `register()` for any type you want the editor
    /// to be able to inspect and edit.
    ///
    /// Typically called via `register_builtins()` for built-in types, and
    /// manually for custom types.
    pub fn register_reflect<T>(&mut self, type_name: &str)
    where
        T: Component + Reflect + Clone + Default,
    {        
        // Cache static field descriptors for schema / .d.ts generation.
        self.reflect_fields.insert(
            type_name.to_string(),
            T::default().fields().to_vec(),
        );
        let name = type_name.to_string();

        // Accessor: clone the component out of hecs and box it as dyn Reflect.
        self.reflect_accessors.insert(
            name.clone(),
            Arc::new(move |world: &ECSWorld, entity: EntityId| {
                world.get_component::<T>(entity)
                    .map(|c| Box::new((*c).clone()) as Box<dyn Reflect>)
            }),
        );

        // Mutator: get a mutable reference (interior mutability via hecs) and call set_field.
        self.reflect_mutators.insert(
            name.clone(),
            Arc::new(move |world: &ECSWorld, entity: EntityId, field: &str, value: ReflectValue| {
                if let Some(mut c) = world.get_component_mut::<T>(entity) {
                    c.set_field(field, value)
                } else {
                    Err(format!("Entity {:?} does not have component '{}'", entity, name))
                }
            }),
        );

        // Remover: type-erased remove for editor add/remove component support.
        self.removers.insert(
            type_name.to_string(),
            Arc::new(move |world: &mut ECSWorld, entity: EntityId| {
                world.remove_component::<T>(entity);
            }),
        );
    }

    /// Read a component's fields as a cloned `Box<dyn Reflect>`.
    ///
    /// Returns `None` if the type is not reflect-registered or the entity
    /// does not have that component.
    pub fn get_reflect(
        &self,
        type_name: &str,
        world: &ECSWorld,
        entity: EntityId,
    ) -> Option<Box<dyn Reflect>> {
        let accessor = self.reflect_accessors.get(type_name)?;
        accessor(world, entity)
    }

    /// Mutate a single field on a component in-place.
    ///
    /// Returns `Err` if the type is not registered, the entity does not have
    /// the component, the field name is unknown, or the value type is wrong.
    pub fn set_reflect_field(
        &self,
        type_name: &str,
        world: &ECSWorld,
        entity: EntityId,
        field: &str,
        value: ReflectValue,
    ) -> Result<(), String> {
        let mutator = self.reflect_mutators.get(type_name)
            .ok_or_else(|| format!("No reflect mutator registered for '{}'", type_name))?;
        mutator(world, entity, field, value)
    }

    /// Returns `true` if reflect accessors are registered for `type_name`.
    pub fn has_reflect(&self, type_name: &str) -> bool {
        self.reflect_accessors.contains_key(type_name)
    }

    /// Returns all type names that have reflect accessors registered.
    /// Used by the inspector to enumerate which components an entity has.
    pub fn reflected_type_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.reflect_accessors.keys().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names
    }

    /// Remove a component by type name using the registered remover.
    /// Returns `false` if the type name has no remover registered.
    pub fn remove_component_by_name(
        &self,
        type_name: &str,
        world: &mut ECSWorld,
        entity: EntityId,
    ) -> bool {
        if let Some(remover) = self.removers.get(type_name) {
            remover(world, entity);
            true
        } else {
            false
        }
    }

    /// Returns the cached field descriptors for a component type (no world needed).
    pub fn component_fields(&self, type_name: &str) -> Option<&[crate::reflect::FieldDescriptor]> {
        self.reflect_fields.get(type_name).map(|v| v.as_slice())
    }

    /// Generate TypeScript interface declarations for all reflected component types.
    ///
    /// Returns a `.d.ts` string suitable for writing to `types/components.d.ts`.
    pub fn generate_component_dts(&self) -> String {
        use crate::reflect::ReflectFieldType as FT;

        let mut out = String::from(
            "// Auto-generated component type declarations — do not edit\n\
             // Generated by fluxion-core::ComponentRegistry\n\n",
        );

        let mut names: Vec<&str> = self.reflect_fields.keys().map(|s| s.as_str()).collect();
        names.sort_unstable();

        for name in &names {
            let fields = &self.reflect_fields[*name];
            out.push_str(&format!("interface {} {{\n", name));
            for f in fields {
                let ts_type = match f.field_type {
                    FT::F32 | FT::U32 | FT::U8 | FT::USize => "number",
                    FT::Bool                                => "boolean",
                    FT::Str                                 => "string",
                    FT::OptionStr                           => "string | undefined",
                    FT::Vec3                                => "[number, number, number]",
                    FT::Quat                                => "[number, number, number, number]",
                    FT::Color3                              => "[number, number, number]",
                    FT::Color4                              => "[number, number, number, number]",
                    FT::Enum                                => "string",
                    FT::Texture                             => "string | undefined",
                };
                let readonly = if f.read_only { "readonly " } else { "" };
                out.push_str(&format!("  /** {} */\n", f.display_name));
                out.push_str(&format!("  {}{}: {};\n", readonly, f.name, ts_type));
            }
            out.push_str("}\n\n");
        }

        // Also emit a union type for component names.
        out.push_str("type ComponentName = ");
        let name_strs: Vec<String> = names.iter().map(|n| format!("\"{}\"" , n)).collect();
        out.push_str(&name_strs.join(" | "));
        out.push_str(";\n");

        out
    }

    /// Register all built-in engine component types.
    ///
    /// Call this on every registry before use. Third-party crates call
    /// `register()` on top of this to add their own component types.
    pub fn register_builtins(&mut self) {
        use glam::{EulerRot, Quat, Vec3};

        use crate::components::camera::{Camera, ProjectionMode};
        use crate::components::light::{Light, LightType};
        use crate::components::mesh_renderer::{MeshRenderer, PrimitiveType};
        use crate::transform::Transform;

        // ── Transform ─────────────────────────────────────────────────────────
        self.register("Transform", |data, world, entity| {
            let mut t = Transform::new();
            if let Some(p) = data.get("position") {
                if let Some(v) = parse_vec3(p) { t.position = v; }
            }
            if let Some(r) = data.get("rotation") {
                if let Some(e) = parse_vec3(r) {
                    t.rotation = Quat::from_euler(EulerRot::XYZ, e.x, e.y, e.z);
                }
            }
            if let Some(s) = data.get("scale") {
                if let Some(v) = parse_vec3(s) { t.scale = v; }
            }
            t.dirty       = true;
            t.world_dirty = true;
            world.add_component(entity, t);
            Ok(())
        });

        // ── MeshRenderer ──────────────────────────────────────────────────────
        self.register("MeshRenderer", |data, world, entity| {
            let cast_shadow    = data.get("castShadow").and_then(|v| v.as_bool()).unwrap_or(true);
            let receive_shadow = data.get("receiveShadow").and_then(|v| v.as_bool()).unwrap_or(true);
            let layer          = data.get("layer").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
            let mesh_path      = data.get("modelPath").and_then(|v| v.as_str()).map(str::to_string);
            let material_path  = data.get("materialPath").and_then(|v| v.as_str()).map(str::to_string);
            let inline         = data.get("material").cloned();

            let primitive = if mesh_path.is_some() {
                None
            } else {
                let pt = data
                    .get("primitiveType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("cube")
                    .to_ascii_lowercase();
                Some(map_primitive(&pt))
            };

            world.add_component(entity, MeshRenderer {
                mesh_path,
                material_path,
                primitive,
                cast_shadow,
                receive_shadow,
                layer,
                mesh_handle:           None,
                material_handle:       None,
                scene_inline_material: inline,
            });
            Ok(())
        });

        // ── Camera ────────────────────────────────────────────────────────────
        self.register("Camera", |data, world, entity| {
            let mut c = Camera::new();
            c.fov        = data.get("fov").and_then(|v| v.as_f64()).unwrap_or(60.0) as f32;
            c.near       = data.get("near").and_then(|v| v.as_f64()).unwrap_or(0.1) as f32;
            c.far        = data.get("far").and_then(|v| v.as_f64()).unwrap_or(1000.0) as f32;
            c.ortho_size = data.get("orthoSize").and_then(|v| v.as_f64()).unwrap_or(10.0) as f32;
            c.is_active  = data.get("isMain").and_then(|v| v.as_bool()).unwrap_or(false);
            if data.get("isOrthographic").and_then(|v| v.as_bool()).unwrap_or(false) {
                c.projection_mode = ProjectionMode::Orthographic;
            }
            world.add_component(entity, c);
            Ok(())
        });

        // ── Light ─────────────────────────────────────────────────────────────
        self.register("Light", |data, world, entity| {
            // Skip ambient lights — handled by scene settings / renderer.
            if data.get("lightType").and_then(|v| v.as_str()) == Some("ambient") {
                return Ok(());
            }

            let light_type = match data.get("lightType").and_then(|v| v.as_str()).unwrap_or("point") {
                "directional" => LightType::Directional,
                "spot"        => LightType::Spot,
                _             => LightType::Point,
            };

            let color = data.get("color")
                .and_then(parse_color_rgb)
                .unwrap_or([1.0, 1.0, 1.0]);

            let mut l = Light {
                light_type,
                color,
                intensity:      data.get("intensity").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                range:          data.get("range").and_then(|v| v.as_f64()).unwrap_or(10.0) as f32,
                spot_angle:     data.get("spotAngle").and_then(|v| v.as_f64()).unwrap_or(45.0) as f32,
                spot_penumbra:  data.get("spotPenumbra").and_then(|v| v.as_f64()).unwrap_or(0.1) as f32,
                cast_shadow:    data.get("castShadow").and_then(|v| v.as_bool()).unwrap_or(true),
                shadow_map_size: data.get("shadowMapSize").and_then(|v| v.as_u64()).unwrap_or(2048) as u32,
                shadow_bias:    data.get("shadowBias").and_then(|v| v.as_f64()).unwrap_or(-0.0001) as f32,
            };

            if light_type == LightType::Directional {
                l.range = f32::MAX;
            }

            world.add_component(entity, l);
            Ok(())
        });

        // ── ParticleEmitter ───────────────────────────────────────────────────
        // Minimal: if data is present just attach a default emitter.
        // Full field parsing can be expanded later.
        self.register("ParticleEmitter", |_data, world, entity| {
            use crate::components::particle_emitter::ParticleEmitter;
            world.add_component(entity, ParticleEmitter::default());
            Ok(())
        });

        // ── RigidBody ─────────────────────────────────────────────────────────
        self.register("RigidBody", |data, world, entity| {
            use crate::components::rigid_body::{BodyType, PhysicsShape, RigidBody};
            let mut rb = RigidBody::default();
            if let Some(bt) = data.get("bodyType").and_then(|v| v.as_str()) {
                rb.body_type = match bt {
                    "Dynamic"   => BodyType::Dynamic,
                    "Kinematic" => BodyType::Kinematic,
                    "Static"    => BodyType::Static,
                    _           => BodyType::Dynamic,
                };
            }
            if let Some(shape_val) = data.get("shape") {
                if let Ok(s) = serde_json::from_value::<PhysicsShape>(shape_val.clone()) {
                    rb.shape = s;
                }
            }
            rb.mass            = data.get("mass").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            rb.linear_damping  = data.get("linearDamping").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            rb.angular_damping = data.get("angularDamping").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            rb.gravity_scale   = data.get("gravityScale").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            rb.can_sleep       = data.get("canSleep").and_then(|v| v.as_bool()).unwrap_or(true);
            rb.restitution     = data.get("restitution").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            rb.friction        = data.get("friction").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
            world.add_component(entity, rb);
            Ok(())
        });

        // ── Environment ───────────────────────────────────────────────────────
        self.register("Environment", |_data, world, entity| {
            use crate::components::environment::Environment;
            world.add_component(entity, Environment::default());
            Ok(())
        });

        // ── Reflect accessors for all built-in types ──────────────────────────
        // Allows the editor to read / write component fields at runtime.
        self.register_reflect::<Transform>("Transform");
        self.register_reflect::<MeshRenderer>("MeshRenderer");
        self.register_reflect::<Camera>("Camera");
        self.register_reflect::<Light>("Light");
        {
            use crate::components::particle_emitter::ParticleEmitter;
            self.register_reflect::<ParticleEmitter>("ParticleEmitter");
        }
        {
            use crate::components::rigid_body::RigidBody;
            self.register_reflect::<RigidBody>("RigidBody");
        }
        {
            use crate::components::camera_controller::CameraController;
            self.register_reflect::<CameraController>("CameraController");
        }
        {
            use crate::components::environment::Environment;
            self.register_reflect::<Environment>("Environment");
        }
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

fn parse_vec3(v: &Value) -> Option<glam::Vec3> {
    let a = v.as_array()?;
    if a.len() < 3 { return None; }
    Some(glam::Vec3::new(
        a[0].as_f64()? as f32,
        a[1].as_f64()? as f32,
        a[2].as_f64()? as f32,
    ))
}

fn parse_color_rgb(v: &Value) -> Option<[f32; 3]> {
    let a = v.as_array()?;
    if a.len() < 3 { return None; }
    Some([
        a[0].as_f64()? as f32,
        a[1].as_f64()? as f32,
        a[2].as_f64()? as f32,
    ])
}

fn map_primitive(pt: &str) -> crate::components::mesh_renderer::PrimitiveType {
    use crate::components::mesh_renderer::PrimitiveType;
    match pt {
        "cube" | "box" => PrimitiveType::Cube,
        "sphere"       => PrimitiveType::Sphere,
        "plane"        => PrimitiveType::Plane,
        "cylinder" | "cone" => PrimitiveType::Cylinder,
        "capsule"      => PrimitiveType::Capsule,
        _              => PrimitiveType::Cube,
    }
}
