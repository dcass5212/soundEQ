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

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use eq_audio::{AudioError, LoopbackCapture, SpectrumAnalyzer, StreamFormat, VolumeMonitor, WasapiRenderer, SPECTRUM_BANDS};
use eq_core::{CrossfeedConfig, CrossfeedProcessor, FilterChain, Profile};

pub struct AudioEngine {
    capture: Option<LoopbackCapture>,
    renderer: Option<WasapiRenderer>,
    /// Polls the VB-Cable endpoint's IAudioEndpointVolume every 50 ms and
    /// writes the scalar into the renderer's volume atomic, so the Windows
    /// volume slider works even though WASAPI loopback captures pre-volume.
    vol_monitor: VolumeMonitor,
    /// Shared with the capture thread. Replace its contents to change EQ live.
    pub filter_chain: Arc<Mutex<FilterChain>>,
    /// Shared with the capture thread. Updated via apply_profile when the
    /// active profile changes. Processes each buffer after the EQ chain.
    pub crossfeed: Arc<Mutex<CrossfeedProcessor>>,
    /// Sample rate of the open device; used to rebuild the FilterChain when
    /// the active profile changes.
    pub sample_rate: u32,
    /// Name of the profile currently loaded into the filter chain.
    /// Used by the background routing thread to avoid needless re-applies.
    pub active_profile_name: String,
    /// When true, the filter chain is held at bypass_all() so audio passes
    /// through unmodified regardless of the active profile. The profile is
    /// still tracked so it can be restored the moment bypass is disabled.
    pub bypassed: bool,
    /// Latest spectrum magnitude data (SPECTRUM_BANDS dB values, 20 Hz–20 kHz).
    /// Written by the capture thread's SpectrumAnalyzer; read by get_spectrum.
    /// Reset to -100 dB when the engine stops so the frontend shows silence.
    pub spectrum: Arc<Mutex<Vec<f32>>>,
    /// Device IDs the engine was most recently started with.
    /// Stored so the device-watch background thread can attempt an automatic
    /// restart with the same configuration when a device error is detected.
    pub started_capture_device: Option<String>,
    pub started_render_device: Option<String>,
    /// Desired output gain (linear, [0.0, 4.0]). Persisted across engine restarts
    /// so the user's boost setting survives Stop/Start cycles.
    output_gain_saved: f32,
    /// Live handle into the renderer's output_gain atomic. Valid while the engine
    /// is running; None when stopped.
    output_gain_arc: Option<Arc<AtomicU32>>,
}

impl AudioEngine {
    pub fn new() -> Self {
        Self {
            capture: None,
            renderer: None,
            vol_monitor: VolumeMonitor::new(),
            filter_chain: Arc::new(Mutex::new(FilterChain::new(48_000.0))),
            crossfeed: Arc::new(Mutex::new(CrossfeedProcessor::new())),
            sample_rate: 48_000,
            active_profile_name: String::new(),
            bypassed: false,
            spectrum: Arc::new(Mutex::new(vec![-100.0; SPECTRUM_BANDS])),
            started_capture_device: None,
            started_render_device: None,
            output_gain_saved: 1.0,
            output_gain_arc: None,
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

        // Save device IDs before moving them into the audio stack, so the
        // device-watch thread can read them without holding the engine lock.
        self.started_capture_device = capture_device_id.clone();
        self.started_render_device  = render_device_id.clone();

        // Start the renderer first so the sender is ready for the capture callback.
        let mut renderer = WasapiRenderer::new();
        let sender = renderer.sender();
        renderer.start(render_device_id)?;

        // Start the volume monitor against the capture device (VB-Cable).
        // VolumeMonitor::start() is idempotent if already running; stop() is
        // called in self.stop() so there is no leak across engine restarts.
        let vol_handle = renderer.volume_handle();
        self.vol_monitor.start(capture_device_id.clone(), vol_handle);

        // Wire in the user-controlled output gain. We grab a handle to the
        // renderer's atomic and store it so set_output_gain() can write to it
        // live without restarting the engine. Apply the saved value immediately.
        let gain_arc = renderer.gain_handle();
        gain_arc.store(self.output_gain_saved.to_bits(), Ordering::Relaxed);
        self.output_gain_arc = Some(gain_arc);

        // Create a fresh spectrum buffer for this engine session.
        // The SpectrumAnalyzer is created after capture.start() so we have the
        // real sample_rate; it is injected into the running closure via this cell.
        //
        // The cell holds Option<SpectrumAnalyzer>: None until start() returns,
        // then set to Some immediately. The closure uses try_lock() so the brief
        // window where it is None causes at most a few missed frames at startup.
        let spectrum_buf = Arc::new(Mutex::new(vec![-100.0f32; SPECTRUM_BANDS]));
        let spec_cell: Arc<Mutex<Option<SpectrumAnalyzer>>> = Arc::new(Mutex::new(None));
        let spec_for_closure = Arc::clone(&spec_cell);

        // Start capture. The callback pushes each processed buffer into the
        // renderer's ring and feeds the spectrum analyzer.
        let mut capture = LoopbackCapture::new();
        let fmt = capture.start(capture_device_id, Arc::clone(&self.filter_chain), Arc::clone(&self.crossfeed), move |samples| {
            sender.write(samples);
            // try_lock: non-blocking; skips spectrum analysis if the cell is
            // being written by the main thread (only at startup, negligible).
            if let Ok(mut g) = spec_for_closure.try_lock() {
                if let Some(a) = g.as_mut() {
                    a.push_samples(samples);
                }
            }
        })?;

        // Now that start() returned we have the real sample_rate.
        // Create the SpectrumAnalyzer with the correct rate and install it.
        let analyzer = SpectrumAnalyzer::new(fmt.sample_rate, Arc::clone(&spectrum_buf));
        *spec_cell.lock().unwrap() = Some(analyzer);
        // spec_cell's refcount stays at 1 inside the closure; our local clone
        // drops here, so the analyzer lives exactly as long as the capture thread.

        self.sample_rate = fmt.sample_rate;
        self.spectrum    = spectrum_buf;
        self.renderer = Some(renderer);
        self.capture = Some(capture);

        Ok(fmt)
    }

    /// Stops both threads cleanly. No-op if not running.
    pub fn stop(&mut self) {
        self.vol_monitor.stop();
        if let Some(mut c) = self.capture.take() {
            c.stop();
        }
        if let Some(mut r) = self.renderer.take() {
            r.stop();
        }
        // Reset spectrum to silence so the canvas clears immediately.
        if let Ok(mut buf) = self.spectrum.lock() {
            for v in buf.iter_mut() { *v = -100.0; }
        }
        self.started_capture_device = None;
        self.started_render_device  = None;
        self.output_gain_arc = None;
    }

    /// Returns true if a WASAPI thread exited unexpectedly due to a device error.
    ///
    /// The device-watch background thread calls this every 2 seconds while the
    /// engine is running. On true, it stops and restarts the engine automatically.
    pub fn has_audio_thread_crashed(&self) -> bool {
        self.capture.as_ref().map_or(false,  |c| c.has_crashed())
            || self.renderer.as_ref().map_or(false, |r| r.has_crashed())
    }

    pub fn is_running(&self) -> bool {
        self.capture.is_some()
    }

    /// Replaces the active FilterChain with one built from `profile`.
    ///
    /// Takes effect on the next audio buffer — no dropout or glitch because
    /// the capture thread locks the Mutex per-packet and sees the new chain
    /// on its very next iteration.
    ///
    /// If bypass is currently active, the chain is immediately zeroed out after
    /// being built so audio continues to pass through unmodified. This keeps
    /// the background routing thread safe: it can call apply_profile() freely
    /// without breaking bypass.
    pub fn apply_profile(&mut self, profile: &Profile) {
        let chain = profile.to_filter_chain(self.sample_rate as f64);
        let mut lock = self.filter_chain.lock().unwrap();
        *lock = chain;
        if self.bypassed {
            lock.bypass_all();
        }
        self.active_profile_name = profile.name.clone();

        // Update crossfeed after releasing the filter chain lock to respect
        // lock ordering (filter_chain and crossfeed are independent here,
        // but we take them separately to keep the pattern explicit).
        self.crossfeed.lock().unwrap()
            .update_config(&profile.crossfeed, self.sample_rate as f64);
    }

    /// Updates the crossfeed configuration directly without a full profile apply.
    ///
    /// Used by `set_profile_crossfeed` when only the crossfeed settings change
    /// and a full EQ chain rebuild would be unnecessarily expensive.
    pub fn apply_crossfeed_config(&self, config: &CrossfeedConfig) {
        self.crossfeed.lock().unwrap()
            .update_config(config, self.sample_rate as f64);
    }

    /// Sets the output gain (linear, clamped to [0.0, 4.0]).
    ///
    /// Takes effect on the next render buffer — no dropout. Persists across
    /// engine restarts via `output_gain_saved`.
    pub fn set_output_gain(&mut self, gain: f32) {
        let clamped = gain.clamp(0.0, 4.0);
        self.output_gain_saved = clamped;
        if let Some(arc) = &self.output_gain_arc {
            arc.store(clamped.to_bits(), Ordering::Relaxed);
        }
    }

    /// Returns the currently stored output gain (linear).
    pub fn get_output_gain(&self) -> f32 {
        self.output_gain_saved
    }

    /// Enables or disables the global EQ bypass.
    ///
    /// When enabling bypass, the live filter chain is immediately zeroed (passthrough).
    /// When disabling, the caller is responsible for re-applying the active profile
    /// because AudioEngine has no access to the ProfileStore.
    pub fn set_bypass(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
        if bypassed {
            self.filter_chain.lock().unwrap().bypass_all();
        }
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}
