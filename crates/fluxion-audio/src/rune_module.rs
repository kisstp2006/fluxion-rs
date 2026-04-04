// ============================================================
// fluxion-audio/rune_module.rs — fluxion::audio Rune module
//
// Unity-style API:
//   fluxion::audio::play(path)            -> i64  (handle)
//   fluxion::audio::play_looping(path)    -> i64
//   fluxion::audio::stop(handle)
//   fluxion::audio::set_volume(handle, v)
//
// Thread-local context pointer is set each frame by the engine host
// via set_audio_context() before Rune scripts run.
// ============================================================

use std::cell::Cell;
use std::ptr::NonNull;

use rune::Module;

use crate::AudioEngine;

// ── Thread-local context ──────────────────────────────────────────────────────

thread_local! {
    static AUDIO_PTR: Cell<Option<NonNull<AudioEngine>>> = Cell::new(None);
}

/// Set the audio engine pointer for the current frame.
/// # Safety: pointer must remain valid for the duration of the Rune call.
pub fn set_audio_context(engine: &mut AudioEngine) {
    AUDIO_PTR.with(|c| c.set(Some(NonNull::from(engine))));
}

/// Clear the audio engine pointer (call after Rune scripts finish).
pub fn clear_audio_context() {
    AUDIO_PTR.with(|c| c.set(None));
}

fn with_audio<R>(f: impl FnOnce(&mut AudioEngine) -> R) -> Option<R> {
    let mut ptr = AUDIO_PTR.with(|c| c.get())?;
    Some(unsafe { f(ptr.as_mut()) })
}

// ── Module builder ────────────────────────────────────────────────────────────

/// Build the `fluxion::audio` Rune module.
/// Register this with the Rune context in your engine host.
pub fn build_audio_rune_module() -> anyhow::Result<Module> {
    let mut m = Module::with_crate_item("fluxion", ["audio"])?;

    // AudioSource.Play(path) -> handle (i64)
    // One-shot playback. Returns 0 on failure.
    m.function("play", |path: String| -> i64 {
        with_audio(|eng| {
            eng.play(&path).map(|h| h.0 as i64).unwrap_or(0)
        }).unwrap_or(0)
    }).build()?;

    // AudioSource.PlayLooping(path) -> handle (i64)
    m.function("play_looping", |path: String| -> i64 {
        with_audio(|eng| {
            eng.play_looping(&path).map(|h| h.0 as i64).unwrap_or(0)
        }).unwrap_or(0)
    }).build()?;

    // AudioSource.PlayWithVolume(path, volume, looping) -> handle (i64)
    m.function("play_with_volume", |path: String, volume: f64, looping: bool| -> i64 {
        with_audio(|eng| {
            eng.play_with_volume(&path, volume as f32, looping)
                .map(|h| h.0 as i64)
                .unwrap_or(0)
        }).unwrap_or(0)
    }).build()?;

    // AudioSource.Stop(handle)
    m.function("stop", |handle: i64| {
        let _ = with_audio(|eng| eng.stop(crate::PlayHandle(handle as u64)));
    }).build()?;

    // AudioSource.SetVolume(handle, volume)
    m.function("set_volume", |handle: i64, volume: f64| {
        let _ = with_audio(|eng| eng.set_volume(crate::PlayHandle(handle as u64), volume as f32));
    }).build()?;

    Ok(m)
}
