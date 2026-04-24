// =============================================================================
// engine.rs — Audio engine lifecycle
//
// WHAT THIS DOES:
// Owns the LoopbackCapture and WasapiRenderer and wires them together:
//   capture → EQ (FilterChain) → RenderSender → renderer → speakers
//
// The FilterChain is shared via Arc<Mutex<>> so the Tauri command layer can
// swap in a new profile at any time without restarting the audio pipeline.
//
// Held behind a Mutex in Tauri's AppState so commands can access it safely.
// =============================================================================

use std::sync::{Arc, Mutex};

use eq_audio::{AudioError, LoopbackCapture, StreamFormat, WasapiRenderer};
use eq_core::{FilterChain, Profile};

pub struct AudioEngine {
    capture: Option<LoopbackCapture>,
    renderer: Option<WasapiRenderer>,
    /// Shared with the capture thread. Replace its contents to change EQ live.
    pub filter_chain: Arc<Mutex<FilterChain>>,
    /// Sample rate of the open device; used to rebuild the FilterChain when
    /// the active profile changes.
    pub sample_rate: u32,
    /// Name of the profile currently loaded into the filter chain.
    /// Used by the background routing thread to avoid needless re-applies.
    pub active_profile_name: String,
}

impl AudioEngine {
    pub fn new() -> Self {
        Self {
            capture: None,
            renderer: None,
            filter_chain: Arc::new(Mutex::new(FilterChain::new(48_000.0))),
            sample_rate: 48_000,
            active_profile_name: String::new(),
        }
    }

    /// Starts the render and capture threads, wiring them together.
    ///
    /// `capture_device_id` — loopback source endpoint ID (the virtual cable device
    /// apps route audio to, e.g. CABLE Input). `None` captures from the system default.
    ///
    /// `render_device_id` — output endpoint ID (real speakers/headphones).
    /// `None` renders to the system-default output device.
    pub fn start(
        &mut self,
        capture_device_id: Option<String>,
        render_device_id: Option<String>,
    ) -> Result<StreamFormat, AudioError> {
        if self.is_running() {
            return Err(AudioError::AlreadyRunning);
        }

        // Start the renderer first so the sender is ready for the capture callback.
        let mut renderer = WasapiRenderer::new();
        let sender = renderer.sender();
        renderer.start(render_device_id)?;

        // Start capture. The callback pushes each processed buffer into the renderer's ring.
        let mut capture = LoopbackCapture::new();
        let fmt = capture.start(capture_device_id, Arc::clone(&self.filter_chain), move |samples| {
            sender.write(samples);
        })?;

        self.sample_rate = fmt.sample_rate;
        self.renderer = Some(renderer);
        self.capture = Some(capture);

        Ok(fmt)
    }

    /// Stops both threads cleanly. No-op if not running.
    pub fn stop(&mut self) {
        if let Some(mut c) = self.capture.take() {
            c.stop();
        }
        if let Some(mut r) = self.renderer.take() {
            r.stop();
        }
    }

    pub fn is_running(&self) -> bool {
        self.capture.is_some()
    }

    /// Replaces the active FilterChain with one built from `profile`.
    ///
    /// Takes effect on the next audio buffer — no dropout or glitch because
    /// the capture thread locks the Mutex per-packet and sees the new chain
    /// on its very next iteration.
    pub fn apply_profile(&mut self, profile: &Profile) {
        let chain = profile.to_filter_chain(self.sample_rate as f64);
        *self.filter_chain.lock().unwrap() = chain;
        self.active_profile_name = profile.name.clone();
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}
