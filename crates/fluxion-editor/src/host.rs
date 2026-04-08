// ============================================================
// host.rs — EditorHost
//
// Owns the pure engine state: ECS world, component registry,
// time, input, Rune VM, and physics.  The renderer and egui
// shell live separately in EditorInner (main.rs) so their
// borrows don't conflict with the world borrow inside
// render_with closures.
// ============================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json;

use fluxion_core::{
    AssetDatabase,
    ComponentRegistry, ECSWorld, InputState, Time,
    transform::Transform,
    transform::system::TransformSystem,
    components::{Camera, Light, LightType, CameraControllerSystem},
    AnimationSystem, LodSystem, CsgSystem, AudioSystem,
    PhysicsMaterial,
    components::rigid_body::RigidBody,
};
use glam::{Quat, Vec3};
use fluxion_rune_scripting::{RuneVm, RuneBehaviour, TIME_SNAPSHOT, input_snapshot};

use crate::rune_bindings::{
    all_editor_modules,
    drain_pending_edits, get_selected_id,
    set_world_context, WorldContextGuard,
    set_physics_context, clear_physics_context,
    set_audio_context, clear_audio_context,
    set_input_context, clear_input_context, set_action_map,
    set_camera_world, clear_camera_world, drain_camera_edits,
    set_environment_world, clear_environment_world, drain_environment_edits, EnvEditValue,
    set_asset_db_context, clear_asset_db_context,
    set_self_entity, clear_self_entity,
    set_self_script, clear_self_script,
    set_script_error, clear_script_error,
    drain_pending_destroys, drain_pending_spawns,
    build_gameplay_modules,
    set_compile_summary,
    set_script_fields, drain_script_fields,
};
use crate::rune_bindings::world_module::push_log;
use crate::undo::UndoStack;

// ── EditorHost ────────────────────────────────────────────────────────────────

pub struct EditorHost {
    pub world:    ECSWorld,
    pub registry: ComponentRegistry,
    pub time:     Time,
    pub input:    InputState,
    pub vm:       RuneVm,
    pub physics:  Option<fluxion_physics::PhysicsEcsWorld>,
    pub audio:    Option<fluxion_audio::AudioEngine>,
    pub undo:     UndoStack,
    /// Indexed asset catalogue — populated at project open, refreshed on rescan.
    pub asset_db:   AssetDatabase,

    /// Scripts directory — watched for hot reload.
    #[allow(dead_code)]
    pub scripts_dir: PathBuf,

    /// Per-entity gameplay Rune scripts, keyed by (entity, script_name).
    /// Rebuilt when play mode starts or scene is loaded.
    pub gameplay_scripts: HashMap<(fluxion_core::EntityId, String), RuneBehaviour>,

    /// Project root — needed to resolve gameplay script asset paths.
    pub project_root: PathBuf,

    /// Cache of loaded `.physmat` assets keyed by their asset path.
    pub physmat_cache: HashMap<String, PhysicsMaterial>,

    /// Paths of `.fluxmat` files that need GPU hot-reload this frame.
    /// Drained by main.rs after tick, which owns the renderer.
    pub pending_material_reloads: Vec<String>,

    /// Set to true when a new asset file was created/written this frame.
    /// main.rs drains this flag and calls asset_db.scan().
    pub needs_asset_rescan: bool,
}

impl EditorHost {
    pub fn new(scripts_dir: PathBuf) -> anyhow::Result<Self> {
        let mut world    = ECSWorld::new();
        let mut registry = ComponentRegistry::new();
        registry.register_builtins();

        Self::spawn_default_scene(&mut world);

        // Collect Rune panel scripts from scripts_dir.
        let script_paths: Vec<PathBuf> = if scripts_dir.is_dir() {
            let mut paths: Vec<PathBuf> = std::fs::read_dir(&scripts_dir)
                .unwrap_or_else(|_| {
                    log::warn!("Cannot read scripts dir: {:?}", scripts_dir);
                    // Return empty iterator equivalent via a workaround
                    std::fs::read_dir(".").unwrap()
                })
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rn"))
                .collect();
            paths.sort();
            paths
        } else {
            log::info!("Scripts dir not found: {:?} — starting without Rune scripts", scripts_dir);
            vec![]
        };

        let path_refs: Vec<&Path> = script_paths.iter().map(|p| p.as_path()).collect();
        // Pass a factory closure so new_with_extra_modules can call it twice
        // (once for the runtime context, once for the compile context) and
        // each call returns freshly constructed Module instances.
        // rune::Module does NOT implement Clone, so sharing one Vec between
        // two contexts would cause a duplicate-function-hash panic.
        let mut vm = RuneVm::new_with_extra_modules(&path_refs, all_editor_modules)?;

        // Enable hot reload on the scripts directory.
        if scripts_dir.is_dir() {
            if let Err(e) = vm.enable_hot_reload(&scripts_dir) {
                log::warn!("Hot reload watch failed: {e}");
            }
        }

        let physics = Some(fluxion_physics::PhysicsEcsWorld::new(
            glam::Vec3::new(0.0, -9.81, 0.0),
        ));

        let audio = fluxion_audio::AudioEngine::try_new();

        Ok(Self {
            world,
            registry,
            time:        Time::new(),
            input:       InputState::new(),
            vm,
            physics,
            audio,
            undo:        UndoStack::new(),
            asset_db:    AssetDatabase::new(),
            scripts_dir,
            gameplay_scripts:          HashMap::new(),
            project_root:              PathBuf::new(),
            physmat_cache:             HashMap::new(),
            pending_material_reloads:  Vec::new(),
            needs_asset_rescan:        false,
        })
    }

    // ── Default scene ─────────────────────────────────────────────────────────

    /// Public version of spawn_default_scene so main.rs can call it for New Scene.
    pub fn spawn_default_scene_pub(world: &mut ECSWorld) {
        Self::spawn_default_scene(world);
    }

    /// If the world has no entity with a Camera component, spawn a default editor camera.
    pub fn ensure_camera_exists(world: &mut ECSWorld) {
        let all: Vec<_> = world.all_entities().collect();
        let has_camera = all.iter().any(|&id| world.get_component::<Camera>(id).is_some());
        if !has_camera {
            let cam = world.spawn(Some("Main Camera"));
            let mut t = Transform::new();
            t.position = Vec3::new(0.0, 2.0, 5.0);
            t.rotation = Quat::from_rotation_x(-15_f32.to_radians());
            t.dirty = true;
            world.add_component(cam, t);
            world.add_component(cam, Camera::default());
            log::info!("No camera in loaded scene — spawned default Main Camera");
        }
    }

    fn spawn_default_scene(world: &mut ECSWorld) {
        // Camera
        let cam = world.spawn(Some("Main Camera"));
        let mut t = Transform::new();
        t.position = Vec3::new(0.0, 2.0, 5.0);
        t.rotation = Quat::from_rotation_x(-15_f32.to_radians());
        t.dirty = true;
        world.add_component(cam, t);
        world.add_component(cam, Camera::default());

        // Directional light (sun)
        let sun = world.spawn(Some("Directional Light"));
        let mut lt = Transform::new();
        lt.rotation = Quat::from_rotation_x(-45_f32.to_radians());
        lt.dirty = true;
        world.add_component(sun, lt);
        world.add_component(sun, Light {
            light_type: LightType::Directional,
            color: [1.0, 0.98, 0.9],
            intensity: 3.0,
            cast_shadow: true,
            ..Light::default()
        });

        // Test cube entity (no mesh yet — visible in hierarchy/inspector)
        let cube = world.spawn(Some("Cube"));
        let mut ct = Transform::new();
        ct.position = Vec3::ZERO;
        ct.dirty = true;
        world.add_component(cube, ct);

        // Environment — post-processing & ambient/fog settings for the scene
        let env_go = world.spawn(Some("Environment"));
        world.add_component(env_go, Transform::new());
        world.add_component(env_go, fluxion_core::components::environment::Environment::default());
    }

    // ── Gameplay script management ──────────────────────────────────────────

    /// Rebuild the per-entity RuneBehaviour map from all ScriptBundle components.
    /// Call when entering play mode or after a scene load.
    pub fn rebuild_gameplay_scripts(&mut self) {
        self.gameplay_scripts.clear();
        let mut entries: Vec<(fluxion_core::EntityId, String, String)> = Vec::new();
        self.world.query_active::<&fluxion_core::ScriptBundle, _>(|id, bundle| {
            for entry in &bundle.scripts {
                if entry.enabled && !entry.path.is_empty() {
                    entries.push((id, entry.name.clone(), entry.path.clone()));
                }
            }
        });
        let total = entries.len();
        for (entity_id, script_name, rel_path) in entries {
            let abs_path = self.project_root.join("assets").join(&rel_path);
            if !abs_path.is_file() {
                log::warn!("[ScriptBundle] File not found: {:?}", abs_path);
                continue;
            }
            match RuneBehaviour::from_file_with_extra_modules(&abs_path, build_gameplay_modules) {
                Ok(behaviour) => {
                    clear_script_error(entity_id.to_bits(), &script_name);
                    self.gameplay_scripts.insert((entity_id, script_name), behaviour);
                }
                Err(e) => {
                    let msg = format!("{:#}", e);
                    log::error!("[ScriptBundle] Compile error '{}' {:?}: {}", script_name, rel_path, msg);
                    set_script_error(entity_id.to_bits(), script_name, msg);
                }
            }
        }
        let error_count = total - self.gameplay_scripts.len();
        set_compile_summary(total, error_count);
        log::info!("[ScriptBundle] {}/{} gameplay scripts loaded", self.gameplay_scripts.len(), total);
    }

    /// Tick all gameplay scripts (play mode only).
    fn tick_gameplay_scripts(&mut self) {
        if self.gameplay_scripts.is_empty() { return; }

        let dt = self.time.dt;

        // Push frame timing so fluxion::time::delta_time() / elapsed() / frame() work.
        TIME_SNAPSHOT.update(self.time.dt, self.time.elapsed, self.time.frame_count);

        // Push held keys so fluxion::input::get_key() works in gameplay scripts.
        let held: Vec<String> = self.input.held_keys().map(str::to_string).collect();
        input_snapshot().update(held, vec![], vec![]);

        // Make world + registry available to gameplay Rune modules.
        let _guard = set_world_context(&self.world, &self.registry);

        // Make input available via fluxion::input::key_down() / axis_horizontal() etc.
        set_input_context(&mut self.input);

        let keys: Vec<(fluxion_core::EntityId, String)> = self.gameplay_scripts.keys().cloned().collect();
        for (entity_id, script_name) in keys {
            set_self_entity(entity_id.to_bits() as i64);
            set_self_script(&script_name);
            // Inject current field values from ScriptEntry into the thread-local.
            let current_fields: Vec<(String, serde_json::Value)> = self.world
                .get_component::<fluxion_core::ScriptBundle>(entity_id)
                .and_then(|b| b.scripts.iter().find(|e| e.name == script_name).map(|e| {
                    e.fields.iter().map(|f| (f.name.clone(), f.value.clone())).collect()
                }))
                .unwrap_or_default();
            set_script_fields(current_fields);
            if let Some(behaviour) = self.gameplay_scripts.get_mut(&(entity_id, script_name.clone())) {
                let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    behaviour.tick(dt);
                }));
                match tick_result {
                    Ok(()) => {
                        if let Some(err) = behaviour.error() {
                            set_script_error(entity_id.to_bits(), &script_name, err.to_string());
                        } else {
                            clear_script_error(entity_id.to_bits(), &script_name);
                        }
                    }
                    Err(panic_val) => {
                        let msg = panic_val.downcast_ref::<String>().map(|s| s.as_str())
                            .or_else(|| panic_val.downcast_ref::<&str>().copied())
                            .unwrap_or("unknown panic");
                        let err_msg = format!("PANIC: {}", msg);
                        log::error!("[ScriptBundle] {} / {}: {}", entity_id.to_bits(), script_name, err_msg);
                        set_script_error(entity_id.to_bits(), &script_name, err_msg);
                    }
                }
            }
            clear_self_script();
            // Drain updated fields back into ScriptEntry.
            let updated = drain_script_fields();
            if !updated.is_empty() {
                if let Some(mut bundle) = self.world.get_component_mut::<fluxion_core::ScriptBundle>(entity_id) {
                    if let Some(entry) = bundle.scripts.iter_mut().find(|e| e.name == script_name) {
                        for (fname, fval) in updated {
                            if let Some(f) = entry.fields.iter_mut().find(|f| f.name == fname) {
                                f.value = fval;
                            }
                        }
                    }
                }
            }
            clear_self_entity();
        }

        // Drop guard before any world mutations.
        drop(_guard);
        clear_input_context();

        // Apply deferred spawns/destroys from gameplay scripts.
        for bits in drain_pending_destroys() {
            let found: Option<fluxion_core::EntityId> =
                self.world.all_entities().find(|e| e.to_bits() == bits);
            if let Some(e) = found {
                self.gameplay_scripts.retain(|(eid, _), _| *eid != e);
                self.world.despawn(e);
            }
        }
        for name in drain_pending_spawns() {
            let e = self.world.spawn(Some(&name));
            self.world.add_component(e, fluxion_core::Transform::new());
        }
    }

    // ── Per-frame tick ────────────────────────────────────────────────────────

    pub fn tick(&mut self) {
        let (fixed_steps, _dt) = self.time.tick();

        // Apply .physmat overrides before physics sync (loads materials lazily).
        self.apply_physics_materials();

        // Physics: fixed-step
        if let Some(ref mut phys) = self.physics {
            for _ in 0..fixed_steps {
                phys.sync_from_ecs(&self.world);
                phys.step(self.time.fixed_dt);
                phys.sync_to_ecs(&self.world);
            }
        }

        // Camera controllers (play mode only)
        let dt = self.time.dt;
        CameraControllerSystem::update(&mut self.world, &self.input, dt);

        // Skeletal animation
        AnimationSystem::update(&self.world, dt);

        // LOD switching
        LodSystem::update(&self.world);

        // 3D spatial audio gains
        AudioSystem::update(&self.world);

        // CSG re-bake (dirty entities only)
        CsgSystem::update(&mut self.world);

        // Transform propagation
        TransformSystem::update(&mut self.world);

        // Dispatch collision events to Rune scripts (Unity-style callbacks).
        self.dispatch_collision_events();

        // Tick per-entity gameplay Rune scripts.
        self.tick_gameplay_scripts();

        // Hot reload poll
        self.vm.poll_hot_reload();

        // Flush pending world edits queued by Rune panels last frame.
        self.flush_pending_edits();

        self.input.begin_frame();
    }

    /// Editor-only tick: runs transforms and hot-reload but skips physics.
    /// Used while in Editing / Paused mode.
    pub fn tick_editor_only(&mut self) {
        self.time.tick();
        AnimationSystem::update(&self.world, self.time.dt);
        LodSystem::update(&self.world);
        AudioSystem::update(&self.world);
        CsgSystem::update(&mut self.world);
        TransformSystem::update(&mut self.world);
        self.vm.poll_hot_reload();
        self.flush_pending_edits();
        self.input.begin_frame();
    }

    // ── World context helpers ───────────────────────────────────────────────────

    /// Set thread-locals so the Rune world module can access ECS data this frame.
    /// Returns a `WorldContextGuard`; thread-locals are cleared when it drops.
    pub fn push_world_context(&mut self) -> WorldContextGuard {
        if let Some(ref mut phys) = self.physics {
            set_physics_context(phys);
        }
        if let Some(ref mut audio) = self.audio {
            set_audio_context(audio);
        }
        set_input_context(&mut self.input);
        // Push the current action map so action_pressed/action_value work this frame.
        crate::rune_bindings::settings_module::with_project_config(|cfg| {
            set_action_map(&cfg.settings.input.actions);
        });
        set_camera_world(&self.world);
        set_environment_world(&self.world);
        set_asset_db_context(&self.asset_db);
        set_world_context(&self.world, &self.registry)
    }

    /// Clear all Rune context pointers (called after every Rune panel tick).
    pub fn clear_rune_context(&mut self) {
        clear_physics_context();
        clear_audio_context();
        clear_input_context();
        clear_camera_world();
        clear_environment_world();
        clear_asset_db_context();
    }

    /// For every RigidBody with a non-empty `physics_material_path`, load the
    /// `.physmat` file (lazy-cached) and apply friction/restitution overrides
    /// to the ECS component so the physics engine picks them up on next sync.
    fn apply_physics_materials(&mut self) {
        // Step 1: collect (entity, path) pairs — immutable borrow of world.
        let mut pending: Vec<(fluxion_core::EntityId, String)> = Vec::new();
        self.world.query_all::<&RigidBody, _>(|eid, rb| {
            if !rb.physics_material_path.is_empty() {
                pending.push((eid, rb.physics_material_path.clone()));
            }
        });

        // Step 2: load materials (may mutate physmat_cache) and collect overrides.
        let mut overrides: Vec<(fluxion_core::EntityId, f32, f32)> = Vec::new();
        for (eid, path) in pending {
            if !self.physmat_cache.contains_key(&path) {
                let full = self.project_root.join(&path);
                if let Ok(text) = std::fs::read_to_string(&full) {
                    if let Ok(m) = PhysicsMaterial::from_json(&text) {
                        self.physmat_cache.insert(path.clone(), m);
                    }
                }
            }
            if let Some(m) = self.physmat_cache.get(&path) {
                overrides.push((eid, m.friction, m.restitution));
            }
        }

        // Step 3: apply overrides to ECS components.
        for (eid, friction, restitution) in overrides {
            if let Some(mut rb) = self.world.get_component_mut::<RigidBody>(eid) {
                rb.friction    = friction;
                rb.restitution = restitution;
            }
        }
    }

    /// Apply queued field mutations produced by Rune inspector panels.
    fn flush_pending_edits(&mut self) {
        // Drain environment edits — apply directly to first Environment component.
        let env_edits = drain_environment_edits();
        if !env_edits.is_empty() {
            use fluxion_core::reflect::ReflectValue;
            let mut env_entity = None;
            self.world.query_active::<&fluxion_core::Environment, _>(|id, _| {
                if env_entity.is_none() { env_entity = Some(id); }
            });
            if let Some(eid) = env_entity {
                if let Some(mut env) = self.world.get_component_mut::<fluxion_core::Environment>(eid) {
                    use fluxion_core::reflect::Reflect;
                    for edit in env_edits {
                        let rv = match (&edit.field[..], edit.value) {
                            ("sky_panorama_path", EnvEditValue::Str(s)) => ReflectValue::AssetPath(if s.is_empty() { None } else { Some(s) }),
                            (_, EnvEditValue::F32(f))   => ReflectValue::F32(f),
                            (_, EnvEditValue::Bool(b))  => ReflectValue::Bool(b),
                            (_, EnvEditValue::Str(s))   => ReflectValue::Enum(s),
                            (_, EnvEditValue::Color(c)) => ReflectValue::Color3(c),
                            (_, EnvEditValue::U32(u))   => ReflectValue::U32(u),
                        };
                        let _ = (*env).set_field(&edit.field, rv);
                    }
                }
            }
        }

        // Drain camera-specific edits (direct Camera component field mutations).
        for cam_edit in drain_camera_edits() {
            if let Some(mut cam) = self.world.get_component_mut::<fluxion_core::Camera>(cam_edit.entity) {
                use fluxion_core::reflect::Reflect;
                match cam_edit.field.as_str() {
                    "viewport_rect_x" => if let fluxion_core::reflect::ReflectValue::F32(v) = cam_edit.value { cam.viewport_rect[0] = v; }
                    "viewport_rect_y" => if let fluxion_core::reflect::ReflectValue::F32(v) = cam_edit.value { cam.viewport_rect[1] = v; }
                    "viewport_rect_w" => if let fluxion_core::reflect::ReflectValue::F32(v) = cam_edit.value { cam.viewport_rect[2] = v; }
                    "viewport_rect_h" => if let fluxion_core::reflect::ReflectValue::F32(v) = cam_edit.value { cam.viewport_rect[3] = v; }
                    "sensor_size_w"   => if let fluxion_core::reflect::ReflectValue::F32(v) = cam_edit.value { cam.sensor_size[0] = v; }
                    "sensor_size_h"   => if let fluxion_core::reflect::ReflectValue::F32(v) = cam_edit.value { cam.sensor_size[1] = v; }
                    "lens_shift_x"    => if let fluxion_core::reflect::ReflectValue::F32(v) = cam_edit.value { cam.lens_shift[0] = v; }
                    "lens_shift_y"    => if let fluxion_core::reflect::ReflectValue::F32(v) = cam_edit.value { cam.lens_shift[1] = v; }
                    field => { let _ = cam.set_field(field, cam_edit.value); }
                }
            }
        }

        for edit in drain_pending_edits() {
            match edit.component.as_str() {
                "__spawn__" => {
                    let name = if edit.field.is_empty() { "Entity" } else { &edit.field };
                    let e = self.world.spawn(Some(name));
                    self.world.add_component(e, fluxion_core::Transform::new());
                }
                "__despawn__" => {
                    if edit.entity.is_valid() {
                        self.world.despawn(edit.entity);
                    }
                }
                "__duplicate__" => {
                    if edit.entity.is_valid() {
                        self.duplicate_entity(edit.entity);
                    }
                }
                "__add_script__" => {
                    if edit.entity.is_valid() {
                        use fluxion_core::{ScriptBundle, scan_struct_fields, derive_script_name};
                        let abs_path = self.project_root.join("assets").join(&edit.field);
                        let source = std::fs::read_to_string(&abs_path).unwrap_or_default();
                        let script_name = derive_script_name(&edit.field);
                        let fields = scan_struct_fields(&source, &script_name);
                        let has_bundle = self.world.has_component::<ScriptBundle>(edit.entity);
                        if has_bundle {
                            if let Some(mut bundle) = self.world.get_component_mut::<ScriptBundle>(edit.entity) {
                                bundle.scripts.push(fluxion_core::ScriptEntry {
                                    name: script_name, path: edit.field.clone(),
                                    enabled: true, fields,
                                });
                            }
                        } else {
                            let mut bundle = ScriptBundle::default();
                            bundle.scripts.push(fluxion_core::ScriptEntry {
                                name: script_name, path: edit.field.clone(),
                                enabled: true, fields,
                            });
                            self.world.add_component(edit.entity, bundle);
                        }
                    }
                }
                "__remove_script__" => {
                    if edit.entity.is_valid() {
                        if let Some(mut bundle) = self.world.get_component_mut::<fluxion_core::ScriptBundle>(edit.entity) {
                            bundle.remove_by_name(&edit.field);
                        }
                        self.gameplay_scripts.retain(|(eid, name), _| !(*eid == edit.entity && name == &edit.field));
                    }
                }
                "__add_comp__" => {
                    if edit.entity.is_valid() {
                        if let Err(e) = self.registry.attach(
                            &edit.field,
                            &serde_json::Value::Object(Default::default()),
                            &mut self.world,
                            edit.entity,
                        ) {
                            log::warn!("add_component '{}': {:?}", edit.field, e);
                        }
                    }
                }
                "__remove_comp__" => {
                    if edit.entity.is_valid() {
                        if !self.registry.remove_component_by_name(
                            &edit.field,
                            &mut self.world,
                            edit.entity,
                        ) {
                            log::warn!("remove_component: no remover for '{}'", edit.field);
                        }
                    }
                }
                "__create_prefab__" => {
                    if edit.entity.is_valid() {
                        let rel_path = &edit.field;
                        let abs_path = if std::path::Path::new(rel_path).is_absolute() {
                            std::path::PathBuf::from(rel_path)
                        } else {
                            self.project_root.join(rel_path)
                        };
                        if let Some(parent) = abs_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        // Serialize the entity using reflection into a single-entity SceneFileData.
                        use fluxion_core::scene::{SceneFileData, SceneSettings, SerializedEntity, SerializedComponent};
                        let eid = edit.entity;
                        let entity_name = self.world.get_name(eid).to_string();
                        let tags: Vec<String> = self.world.tags_of(eid).map(str::to_string).collect();
                        let mut components: Vec<SerializedComponent> = Vec::new();
                        for &comp_type in self.world.component_names(eid) {
                            if let Some(reflected) = self.registry.get_reflect(comp_type, &self.world, eid) {
                                components.push(SerializedComponent {
                                    component_type: comp_type.to_string(),
                                    data: reflected.to_serialized_data(),
                                });
                            }
                        }
                        let prefab_name = abs_path.file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "prefab".to_string());
                        let data = SceneFileData {
                            name: prefab_name,
                            version: 2,
                            settings: SceneSettings::default(),
                            editor_camera: None,
                            entities: vec![SerializedEntity {
                                id: 1, name: entity_name, parent: None, tags, components,
                            }],
                        };
                        match fluxion_core::scene::save_scene_file(abs_path.to_str().unwrap_or(""), &data) {
                            Ok(()) => {
                                log::info!("Prefab saved: {:?}", abs_path);
                                self.asset_db.scan(&self.project_root);
                            }
                            Err(e) => log::warn!("Prefab save failed: {e}"),
                        }
                    }
                }
                "__instantiate_prefab__" => {
                    let path = &edit.field;
                    match fluxion_core::scene::load_scene_file(path) {
                        Ok(data) => {
                            match fluxion_core::scene::instantiate_entities(
                                &mut self.world,
                                &data.entities,
                                &self.registry,
                            ) {
                                Ok(_) => log::info!("Prefab instantiated: {path}"),
                                Err(e) => log::warn!("Prefab instantiate error: {e}"),
                            }
                        }
                        Err(e) => log::warn!("Prefab load failed '{path}': {e}"),
                    }
                }
                "__set_script_field__" => {
                    if edit.entity.is_valid() {
                        // Packed as "script_name\x00field_name\x00value_str"
                        let parts: Vec<&str> = edit.field.splitn(3, '\x00').collect();
                        if parts.len() == 3 {
                            let (sname, fname, vstr) = (parts[0], parts[1], parts[2]);
                            if let Some(mut bundle) = self.world.get_component_mut::<fluxion_core::ScriptBundle>(edit.entity) {
                                if let Some(entry) = bundle.scripts.iter_mut().find(|e| e.name == sname) {
                                    if let Some(field) = entry.fields.iter_mut().find(|f| f.name == fname) {
                                        // Parse the value string heuristically.
                                        field.value = if vstr == "true" {
                                            serde_json::Value::Bool(true)
                                        } else if vstr == "false" {
                                            serde_json::Value::Bool(false)
                                        } else if let Ok(n) = vstr.parse::<i64>() {
                                            serde_json::json!(n)
                                        } else if let Ok(f) = vstr.parse::<f64>() {
                                            serde_json::json!(f)
                                        } else {
                                            serde_json::Value::String(vstr.to_string())
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
                "__set_parent__" => {
                    if edit.entity.is_valid() {
                        let parent_id: i64 = edit.field.parse().unwrap_or(-1);
                        if parent_id < 0 {
                            self.world.set_parent(edit.entity, None, false);
                        } else {
                            let parent_bits = parent_id as u64;
                            let parent_entity = self.world.all_entities()
                                .find(|e| e.to_bits() == parent_bits);
                            if let Some(parent) = parent_entity {
                                self.world.set_parent(edit.entity, Some(parent), false);
                            }
                        }
                    }
                }
                "__set_collision_layer__" => {
                    if edit.entity.is_valid() {
                        if let Ok(layer) = edit.field.parse::<u32>() {
                            if let Some(mut rb) = self.world.get_component_mut::<RigidBody>(edit.entity) {
                                rb.collision_layer = layer;
                            }
                        }
                    }
                }
                "__set_collision_mask__" => {
                    if edit.entity.is_valid() {
                        if let Ok(mask) = edit.field.parse::<i64>() {
                            if let Some(mut rb) = self.world.get_component_mut::<RigidBody>(edit.entity) {
                                rb.collision_mask = mask as u32;
                            }
                        }
                    }
                }
                "__set_physics_material__" => {
                    if edit.entity.is_valid() {
                        if let Some(mut rb) = self.world.get_component_mut::<RigidBody>(edit.entity) {
                            rb.physics_material_path = edit.field.clone();
                        }
                    }
                }
                "__write_material__" => {
                    let path = edit.field.clone();
                    let json = if let fluxion_core::reflect::ReflectValue::Str(s) = edit.value.clone() { s } else { String::new() };
                    let full_path = self.project_root.join("assets").join(&path);
                    if let Err(e) = std::fs::write(&full_path, json.as_bytes()) {
                        log::warn!("write_material: failed to write {path}: {e}");
                    } else {
                        self.pending_material_reloads.push(path);
                        self.needs_asset_rescan = true;
                    }
                }
                "__create_material__" => {
                    let path = edit.field.clone();
                    let full_path = self.project_root.join("assets").join(&path);
                    if !full_path.exists() {
                        if let Some(parent) = full_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let default_asset = fluxion_renderer::material::MaterialAsset::default();
                        match serde_json::to_vec_pretty(&default_asset) {
                            Ok(bytes) => {
                                if let Err(e) = std::fs::write(&full_path, &bytes) {
                                    log::warn!("create_material: failed to write {path}: {e}");
                                } else {
                                    self.needs_asset_rescan = true;
                                }
                            }
                            Err(e) => log::warn!("create_material: serialize failed: {e}"),
                        }
                    }
                }
                "__set_material_slot__" => {
                    if edit.entity.is_valid() {
                        if let Ok(idx) = edit.field.parse::<usize>() {
                            let new_path = if let fluxion_core::reflect::ReflectValue::Str(s) = edit.value.clone() {
                                if s.is_empty() { None } else { Some(s) }
                            } else { None };
                            if let Some(mut mr) = self.world.get_component_mut::<fluxion_core::MeshRenderer>(edit.entity) {
                                if let Some(slot) = mr.material_slots.get_mut(idx) {
                                    slot.material_path = new_path.clone();
                                    slot.material_handle = None;
                                }
                            }
                            // Signal for re-hydration by main.rs on the next frame.
                            if let Some(new_path_str) = new_path {
                                self.pending_material_reloads.push(new_path_str);
                            }
                        }
                    }
                }
                "__set_material_path__" => {
                    if edit.entity.is_valid() {
                        let path = edit.field.clone();
                        let new_path = if path.is_empty() { None } else { Some(path.clone()) };
                        if let Some(mut mr) = self.world.get_component_mut::<fluxion_core::MeshRenderer>(edit.entity) {
                            mr.material_path = new_path.clone();
                            mr.material_handle = None;
                        }
                        if let Some(new_path_str) = new_path {
                            self.pending_material_reloads.push(new_path_str);
                        }
                    }
                }
                _ => {
                    // Capture current value as undo inverse before applying.
                    if edit.entity.is_valid() {
                        let inverse = if let Some(reflect) = self.registry.get_reflect(&edit.component, &self.world, edit.entity) {
                            reflect.get_field(&edit.field).map(|old_val| {
                                crate::rune_bindings::PendingEdit {
                                    entity:    edit.entity,
                                    component: edit.component.clone(),
                                    field:     edit.field.clone(),
                                    value:     old_val,
                                }
                            })
                        } else {
                            None
                        };
                        let is_main_set = edit.component == "Camera"
                            && edit.field == "is_main"
                            && edit.value == fluxion_core::reflect::ReflectValue::Bool(true);
                        if let Err(e) = self.registry.set_reflect_field(
                            &edit.component,
                            &self.world,
                            edit.entity,
                            &edit.field,
                            edit.value,
                        ) {
                            log::warn!("EditorHost::flush_pending_edits: {e}");
                        } else {
                            if let Some(inv) = inverse {
                                let label = format!("Edit {}.{}", edit.component, edit.field);
                                self.undo.push(label, vec![inv]);
                            }
                            // Enforce single-main-camera: clear is_main on every other Camera.
                            if is_main_set {
                                let owner = edit.entity;
                                let others: Vec<fluxion_core::EntityId> = self.world
                                    .all_entities()
                                    .filter(|&e| e != owner
                                        && self.world.get_component::<Camera>(e).is_some())
                                    .collect();
                                for other in others {
                                    if let Some(mut cam) = self.world.get_component_mut::<Camera>(other) {
                                        cam.is_main = false;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Drain collision events from physics and call Rune on_collision_enter / on_collision_exit.
    fn dispatch_collision_events(&mut self) {
        let events = match self.physics {
            Some(ref mut phys) => phys.drain_collision_events(),
            None => return,
        };
        if events.is_empty() { return; }

        for evt in events {
            let id_a = evt.entity_a.to_bits() as i64;
            let id_b = evt.entity_b.to_bits() as i64;
            if evt.started {
                if let Err(e) = self.vm.on_collision_enter(id_a, id_b) {
                    log::warn!("[collision] on_collision_enter error: {e}");
                }
            } else {
                if let Err(e) = self.vm.on_collision_exit(id_a, id_b) {
                    log::warn!("[collision] on_collision_exit error: {e}");
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn log(&mut self, line: impl Into<String>) {
        push_log(line.into());
    }

    /// Currently selected entity (forwarded from Rune world module state).
    pub fn selected_entity(&self) -> Option<fluxion_core::EntityId> {
        get_selected_id()
    }

    /// Deep-clone an entity: copies all reflected components + name onto a new entity.
    pub fn duplicate_entity(&mut self, src: fluxion_core::EntityId) -> fluxion_core::EntityId {
        let name = format!("{} (copy)", self.world.get_name(src));
        let new_e = self.world.spawn(Some(&name));

        // Collect component type names first to avoid borrow conflicts.
        let comp_names: Vec<String> = self.world
            .component_names(src)
            .iter()
            .map(|s| s.to_string())
            .collect();

        for comp_type in &comp_names {
            // Serialize via reflection, then re-attach via registry.
            let json_opt = self.registry
                .get_reflect(comp_type, &self.world, src)
                .map(|r| r.to_serialized_data());
            if let Some(json_data) = json_opt {
                if let Err(e) = self.registry.attach(
                    comp_type, &json_data, &mut self.world, new_e,
                ) {
                    log::warn!("duplicate_entity: attach '{}' failed: {e}", comp_type);
                }
            }
        }
        new_e
    }

    /// Duplicate the currently selected entity (Ctrl+D shortcut handler).
    /// Returns true if anything was duplicated (so the caller can mark scene dirty).
    pub fn duplicate_selected(&mut self) -> bool {
        let Some(src) = self.selected_entity() else { return false; };
        let new_e = self.duplicate_entity(src);
        // Also duplicate multi-selected entities.
        let others: Vec<_> = crate::rune_bindings::get_multi_selected()
            .into_iter()
            .filter(|e| *e != src)
            .collect();
        for other in others {
            self.duplicate_entity(other);
        }
        crate::rune_bindings::set_selected_id(Some(new_e));
        true
    }
}
