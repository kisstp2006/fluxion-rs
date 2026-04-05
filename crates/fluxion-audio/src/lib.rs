// ============================================================
// fluxion-audio — runtime audio playback engine
//
// Architecture:
//   AudioEngine  — owns the cpal output stream and a mixer.
//                  Lives on the main thread; the stream callback
//                  runs on a dedicated OS audio thread.
//   AudioClip    — decoded PCM f32 samples (stereo, 44100 Hz
//                  or whatever the device requests).
//   PlayHandle   — opaque id returned by `play()`.
//
// Usage:
//   let engine = AudioEngine::try_new().unwrap();
//   let id     = engine.play("assets/sounds/jump.ogg");
//   engine.stop(id);
// ============================================================

use std::{
    collections::HashMap,
    io::Cursor,
    path::Path,
    sync::{Arc, Mutex},
};

#[cfg(feature = "rune-scripting")]
pub mod rune_module;
#[cfg(feature = "rune-scripting")]
pub use rune_module::{build_audio_rune_module, set_audio_context, clear_audio_context};

use anyhow::Context;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, SupportedStreamConfig,
};

// ── PlayHandle ────────────────────────────────────────────────────────────────

/// Opaque handle identifying a playing sound instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlayHandle(pub u64);

// ── AudioClip ─────────────────────────────────────────────────────────────────

/// Decoded audio data: interleaved f32 PCM, in the device's channel layout.
struct AudioClip {
    /// Interleaved f32 samples (left, right, left, right, …).
    samples:     Vec<f32>,
    /// Number of audio channels (usually 2).
    #[allow(dead_code)]
    channels:    usize,
    /// Sample rate (Hz).
    #[allow(dead_code)]
    sample_rate: u32,
}

/// Decode a file (WAV/OGG/MP3) into an `AudioClip`.
fn decode_file(path: &Path) -> anyhow::Result<AudioClip> {
    use symphonia::core::audio::{AudioBufferRef, Signal};
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let data = std::fs::read(path)
        .with_context(|| format!("Cannot read audio file {:?}", path))?;

    let cursor = Cursor::new(data);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .context("Symphonia: unsupported format")?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .context("no audio track found")?;

    let track_id  = track.id;
    let channels  = track.codec_params.channels
        .map(|c| c.count())
        .unwrap_or(2);
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("cannot create decoder")?;

    let mut samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p)  => p,
            Err(_) => break,
        };
        if packet.track_id() != track_id { continue; }

        let decoded = match decoder.decode(&packet) {
            Ok(d)  => d,
            Err(_) => continue,
        };

        // Convert any sample format → f32.
        match decoded {
            AudioBufferRef::F32(buf) => {
                for frame in 0..buf.frames() {
                    for ch in 0..buf.spec().channels.count() {
                        samples.push(buf.chan(ch)[frame]);
                    }
                }
            }
            AudioBufferRef::S16(buf) => {
                for frame in 0..buf.frames() {
                    for ch in 0..buf.spec().channels.count() {
                        samples.push(buf.chan(ch)[frame] as f32 / i16::MAX as f32);
                    }
                }
            }
            AudioBufferRef::S32(buf) => {
                for frame in 0..buf.frames() {
                    for ch in 0..buf.spec().channels.count() {
                        samples.push(buf.chan(ch)[frame] as f32 / i32::MAX as f32);
                    }
                }
            }
            other => {
                // Convert via a temporary f32 buffer.
                let mut tmp = symphonia::core::audio::AudioBuffer::<f32>::new(
                    other.frames() as u64,
                    *other.spec(),
                );
                other.convert(&mut tmp);
                for frame in 0..tmp.frames() {
                    for ch in 0..tmp.spec().channels.count() {
                        samples.push(tmp.chan(ch)[frame]);
                    }
                }
            }
        }
    }

    Ok(AudioClip { samples, channels, sample_rate })
}

// ── Mixer ─────────────────────────────────────────────────────────────────────

/// A single playing voice inside the mixer.
struct Voice {
    samples:  Arc<Vec<f32>>,
    cursor:   usize,
    volume:   f32,
    looping:  bool,
    finished: bool,
}

/// Shared mixer state accessed by both the main thread and the audio callback.
struct MixerState {
    voices:    HashMap<u64, Voice>,
    next_id:   u64,
    /// Device channel count (1 or 2).
    #[allow(dead_code)]
    channels:  usize,
}

impl MixerState {
    fn new(channels: usize) -> Self {
        Self { voices: HashMap::new(), next_id: 1, channels }
    }

    fn play(&mut self, clip: Arc<Vec<f32>>, volume: f32, looping: bool) -> PlayHandle {
        let id = self.next_id;
        self.next_id += 1;
        self.voices.insert(id, Voice { samples: clip, cursor: 0, volume, looping, finished: false });
        PlayHandle(id)
    }

    fn stop(&mut self, handle: PlayHandle) {
        if let Some(v) = self.voices.get_mut(&handle.0) {
            v.finished = true;
        }
    }

    fn set_volume(&mut self, handle: PlayHandle, volume: f32) {
        if let Some(v) = self.voices.get_mut(&handle.0) {
            v.volume = volume;
        }
    }

    /// Fill `out` with mixed f32 samples; remove finished voices.
    fn mix(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() { *s = 0.0; }

        for voice in self.voices.values_mut() {
            if voice.finished { continue; }
            let src = &voice.samples;
            for s in out.iter_mut() {
                if voice.cursor >= src.len() {
                    if voice.looping {
                        voice.cursor = 0;
                    } else {
                        voice.finished = true;
                        break;
                    }
                }
                *s += src[voice.cursor] * voice.volume;
                voice.cursor += 1;
            }
        }
        self.voices.retain(|_, v| !v.finished);

        // Soft clip to [-1, 1].
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

// ── AudioEngine ───────────────────────────────────────────────────────────────

/// Runtime audio engine. Create once at startup; keep alive for the session.
pub struct AudioEngine {
    pub device_name: Option<String>,
    mixer:  Arc<Mutex<MixerState>>,
    _stream: cpal::Stream,
    /// Loaded clip cache (path → decoded PCM).
    clip_cache: Mutex<HashMap<String, Arc<Vec<f32>>>>,
}

impl AudioEngine {
    /// Open the default output device and start the audio stream.
    pub fn new() -> anyhow::Result<Self> {
        let host   = cpal::default_host();
        let device = host.default_output_device()
            .context("no default audio output device")?;
        let name   = device.name().ok();
        log::info!("fluxion-audio: device {:?}", name);

        let config: SupportedStreamConfig = device
            .default_output_config()
            .context("no supported output config")?;

        let channels = config.channels() as usize;
        let mixer     = Arc::new(Mutex::new(MixerState::new(channels)));
        let mixer_cb  = Arc::clone(&mixer);

        let stream = match config.sample_format() {
            SampleFormat::F32 => Self::build_stream_f32(&device, &config, mixer_cb)?,
            SampleFormat::I16 => Self::build_stream_i16(&device, &config, mixer_cb)?,
            SampleFormat::U16 => Self::build_stream_u16(&device, &config, mixer_cb)?,
            other => anyhow::bail!("unsupported sample format: {:?}", other),
        };
        stream.play().context("failed to start audio stream")?;

        Ok(Self {
            device_name: name,
            mixer,
            _stream: stream,
            clip_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Non-fatal init: logs and returns `None` if audio is unavailable.
    pub fn try_new() -> Option<Self> {
        match Self::new() {
            Ok(e)  => Some(e),
            Err(e) => { log::warn!("fluxion-audio: disabled ({e})"); None }
        }
    }

    // ── Playback API ───────────────────────────────────────────────────────────

    /// Load (or reuse cached) clip and start playing it. Returns a handle.
    pub fn play(&self, path: &str) -> Option<PlayHandle> {
        let clip = self.load_clip(path)?;
        let handle = self.mixer.lock().unwrap().play(clip, 1.0, false);
        Some(handle)
    }

    /// Like `play` but loops until `stop()` is called.
    pub fn play_looping(&self, path: &str) -> Option<PlayHandle> {
        let clip = self.load_clip(path)?;
        let handle = self.mixer.lock().unwrap().play(clip, 1.0, true);
        Some(handle)
    }

    /// Play with explicit volume (0.0–1.0).
    pub fn play_with_volume(&self, path: &str, volume: f32, looping: bool) -> Option<PlayHandle> {
        let clip = self.load_clip(path)?;
        let handle = self.mixer.lock().unwrap().play(clip, volume, looping);
        Some(handle)
    }

    /// Stop a playing sound.
    pub fn stop(&self, handle: PlayHandle) {
        self.mixer.lock().unwrap().stop(handle);
    }

    /// Set the volume of a playing sound (0.0–1.0).
    pub fn set_volume(&self, handle: PlayHandle, volume: f32) {
        self.mixer.lock().unwrap().set_volume(handle, volume);
    }

    // ── Internal ───────────────────────────────────────────────────────────────

    fn load_clip(&self, path: &str) -> Option<Arc<Vec<f32>>> {
        {
            let cache = self.clip_cache.lock().unwrap();
            if let Some(c) = cache.get(path) {
                return Some(Arc::clone(c));
            }
        }
        match decode_file(Path::new(path)) {
            Ok(clip) => {
                let arc = Arc::new(clip.samples);
                self.clip_cache.lock().unwrap().insert(path.to_string(), Arc::clone(&arc));
                Some(arc)
            }
            Err(e) => {
                log::error!("fluxion-audio: failed to load {:?}: {e}", path);
                None
            }
        }
    }

    fn build_stream_f32(
        device:   &cpal::Device,
        config:   &SupportedStreamConfig,
        mixer:    Arc<Mutex<MixerState>>,
    ) -> anyhow::Result<cpal::Stream> {
        let stream_config = config.config();
        Ok(device.build_output_stream(
            &stream_config,
            move |data: &mut [f32], _| {
                mixer.lock().unwrap().mix(data);
            },
            |e| log::error!("fluxion-audio stream error: {e}"),
            None,
        )?)
    }

    fn build_stream_i16(
        device:   &cpal::Device,
        config:   &SupportedStreamConfig,
        mixer:    Arc<Mutex<MixerState>>,
    ) -> anyhow::Result<cpal::Stream> {
        let stream_config = config.config();
        Ok(device.build_output_stream(
            &stream_config,
            move |data: &mut [i16], _| {
                let mut buf = vec![0.0f32; data.len()];
                mixer.lock().unwrap().mix(&mut buf);
                for (out, s) in data.iter_mut().zip(buf.iter()) {
                    *out = (s * i16::MAX as f32) as i16;
                }
            },
            |e| log::error!("fluxion-audio stream error: {e}"),
            None,
        )?)
    }

    fn build_stream_u16(
        device:   &cpal::Device,
        config:   &SupportedStreamConfig,
        mixer:    Arc<Mutex<MixerState>>,
    ) -> anyhow::Result<cpal::Stream> {
        let stream_config = config.config();
        Ok(device.build_output_stream(
            &stream_config,
            move |data: &mut [u16], _| {
                let mut buf = vec![0.0f32; data.len()];
                mixer.lock().unwrap().mix(&mut buf);
                for (out, s) in data.iter_mut().zip(buf.iter()) {
                    *out = ((s + 1.0) * 0.5 * u16::MAX as f32) as u16;
                }
            },
            |e| log::error!("fluxion-audio stream error: {e}"),
            None,
        )?)
    }
}
