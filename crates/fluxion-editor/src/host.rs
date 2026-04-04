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

use fluxion_core::{
    ComponentRegistry, ECSWorld, InputState, Time,
    transform::Transform,
    transform::system::TransformSystem,
    components::{Camera, Light, LightType},
};
use glam::{Quat, Vec3};
use fluxion_rune_scripting::RuneVm;

use crate::rune_bindings::{
    all_editor_modules,
    drain_pending_edits, get_selected_id,
    set_world_context, WorldContextGuard,
};
use crate::rune_bindings::world_module::push_log;

// ── EditorHost ────────────────────────────────────────────────────────────────

pub struct EditorHost {
    pub world:    ECSWorld,
    pub registry: ComponentRegistry,
    pub time:     Time,
    pub input:    InputState,
    pub vm:       RuneVm,
    pub physics:  Option<fluxion_physics::PhysicsEcsWorld>,

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

        Ok(Self {
            world,
            registry,
            time:        Time::new(),
            input:       InputState::new(),
            vm,
            physics,
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

        // Transform propagation
        TransformSystem::update(&mut self.world);

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
    pub fn push_world_context(&self) -> WorldContextGuard {
        set_world_context(&self.world, &self.registry)
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
                _ => {
                    if let Err(e) = self.registry.set_reflect_field(
                        &edit.component,
                        &self.world,
                        edit.entity,
                        &edit.field,
                        edit.value,
                    ) {
                        log::warn!("EditorHost::flush_pending_edits: {e}");
                    }
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
