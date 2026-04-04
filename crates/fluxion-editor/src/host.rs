// ============================================================
// host.rs — EditorHost
//
// Owns the pure engine state: ECS world, component registry,
// time, input, Rune VM, and physics.  The renderer and egui
// shell live separately in EditorInner (main.rs) so their
// borrows don't conflict with the world borrow inside
// render_with closures.
// ============================================================

use std::path::{Path, PathBuf};

use serde_json;

use fluxion_core::{
    ComponentRegistry, ECSWorld, InputState, Time,
    transform::Transform,
    transform::system::TransformSystem,
    components::{Camera, Light, LightType, CameraControllerSystem},
};
use glam::{Quat, Vec3};
use fluxion_rune_scripting::RuneVm;

use crate::rune_bindings::{
    all_editor_modules,
    drain_pending_edits, get_selected_id,
    set_world_context, WorldContextGuard,
    set_physics_context, clear_physics_context,
    set_audio_context, clear_audio_context,
    set_input_context, clear_input_context,
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

    /// Scripts directory — watched for hot reload.
    pub scripts_dir: PathBuf,
}

impl EditorHost {
    pub fn new(scripts_dir: PathBuf) -> anyhow::Result<Self> {
        let mut world    = ECSWorld::new();
        let mut registry = ComponentRegistry::new();
        registry.register_builtins();

        Self::spawn_default_scene(&mut world);

        // Rune VM: install editor-specific modules (fluxion::ui, fluxion::world)
        // on top of the default engine modules.
        let editor_modules = all_editor_modules()?;

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
        let mut vm = RuneVm::new_with_extra_modules(&path_refs, editor_modules)?;

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
            scripts_dir,
        })
    }

    // ── Default scene ─────────────────────────────────────────────────────────

    /// Public version of spawn_default_scene so main.rs can call it for New Scene.
    pub fn spawn_default_scene_pub(world: &mut ECSWorld) {
        Self::spawn_default_scene(world);
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
    }

    // ── Per-frame tick ────────────────────────────────────────────────────────

    pub fn tick(&mut self) {
        let (fixed_steps, _dt) = self.time.tick();

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

        // Transform propagation
        TransformSystem::update(&mut self.world);

        // Dispatch collision events to Rune scripts (Unity-style callbacks).
        self.dispatch_collision_events();

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
        set_world_context(&self.world, &self.registry)
    }

    /// Clear all Rune context pointers (called after every Rune panel tick).
    pub fn clear_rune_context(&mut self) {
        clear_physics_context();
        clear_audio_context();
        clear_input_context();
    }

    /// Apply queued field mutations produced by Rune inspector panels.
    fn flush_pending_edits(&mut self) {
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
                "__rename__" => {
                    if edit.entity.is_valid() {
                        self.world.set_name(edit.entity, &edit.field);
                    }
                }
                "__duplicate__" => {
                    if edit.entity.is_valid() {
                        let name = format!("{} (copy)", self.world.get_name(edit.entity));
                        let new_e = self.world.spawn(Some(&name));
                        let cloned_transform = self.world
                            .get_component::<fluxion_core::Transform>(edit.entity)
                            .map(|t| (*t).clone());
                        if let Some(t) = cloned_transform {
                            self.world.add_component(new_e, t);
                        }
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
                        if let Err(e) = self.registry.set_reflect_field(
                            &edit.component,
                            &self.world,
                            edit.entity,
                            &edit.field,
                            edit.value,
                        ) {
                            log::warn!("EditorHost::flush_pending_edits: {e}");
                        } else if let Some(inv) = inverse {
                            let label = format!("Edit {}.{}", edit.component, edit.field);
                            self.undo.push(label, vec![inv]);
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

    pub fn log(&mut self, line: impl Into<String>) {
        push_log(line.into());
    }

    /// Currently selected entity (forwarded from Rune world module state).
    pub fn selected_entity(&self) -> Option<fluxion_core::EntityId> {
        get_selected_id()
    }
}
