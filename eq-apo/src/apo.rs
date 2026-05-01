// eq-apo/src/apo.rs
//
// SoundEqApo — the core COM object loaded by audiodg.exe into the audio pipeline.
//
// An APO (Audio Processing Object) implements a set of COM interfaces that the
// Windows audio engine calls to set up and drive real-time audio processing.
// We sit in the MFX (Mode Effects) slot — PKEY_FX_ModeEffectClsid, index 4 in
// FxProperties. MFX runs on the post-mix combined stream, after endpoint volume
// is applied, so the Windows volume slider works correctly while soundEQ is active.
//
// Required interfaces:
//   IAudioProcessingObject            — initialization and format negotiation
//   IAudioProcessingObjectConfiguration — LockForProcess / UnlockForProcess
//   IAudioProcessingObjectRT          — real-time audio processing (APOProcess)
//   IAudioSystemEffects               — marker interface (no methods)
//   IAudioSystemEffects2              — EffectsChanged notification support
//
// Phase 2 design:
//   Initialize       — reads active_profile.json from %PUBLIC%\soundEQ\,
//                      parses bands + sample rate, stores them in ApoState.
//   LockForProcess   — builds a FilterChain from the stored bands and sample
//                      rate, ready for APOProcess to use.
//   APOProcess       — applies the FilterChain in-place to each audio buffer.
//                      NO allocation here — FilterChain is stack-allocated and
//                      process_interleaved() uses only fixed-size arrays.
//
// Thread safety:
//   ApoState is behind a Mutex. LockForProcess (config thread) calls lock().
//   APOProcess (RT audio thread) calls try_lock() — if the config thread holds
//   the lock, APOProcess skips that buffer rather than blocking the RT thread.
//   A skipped buffer is a brief glitch; a blocked RT thread is a hard dropout.

#![allow(non_snake_case, unused_variables)]

use std::sync::Mutex;

use windows::{
    core::{implement, Result, GUID},
    Win32::Foundation::{E_NOTIMPL, E_OUTOFMEMORY, HANDLE},
    Win32::Media::Audio::Apo::{
        APO_CONNECTION_DESCRIPTOR, APO_CONNECTION_PROPERTY,
        APO_FLAG_INPLACE,
        APO_REG_PROPERTIES, BUFFER_SILENT,
        IAudioMediaType,
        IAudioProcessingObject, IAudioProcessingObject_Impl,
        IAudioProcessingObjectConfiguration,
        IAudioProcessingObjectConfiguration_Impl,
        IAudioProcessingObjectRT, IAudioProcessingObjectRT_Impl,
        IAudioSystemEffects, IAudioSystemEffects_Impl,
        IAudioSystemEffects2, IAudioSystemEffects2_Impl,
    },
    Win32::System::Com::CoTaskMemAlloc,
};

use eq_core::{BandConfig, FilterChain, Profile};

use crate::{decrement_object_count, increment_object_count};

fn trace(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
        .open("C:\\Users\\Public\\soundEQ\\apo_trace.log")
    {
        let _ = f.write_all(msg.as_bytes());
        let _ = f.write_all(b"\n");
    }
}

// ── Shared state file ─────────────────────────────────────────────────────────
//
// The Tauri app writes this JSON file whenever the active EQ profile changes.
// %PUBLIC% (C:\Users\Public) is writable by all authenticated users and
// readable by audiodg.exe regardless of its service identity.

/// JSON structure written by the Tauri app to %PUBLIC%\soundEQ\active_profile.json.
#[derive(serde::Deserialize)]
struct ApoStateFile {
    /// Profile name (informational only — we apply the bands regardless).
    #[allow(dead_code)]
    name: String,
    /// EQ bands to apply. Empty = passthrough.
    bands: Vec<BandConfig>,
    /// Sample rate in Hz from the running WASAPI session (e.g. 48_000).
    /// Used to compute biquad coefficients — the formula is sample-rate-dependent.
    sample_rate: u32,
}

/// Returns the path to the APO shared state file.
fn apo_config_path() -> std::path::PathBuf {
    // %PUBLIC% is set machine-wide (not per-user) and is readable by all
    // processes including audiodg.exe running as LOCAL SERVICE.
    let public = std::env::var("PUBLIC").unwrap_or_else(|_| r"C:\Users\Public".into());
    std::path::Path::new(&public)
        .join("soundEQ")
        .join("active_profile.json")
}

/// Reads and parses the APO shared state file.
///
/// Returns `(bands, sample_rate)`. On any error (file absent, bad JSON,
/// first launch before the Tauri app has run) returns an empty band list
/// and a safe default of 48 000 Hz, so the chain runs as passthrough.
fn read_apo_config() -> (Vec<BandConfig>, u32) {
    let path = apo_config_path();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return (Vec::new(), 48_000),
    };
    match serde_json::from_slice::<ApoStateFile>(&bytes) {
        Ok(state) => (state.bands, state.sample_rate),
        Err(_) => (Vec::new(), 48_000),
    }
}

// ── Internal state ────────────────────────────────────────────────────────────

/// State shared between the configuration and RT interfaces, protected by Mutex.
struct ApoState {
    /// Sample rate in Hz from the shared state file, refreshed in Initialize.
    /// Used in LockForProcess to compute biquad coefficients for the FilterChain.
    sample_rate: u32,

    /// Whether LockForProcess has been called (stream is running).
    locked: bool,

    /// The active filter chain. Rebuilt in LockForProcess; applied in APOProcess.
    ///
    /// FilterChain is a fixed-size struct ([FilterBand; MAX_BANDS] array) — no
    /// heap allocation. Holding it inside the Mutex is fine because the RT thread
    /// accesses it only through try_lock(), which doesn't allocate.
    filter_chain: FilterChain,

    /// Bands read from the shared state file in Initialize.
    /// Stored here so LockForProcess can (re)build the FilterChain after the
    /// negotiated sample rate is known. This Vec is only ever touched from the
    /// configuration thread, never from the RT thread.
    pending_bands: Vec<BandConfig>,
}

impl ApoState {
    fn new() -> Self {
        ApoState {
            sample_rate: 48_000,
            locked: false,
            filter_chain: FilterChain::new(48_000.0),
            pending_bands: Vec::new(),
        }
    }
}

// ── COM object ────────────────────────────────────────────────────────────────

/// The APO COM object. One instance is created per audio stream endpoint.
///
/// `#[implement]` generates the COM vtable and IUnknown ref-counting boilerplate
/// so we only write the interface method bodies.
#[implement(
    IAudioProcessingObject,
    IAudioProcessingObjectConfiguration,
    IAudioProcessingObjectRT,
    IAudioSystemEffects,
    IAudioSystemEffects2
)]
pub struct SoundEqApo {
    state: Mutex<ApoState>,
}

impl SoundEqApo {
    pub fn new() -> Self {
        increment_object_count();
        trace("SoundEqApo::new() — COM object created");
        SoundEqApo { state: Mutex::new(ApoState::new()) }
    }
}

impl Drop for SoundEqApo {
    fn drop(&mut self) {
        decrement_object_count();
    }
}

// ── IAudioProcessingObject ────────────────────────────────────────────────────
// Called once by audiodg.exe during audio graph setup, before streaming starts.

impl IAudioProcessingObject_Impl for SoundEqApo_Impl {
    fn Initialize(&self, _cbydata: u32, _pbydata: *const u8) -> Result<()> {
        trace("Initialize called");
        let (bands, sample_rate) = read_apo_config();
        let mut state = self.state.lock().unwrap();
        state.pending_bands = bands;
        state.sample_rate = sample_rate;

        // DIAGNOSTIC — write a log file so we can confirm audiodg.exe loaded
        // the DLL and called Initialize. Remove after debugging.
        let msg = format!(
            "Initialize called: {} bands, sample_rate={}\n",
            state.pending_bands.len(),
            state.sample_rate,
        );
        let _ = std::fs::write("C:\\Users\\Public\\soundEQ\\apo_debug.log", msg);
        Ok(())
    }

    fn IsInputFormatSupported(
        &self,
        _poutputformat: Option<&IAudioMediaType>,
        _prequestedinputformat: Option<&IAudioMediaType>,
    ) -> Result<IAudioMediaType> {
        trace("IsInputFormatSupported called");
        // E_NOTIMPL signals "accept any format" — the audio engine uses the
        // format it prefers without asking us to convert.
        Err(E_NOTIMPL.into())
    }

    fn IsOutputFormatSupported(
        &self,
        _pinputformat: Option<&IAudioMediaType>,
        _prequestedoutputformat: Option<&IAudioMediaType>,
    ) -> Result<IAudioMediaType> {
        trace("IsOutputFormatSupported called");
        Err(E_NOTIMPL.into())
    }

    fn GetInputChannelCount(&self) -> Result<u32> {
        Ok(2) // stereo
    }

    fn GetLatency(&self) -> Result<i64> {
        Ok(0) // no additional latency; units are 100-nanosecond intervals
    }

    fn GetRegistrationProperties(&self) -> Result<*mut APO_REG_PROPERTIES> {
        // audiodg.exe calls this (or reads the CLSID\Properties registry subkey)
        // before it even tries to load the DLL. Returning E_NOTIMPL with an
        // empty Properties subkey causes it to silently skip our APO entirely.
        // We must return a valid APO_REG_PROPERTIES allocated with CoTaskMemAlloc;
        // the audio engine frees it with CoTaskMemFree.
        let ptr = unsafe { CoTaskMemAlloc(std::mem::size_of::<APO_REG_PROPERTIES>()) }
            as *mut APO_REG_PROPERTIES;
        if ptr.is_null() {
            return Err(E_OUTOFMEMORY.into());
        }
        unsafe {
            // Zero-initialize first (APO_REG_PROPERTIES::default() is mem::zeroed).
            ptr.write(APO_REG_PROPERTIES::default());
            let p = &mut *ptr;

            p.clsid = crate::CLSID_SOUND_EQ_APO;
            // APO_FLAG_INPLACE: MFX APOs operate on a shared buffer — the same
            // memory region serves as both input and output.
            p.Flags = APO_FLAG_INPLACE;

            // Null-terminated UTF-16 friendly name inside the fixed 256-wchar array.
            // default() already zeroed the array so the null terminator is implicit.
            for (i, c) in "SoundEQ Parametric EQ".encode_utf16().enumerate().take(255) {
                p.szFriendlyName[i] = c;
            }

            p.u32MajorVersion = 1;
            p.u32MinorVersion = 0;
            // MFX: exactly 1 input connection and 1 output connection (the same buffer).
            p.u32MinInputConnections  = 1;
            p.u32MaxInputConnections  = 1;
            p.u32MinOutputConnections = 1;
            p.u32MaxOutputConnections = 1;
            p.u32MaxInstances    = 0; // 0 = unlimited
            p.u32NumAPOInterfaces = 0; // no extension interfaces advertised here
        }
        Ok(ptr)
    }

    fn Reset(&self) -> Result<()> {
        Ok(())
    }
}

// ── IAudioProcessingObjectConfiguration ──────────────────────────────────────
// Called on the non-RT thread when the audio engine transitions the stream
// into or out of the streaming state.

impl IAudioProcessingObjectConfiguration_Impl for SoundEqApo_Impl {
    /// Called before APOProcess starts receiving buffers.
    ///
    /// We build the FilterChain here (on the config thread) from the bands
    /// and sample rate stored during Initialize. Building here rather than in
    /// Initialize gives us the opportunity to use the actually-negotiated format
    /// in Phase 3. For Phase 2 we use the sample rate from the JSON file which
    /// matches the running WASAPI session rate.
    fn LockForProcess(
        &self,
        _u32numinputconnections: u32,
        _ppinputconnections: *const *const APO_CONNECTION_DESCRIPTOR,
        _u32numoutputconnections: u32,
        _ppoutputconnections: *const *const APO_CONNECTION_DESCRIPTOR,
    ) -> Result<()> {
        let mut state = self.state.lock().unwrap();

        // Build the FilterChain from bands read in Initialize.
        // Profile::with_bands / to_filter_chain allocate on this config thread —
        // that is fine. APOProcess (RT thread) never allocates.
        let profile = Profile::with_bands("active".to_string(), state.pending_bands.clone());
        state.filter_chain = profile.to_filter_chain(state.sample_rate as f64);
        state.locked = true;
        Ok(())
    }

    fn UnlockForProcess(&self) -> Result<()> {
        self.state.lock().unwrap().locked = false;
        Ok(())
    }
}

// ── IAudioProcessingObjectRT ──────────────────────────────────────────────────
// The real-time audio callback. Called by audiodg.exe on the audio thread for
// every buffer period (typically 10 ms at 48 kHz = 480 frames).
//
// RT thread rules (same as eq-audio):
//   - NO heap allocation (no Vec, Box, String)
//   - NO blocking (use try_lock, not lock)
//   - NO logging (I/O blocks the thread)

impl IAudioProcessingObjectRT_Impl for SoundEqApo_Impl {
    /// Apply EQ filtering to the audio buffer in-place.
    ///
    /// MFX APOs share the input/output buffer — modifying pconnectionsinput[0]->pBuffer
    /// IS the output. The audio engine reads the result after we return.
    fn APOProcess(
        &self,
        u32numinputconnections: u32,
        pconnectionsinput: *const *const APO_CONNECTION_PROPERTY,
        _u32numoutputconnections: u32,
        _pconnectionsoutput: *mut *mut APO_CONNECTION_PROPERTY,
    ) {
        if u32numinputconnections == 0 || pconnectionsinput.is_null() {
            return;
        }

        // SAFETY:
        // - pconnectionsinput[0] is guaranteed valid for the call duration by
        //   the APO contract (audiodg.exe owns the buffer and does not free it
        //   while APOProcess is running).
        // - pBuffer is a usize holding the address of the audio buffer, which
        //   contains exactly u32ValidFrameCount * 2 (stereo) f32 samples.
        // - We write only within those bounds — no buffer overrun is possible.
        unsafe {
            let prop_ptr = *pconnectionsinput;
            if prop_ptr.is_null() { return; }
            let prop = &*prop_ptr;

            // BUFFER_SILENT (= 2) means the buffer contains no valid audio.
            // Skip processing — the output is already silence and running the
            // EQ on silence would needlessly dirty the filter delay lines.
            if prop.u32BufferFlags == BUFFER_SILENT { return; }

            let frame_count = prop.u32ValidFrameCount as usize;
            if frame_count == 0 { return; }

            // Build a mutable slice over the interleaved stereo samples.
            // Layout: [L0, R0, L1, R1, ..., L(n-1), R(n-1)]
            let samples = std::slice::from_raw_parts_mut(
                prop.pBuffer as *mut f32,
                frame_count * 2,
            );

            // DIAGNOSTIC — log the first APOProcess call to confirm the RT
            // callback is actually being invoked. Remove after debugging.
            static PROCESS_LOGGED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !PROCESS_LOGGED.load(std::sync::atomic::Ordering::Relaxed) {
                PROCESS_LOGGED.store(true, std::sync::atomic::Ordering::Relaxed);
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .append(true).open("C:\\Users\\Public\\soundEQ\\apo_debug.log")
                {
                    let _ = writeln!(f, "APOProcess first call: {} frames", frame_count);
                }
            }

            if let Ok(mut st) = self.state.try_lock() {
                st.filter_chain.process_interleaved(samples);
            }
        }
    }

    fn CalcInputFrames(&self, output_frame_count: u32) -> u32 {
        output_frame_count // 1:1 — we don't change the frame count
    }

    fn CalcOutputFrames(&self, input_frame_count: u32) -> u32 {
        input_frame_count
    }
}

// ── IAudioSystemEffects ───────────────────────────────────────────────────────
// Marker interface — no methods. Presence signals to Windows that this is a
// system effects APO rather than a raw audio transform.

impl IAudioSystemEffects_Impl for SoundEqApo_Impl {}

// ── IAudioSystemEffects2 ──────────────────────────────────────────────────────
// Extends IAudioSystemEffects with an event mechanism so the audio engine can
// notify the APO when the effect list changes (Phase 3: profile hot-reload).

impl IAudioSystemEffects2_Impl for SoundEqApo_Impl {
    fn GetEffectsList(
        &self,
        ppeffectsids: *mut *mut GUID,
        pceffects: *mut u32,
        _event: HANDLE,
    ) -> Result<()> {
        // Return an empty effects list — we have no discrete toggle-able effects;
        // the entire EQ curve is treated as one transparent gain stage.
        // SAFETY: the audio engine guarantees these pointers are valid.
        unsafe {
            if !ppeffectsids.is_null() { *ppeffectsids = std::ptr::null_mut(); }
            if !pceffects.is_null()    { *pceffects    = 0; }
        }
        Ok(())
    }
}
