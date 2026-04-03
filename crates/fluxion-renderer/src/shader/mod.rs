// ============================================================
// fluxion-renderer — ShaderCache
//
// Manages compiled wgpu ShaderModules. Caches by source string
// hash so the same WGSL source is only compiled once.
//
// Built-in shaders are embedded at compile time via include_str!().
// External shaders can be loaded at runtime for hot-reload support
// (dev builds) or loaded once for release builds.
// ============================================================

pub mod library;

use std::collections::HashMap;
use wgpu::Device;

/// A compiled shader module plus its source (for re-compilation on hot-reload).
pub struct CachedShader {
    pub module: wgpu::ShaderModule,
    pub source: String,
}

/// Shader module cache. Avoids recompiling the same WGSL source twice.
pub struct ShaderCache {
    modules: HashMap<String, CachedShader>,
}

impl ShaderCache {
    pub fn new() -> Self {
        Self { modules: HashMap::new() }
    }

    /// Compile WGSL source and cache under `name`. Returns a reference to the module.
    /// If already cached, returns the existing module.
    pub fn get_or_compile<'a>(
        &'a mut self,
        device: &Device,
        name:   &str,
        source: &str,
    ) -> &'a wgpu::ShaderModule {
        if !self.modules.contains_key(name) {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label:  Some(name),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            self.modules.insert(name.to_string(), CachedShader {
                module,
                source: source.to_string(),
            });
        }
        &self.modules[name].module
    }

    /// Force-recompile a shader (used for hot-reload in dev mode).
    pub fn recompile(&mut self, device: &Device, name: &str, new_source: &str) {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some(name),
            source: wgpu::ShaderSource::Wgsl(new_source.into()),
        });
        self.modules.insert(name.to_string(), CachedShader {
            module,
            source: new_source.to_string(),
        });
    }

    pub fn get(&self, name: &str) -> Option<&wgpu::ShaderModule> {
        self.modules.get(name).map(|c| &c.module)
    }
}

impl Default for ShaderCache {
    fn default() -> Self { Self::new() }
}
