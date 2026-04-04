// ============================================================
// fluxion-audio — cpal output device probe (MVP)
//
// WASM (`wasm32-unknown-unknown`): build without default backend;
// games can expose audio through JS / Web Audio.
// ============================================================

use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait};

/// Holds default output selection; streams can be built on top later.
pub struct AudioEngine {
    pub device_name: Option<String>,
}

impl AudioEngine {
    /// Opens the default host and default output device (no stream started).
    pub fn new() -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no default audio output device")?;
        let name = device.name().ok();
        log::info!("fluxion-audio: output device {:?}", name);
        Ok(Self { device_name: name })
    }

    /// Try init; logs and returns `None` if no device (non-fatal for sandbox).
    pub fn try_new() -> Option<Self> {
        match Self::new() {
            Ok(e) => Some(e),
            Err(e) => {
                log::warn!("fluxion-audio: disabled ({e})");
                None
            }
        }
    }
}
