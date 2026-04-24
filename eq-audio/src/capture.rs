// =============================================================================
// capture.rs — WASAPI loopback capture
//
// WHAT THIS DOES:
// Opens the system's default audio render (output) endpoint in "loopback"
// mode, which lets us read whatever audio is currently playing — even though
// we are a different process from the one generating the sound. This is how
// system-wide EQ works: we intercept the mixed audio before it reaches the
// speaker driver.
//
// WASAPI LOOPBACK — QUICK PRIMER:
// Windows Audio Session API (WASAPI) has two share modes:
//   - Exclusive: the app owns the device (low latency, used by DAWs)
//   - Shared:    the Windows audio engine mixes all apps together
//
// In shared mode, the engine exposes a "loopback" flag that makes a render
// endpoint behave like a capture endpoint. We read the already-mixed output
// stream rather than raw audio from a microphone.
//
// THREADING MODEL:
// All WASAPI objects are created and used on the capture thread.
// The control thread (caller) communicates via:
//   - Arc<AtomicBool>        — stop signal
//   - Arc<Mutex<FilterChain>> — live EQ parameter updates
//   - mpsc channel           — one-shot format report at startup
//
// SAFETY:
// This module contains several `unsafe` blocks. Each one is annotated with
// why unsafe is required and what invariants make it sound.
// =============================================================================

use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{
    IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    WAVEFORMATEX, WAVEFORMATEXTENSIBLE, eConsole, eRender,
};
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};

use eq_core::FilterChain;

use crate::error::AudioError;

// ---------------------------------------------------------------------------
// WAVE_FORMAT_* tag constants (from the Windows Multimedia SDK)
//
// GetMixFormat returns a WAVEFORMATEX whose wFormatTag tells us the encoding.
// EXTENSIBLE is the modern variant that carries a SubFormat GUID for more
// detailed format identification (e.g. float vs PCM at a given bit depth).
// ---------------------------------------------------------------------------
// These constants are shared with render.rs (which uses the same format parsing logic).
pub(crate) const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
pub(crate) const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

// ---------------------------------------------------------------------------
// StreamFormat — describes the audio stream we captured from WASAPI
//
// The caller needs this to create a FilterChain with the correct sample rate
// and to know how many channels are in each buffer.
// ---------------------------------------------------------------------------

/// Properties of the captured audio stream, reported after setup completes.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct StreamFormat {
    /// Samples per second per channel (e.g. 44100 or 48000).
    /// A "sample" is one amplitude measurement for one channel.
    pub sample_rate: u32,

    /// Number of audio channels (2 for stereo, 6 for 5.1, etc.).
    /// The EQ engine currently requires 2 (stereo).
    pub channels: u16,
}

// ---------------------------------------------------------------------------
// LoopbackCapture — the public API
// ---------------------------------------------------------------------------

/// Opens a WASAPI loopback session and processes captured audio through a
/// FilterChain on a dedicated background thread.
///
/// Lifecycle:
/// 1. `LoopbackCapture::new()` — create the handle.
/// 2. Create a `FilterChain` (see `start()` return value for the sample rate).
/// 3. `start(filter_chain, callback)` — begin capturing; returns the stream format.
/// 4. Update `filter_chain` from any thread (protected by Mutex) to change EQ live.
/// 5. `stop()` — signal the capture thread to exit and wait for it.
pub struct LoopbackCapture {
    stop_flag: Arc<AtomicBool>,
    thread_handle: Option<JoinHandle<()>>,
}

impl LoopbackCapture {
    /// Creates a new (idle) capture handle. Does not open any WASAPI resources
    /// or start any threads — call `start()` for that.
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        }
    }

    /// Starts the capture thread and begins processing audio.
    ///
    /// `capture_device_id` — the WASAPI endpoint ID of the device to capture
    /// from (e.g. the VB-Cable "CABLE Input" device). Pass `None` to capture
    /// from the current system-default output device.
    ///
    /// `filter_chain` is applied to each buffer on the capture thread.
    /// You can update its bands from any other thread (the Mutex protects it).
    ///
    /// `on_processed` receives a reference to the processed interleaved f32
    /// buffer after the EQ is applied. In the routing pipeline this callback
    /// feeds the WasapiRenderer's sample queue.
    ///
    /// Returns the `StreamFormat` (sample rate + channel count) reported by
    /// the audio device so the caller can configure the FilterChain correctly.
    pub fn start(
        &mut self,
        capture_device_id: Option<String>,
        filter_chain: Arc<Mutex<FilterChain>>,
        on_processed: impl FnMut(&[f32]) + Send + 'static,
    ) -> Result<StreamFormat, AudioError> {
        if self.thread_handle.is_some() {
            return Err(AudioError::AlreadyRunning);
        }

        // Reset the stop flag in case this instance is being restarted.
        self.stop_flag.store(false, Ordering::Relaxed);

        // Synchronous channel (capacity 1) carries the format report (or error)
        // from the capture thread back to this thread. We block until setup
        // completes so the caller can immediately use the returned StreamFormat.
        let (format_tx, format_rx) = mpsc::sync_channel::<Result<StreamFormat, AudioError>>(1);

        let stop_flag = Arc::clone(&self.stop_flag);

        let handle = thread::Builder::new()
            .name("wasapi-loopback-capture".to_string())
            .spawn(move || {
                capture_thread_main(stop_flag, filter_chain, format_tx, on_processed, capture_device_id);
            })?;

        self.thread_handle = Some(handle);

        // Block until the capture thread reports the format or fails.
        // If the thread panics before sending, recv() returns Err(RecvError).
        format_rx.recv().map_err(|_| AudioError::ThreadSetupFailed)?
    }

    /// Signals the capture thread to stop and waits for it to exit cleanly.
    /// No-op if capture was never started or has already stopped.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            // join() blocks until the thread's run loop sees the flag and exits.
            // We discard the result — a panicking capture thread has already
            // logged its error and there is nothing useful to do here.
            let _ = handle.join();
        }
    }

    /// Returns true if a capture session is currently active.
    pub fn is_running(&self) -> bool {
        self.thread_handle.is_some()
    }
}

impl Default for LoopbackCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LoopbackCapture {
    fn drop(&mut self) {
        // Ensure the background thread is stopped if the caller forgets.
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Capture thread — everything below runs on the background thread
// ---------------------------------------------------------------------------

/// Entry point for the capture thread.
///
/// Responsible for:
/// 1. Initialising COM (MTA) for this thread.
/// 2. Opening the WASAPI loopback device and reporting the format.
/// 3. Running the capture/process loop until the stop flag is set.
/// 4. Cleaning up COM on exit.
fn capture_thread_main(
    stop_flag: Arc<AtomicBool>,
    filter_chain: Arc<Mutex<FilterChain>>,
    format_tx: SyncSender<Result<StreamFormat, AudioError>>,
    mut on_processed: impl FnMut(&[f32]),
    capture_device_id: Option<String>,
) {
    // RAII guard: CoUninitialize is called automatically when _com drops.
    // It must outlive all COM objects created below — Rust drops locals in
    // reverse declaration order, so declaring it first means it drops last.
    let _com = match ComGuard::init() {
        Ok(g) => g,
        Err(e) => { let _ = format_tx.send(Err(e)); return; }
    };

    // Open the loopback device and query its mix format.
    let (audio_client, capture_client, format) = match open_loopback(capture_device_id.as_deref()) {
        Ok(v) => v,
        Err(e) => { let _ = format_tx.send(Err(e)); return; }
    };

    // Report the format to the starting thread so it can configure its
    // FilterChain with the correct sample rate.
    let _ = format_tx.send(Ok(format));

    // Update the FilterChain's sample rate to match this device.
    // This resets filter state, which is fine at startup.
    {
        let mut chain = filter_chain.lock().unwrap();
        chain.set_sample_rate(format.sample_rate as f64);
    }

    // unsafe: Start() has no invariants beyond the audio client being properly
    // initialised, which open_loopback() guarantees before returning.
    if let Err(e) = unsafe { audio_client.Start() } {
        eprintln!("[eq-audio] IAudioClient::Start failed: {e}");
        return;
    }

    // Pre-allocate the processing buffer once so the hot loop never allocates.
    // Capacity of 4096 f32 samples covers most WASAPI buffer sizes
    // (typical WASAPI shared-mode buffer ≈ 480 frames × 2 channels = 960 samples).
    let mut processing_buf: Vec<f32> = Vec::with_capacity(4096);

    run_capture_loop(
        &stop_flag,
        &filter_chain,
        &capture_client,
        &format,
        &mut processing_buf,
        &mut on_processed,
    );

    // unsafe: Stop() on a started client is always valid.
    let _ = unsafe { audio_client.Stop() };
}

/// The hot loop: read WASAPI packets, apply EQ, call the sink.
///
/// Extracted from `capture_thread_main` so the setup/teardown is clearly
/// separated from the steady-state path.
fn run_capture_loop(
    stop_flag: &AtomicBool,
    filter_chain: &Mutex<FilterChain>,
    capture_client: &IAudioCaptureClient,
    format: &StreamFormat,
    processing_buf: &mut Vec<f32>,
    on_processed: &mut impl FnMut(&[f32]),
) {
    while !stop_flag.load(Ordering::Relaxed) {
        // GetNextPacketSize returns the number of frames in the next packet.
        // 0 means no data is ready yet — we sleep briefly rather than busy-wait.
        //
        // unsafe: capture_client is valid (alive for the duration of this function)
        // and GetNextPacketSize has no preconditions beyond that.
        let packet_frames = match unsafe { capture_client.GetNextPacketSize() } {
            Ok(n) => n,
            Err(e) => { eprintln!("[eq-audio] GetNextPacketSize: {e}"); break; }
        };

        if packet_frames == 0 {
            // Typical loopback period is 10 ms. Sleeping 5 ms here means we
            // check roughly twice per period — low CPU, adequate latency.
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        // Drain all available packets before sleeping again.
        loop {
            let mut data_ptr: *mut u8 = ptr::null_mut();
            let mut num_frames: u32 = 0;
            let mut flags: u32 = 0;

            // unsafe: GetBuffer writes to our out-params and returns a pointer
            // into the WASAPI-managed capture buffer. The pointer is valid until
            // ReleaseBuffer is called (immediately after this block).
            let get_result = unsafe {
                capture_client.GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None)
            };

            if let Err(e) = get_result {
                eprintln!("[eq-audio] GetBuffer: {e}");
                break;
            }

            // AUDCLNT_BUFFERFLAGS_SILENT means the driver had no real data —
            // the buffer contents are undefined and we must treat it as silence.
            let is_silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;

            if !is_silent && num_frames > 0 {
                let n_samples = num_frames as usize * format.channels as usize;

                // unsafe: data_ptr points to a valid WASAPI buffer of n_samples f32
                // values. The lifetime of this slice is bounded by the ReleaseBuffer
                // call below — we copy into processing_buf before releasing.
                let samples = unsafe {
                    slice::from_raw_parts(data_ptr as *const f32, n_samples)
                };

                // Copy into the pre-allocated processing buffer.
                // For stereo: copy the whole slice directly.
                // For multi-channel (5.1, 7.1, etc.): extract only channels 0 (L)
                // and 1 (R) from each frame — the FilterChain always works in stereo.
                processing_buf.clear();
                if format.channels == 2 {
                    processing_buf.extend_from_slice(samples);
                } else {
                    let ch = format.channels as usize;
                    for frame in samples.chunks_exact(ch) {
                        processing_buf.push(frame[0]); // L
                        processing_buf.push(frame[1]); // R
                    }
                }

                // Apply the EQ. Lock is acquired and released per packet, so
                // the control thread can update bands between packets.
                {
                    let mut chain = filter_chain.lock().unwrap();
                    chain.process_interleaved(processing_buf);
                }

                on_processed(processing_buf);
            }

            // unsafe: num_frames must match the value from GetBuffer.
            // We use the exact value returned, satisfying this requirement.
            if let Err(e) = unsafe { capture_client.ReleaseBuffer(num_frames) } {
                eprintln!("[eq-audio] ReleaseBuffer: {e}");
                break;
            }

            // Check whether another packet is already waiting before sleeping.
            match unsafe { capture_client.GetNextPacketSize() } {
                Ok(0) | Err(_) => break,
                Ok(_) => continue,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// WASAPI setup helpers
// ---------------------------------------------------------------------------

/// Opens a render endpoint in loopback mode.
///
/// `capture_device_id` — WASAPI endpoint ID of the device to capture from
/// (e.g. "CABLE Input (VB-Audio Virtual Cable)"). Pass `None` to use the
/// current system-default render device.
///
/// Returns the IAudioClient (needed for Start/Stop), the IAudioCaptureClient
/// (needed for GetBuffer/ReleaseBuffer), and the parsed StreamFormat.
///
/// All returned COM objects must be used on the same thread (this one).
fn open_loopback(capture_device_id: Option<&str>) -> Result<(IAudioClient, IAudioCaptureClient, StreamFormat), AudioError> {
    // unsafe blocks below: all COM API calls require unsafe in windows-rs.
    // Each call is safe given the preconditions documented inline.

    // unsafe: CoCreateInstance is inherently unsafe (raw COM). We are on an MTA
    // thread (guaranteed by ComGuard), which satisfies WASAPI's threading requirements.
    let enumerator: IMMDeviceEnumerator = unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?
    };

    // Open the requested device or fall back to the system default.
    // eRender + eConsole = the speakers/headphones set as default in Sound settings.
    let device = if let Some(id) = capture_device_id {
        // Convert UTF-8 device ID to a null-terminated UTF-16 string for the Win32 API.
        let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
        // unsafe: PCWSTR borrows wide; wide is alive for the duration of this call.
        unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr()))? }
    } else {
        // unsafe: enumerator is a valid COM object.
        unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? }
    };

    // Activate an IAudioClient on the render device.
    // Passing None for activation parameters uses the default configuration.
    // unsafe: device is a valid IMMDevice.
    let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };

    // GetMixFormat returns a heap-allocated WAVEFORMATEX (or WAVEFORMATEXTENSIBLE).
    // unsafe: audio_client is a valid IAudioClient.
    let fmt_ptr: *mut WAVEFORMATEX = unsafe { audio_client.GetMixFormat()? };

    // Parse the format to extract sample rate and channel count.
    // unsafe: fmt_ptr is non-null and points to a Windows-allocated WAVEFORMATEX.
    let parse_result = unsafe { parse_format(fmt_ptr) };

    // Initialize using the ORIGINAL format pointer from GetMixFormat.
    // This correctly handles WAVEFORMATEXTENSIBLE (multi-channel: 5.1, 7.1, etc.)
    // without needing to reconstruct the struct, which would require a valid
    // channel mask for the extended format.
    //
    // AUDCLNT_STREAMFLAGS_LOOPBACK: read the already-mixed render stream.
    let init_result = unsafe {
        audio_client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            0,    // buffer duration: use device minimum
            0,    // periodicity: must be 0 in shared mode
            fmt_ptr as *const WAVEFORMATEX,
            None, // audio session GUID: use default session
        )
    };

    // Free the COM-allocated format struct now that both parse and Initialize are done.
    // unsafe: CoTaskMemFree frees the COM-allocated pointer exactly once.
    unsafe { CoTaskMemFree(Some(fmt_ptr.cast())) };

    let stream_format = parse_result?;
    init_result?;

    // Get the IAudioCaptureClient service from the initialised audio client.
    // unsafe: audio_client has been successfully initialised immediately above.
    let capture_client: IAudioCaptureClient = unsafe { audio_client.GetService()? };

    Ok((audio_client, capture_client, stream_format))
}

/// Parses a WAVEFORMATEX (or WAVEFORMATEXTENSIBLE) and validates that we can
/// handle the format: 32-bit IEEE float. Any channel count is accepted;
/// multi-channel is downmixed to stereo in the capture loop.
///
/// # Safety
/// `ptr` must be a valid non-null pointer to a WAVEFORMATEX or
/// WAVEFORMATEXTENSIBLE allocated by Windows (e.g. from GetMixFormat).
unsafe fn parse_format(ptr: *const WAVEFORMATEX) -> Result<StreamFormat, AudioError> {
    // Read the base struct. This is safe given the caller's invariant.
    let fmt = &*ptr;

    let sample_rate = fmt.nSamplesPerSec;
    let channels    = fmt.nChannels;
    let bits        = fmt.wBitsPerSample;
    let tag         = fmt.wFormatTag;

    // Determine whether this is float32 format.
    let is_float32 = match tag {
        WAVE_FORMAT_IEEE_FLOAT => bits == 32,

        WAVE_FORMAT_EXTENSIBLE => {
            // WAVEFORMATEXTENSIBLE extends WAVEFORMATEX by appending extra fields.
            // We cast the base pointer to the extended struct.
            //
            // unsafe: WAVEFORMATEXTENSIBLE is repr(C, packed) — every field in it
            // is potentially unaligned relative to the struct's base address.
            // We must NOT create a Rust reference to any field; instead we use
            // ptr::addr_of! to get a raw pointer without a reference, then
            // read_unaligned to safely read the value from that raw pointer.
            let ext: WAVEFORMATEXTENSIBLE = ptr::read_unaligned(ptr as *const WAVEFORMATEXTENSIBLE);
            let sub_format = ptr::addr_of!(ext.SubFormat).read_unaligned();

            // SubFormat is a GUID identifying the PCM sub-encoding.
            // KSDATAFORMAT_SUBTYPE_IEEE_FLOAT = {00000003-0000-0010-8000-00AA00389B71}
            sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT && bits == 32
        }

        _ => false,
    };

    if !is_float32 {
        return Err(AudioError::UnsupportedFormat(format!(
            "tag=0x{tag:04X}, bits={bits} — expected 32-bit IEEE float (tag=0x0003 or 0xFFFE)"
        )));
    }

    Ok(StreamFormat { sample_rate, channels })
}

/// Builds a minimal WAVEFORMATEX for float32 stereo at the given sample rate.
///
/// Used when calling `IAudioClient::Initialize` — we pass back a format
/// consistent with what GetMixFormat told us, which guarantees the Initialize
/// call accepts it in shared mode.
fn make_waveformatex(fmt: &StreamFormat) -> WAVEFORMATEX {
    // In shared mode, WASAPI accepts the exact format returned by GetMixFormat.
    // block_align = channels × (bits_per_sample / 8) = 2 × 4 = 8 bytes per frame.
    // avg_bytes_per_sec = sample_rate × block_align.
    let block_align: u16 = fmt.channels * 4; // 4 bytes per sample (f32)
    WAVEFORMATEX {
        wFormatTag:      WAVE_FORMAT_IEEE_FLOAT,
        nChannels:       fmt.channels,
        nSamplesPerSec:  fmt.sample_rate,
        nAvgBytesPerSec: fmt.sample_rate * block_align as u32,
        nBlockAlign:     block_align,
        wBitsPerSample:  32,
        cbSize:          0, // no extra bytes — we're using plain WAVEFORMATEX
    }
}

// ---------------------------------------------------------------------------
// COM initialisation guard
// ---------------------------------------------------------------------------

/// RAII wrapper for CoInitializeEx / CoUninitialize.
///
/// Creates a COM multi-threaded apartment (MTA) on the current thread.
/// CoUninitialize is called automatically when this guard is dropped,
/// which is why it must be declared BEFORE any COM objects in the same scope.
struct ComGuard;

impl ComGuard {
    fn init() -> Result<Self, AudioError> {
        // unsafe: CoInitializeEx must be called once per thread before using COM.
        // COINIT_MULTITHREADED (MTA) allows COM objects to be used from any thread,
        // which is what WASAPI objects require when accessed from a worker thread.
        //
        // Returns S_OK (0) on first init or S_FALSE (1) if already initialised
        // by a prior call on this thread — both are success codes (HRESULT >= 0).
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        hr.ok()?;
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // unsafe: CoUninitialize must be called once for each successful
        // CoInitializeEx. This guard ensures exactly that pairing.
        unsafe { CoUninitialize() };
    }
}

// ---------------------------------------------------------------------------
// Tests
//
// WASAPI requires real audio hardware and a running Windows audio service.
// Tests that exercise the actual device are marked #[ignore] — run them
// manually with `cargo test -- --ignored` on a machine with audio.
// Logic tests (format parsing, struct construction) run without hardware.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // --- Format parsing (no hardware needed) ---

    // WAVEFORMATEX is repr(packed) in windows-rs, so taking a reference to any
    // field (which assert_eq! does internally) triggers E0793.
    // We copy each field to a plain local before asserting.
    fn read_wfx_fields(wfx: &WAVEFORMATEX) -> (u16, u16, u32, u32, u16, u16) {
        // Safety: wfx is a valid reference; addr_of! avoids creating a reference
        // to a potentially-unaligned field, and read_unaligned handles misalignment.
        unsafe {(
            ptr::addr_of!(wfx.wFormatTag).read_unaligned(),
            ptr::addr_of!(wfx.nChannels).read_unaligned(),
            ptr::addr_of!(wfx.nSamplesPerSec).read_unaligned(),
            ptr::addr_of!(wfx.nAvgBytesPerSec).read_unaligned(),
            ptr::addr_of!(wfx.nBlockAlign).read_unaligned(),
            ptr::addr_of!(wfx.wBitsPerSample).read_unaligned(),
        )}
    }

    #[test]
    fn make_waveformatex_computes_correct_fields() {
        let fmt = StreamFormat { sample_rate: 48_000, channels: 2 };
        let wfx = make_waveformatex(&fmt);
        let (tag, channels, sample_rate, avg_bps, block_align, bits) = read_wfx_fields(&wfx);
        assert_eq!(tag,          WAVE_FORMAT_IEEE_FLOAT);
        assert_eq!(channels,     2);
        assert_eq!(sample_rate,  48_000);
        assert_eq!(bits,         32);
        assert_eq!(block_align,  8);          // 2 ch × 4 bytes/sample
        assert_eq!(avg_bps,      48_000 * 8);
    }

    #[test]
    fn make_waveformatex_44100_hz() {
        let fmt = StreamFormat { sample_rate: 44_100, channels: 2 };
        let wfx = make_waveformatex(&fmt);
        let (_, _, sample_rate, avg_bps, _, _) = read_wfx_fields(&wfx);
        assert_eq!(sample_rate, 44_100);
        assert_eq!(avg_bps,     44_100 * 8);
    }

    // --- Thread / device tests (require audio hardware) ---

    #[test]
    #[ignore = "requires audio hardware and Windows audio service"]
    fn can_open_default_loopback_device() {
        // This test verifies the full WASAPI setup path compiles and runs
        // without errors on a real Windows machine with audio.
        use std::sync::{Arc, Mutex};
        use eq_core::FilterChain;

        let mut capture = LoopbackCapture::new();
        assert!(!capture.is_running());

        // Start with a flat (passthrough) chain — any sample rate works here
        // because start() updates the chain's sample rate before capturing.
        let chain = Arc::new(Mutex::new(FilterChain::new(48_000.0)));
        let format = capture
            .start(None, chain, |_buf| { /* discard processed audio */ })
            .expect("failed to open loopback device");

        assert!(format.sample_rate > 0);
        assert_eq!(format.channels, 2);
        assert!(capture.is_running());

        // Let the capture thread run for a moment.
        thread::sleep(Duration::from_millis(200));

        capture.stop();
        assert!(!capture.is_running());
    }

    #[test]
    #[ignore = "requires audio hardware and Windows audio service"]
    fn processed_samples_stay_in_valid_range() {
        // Verifies that the clamp logic in FilterChain is respected end-to-end.
        use std::sync::{Arc, Mutex};
        use eq_core::{FilterChain, FilterType, BandConfig};

        let mut bad_band = BandConfig::new(FilterType::Peak, 1000.0);
        bad_band.gain_db = 24.0; // maximum gain — most likely to overflow
        bad_band.q = 10.0;

        let mut chain = FilterChain::new(48_000.0);
        chain.set_bands(&[bad_band]);
        let chain = Arc::new(Mutex::new(chain));

        let all_in_range = Arc::new(AtomicBool::new(true));
        let flag_clone = Arc::clone(&all_in_range);

        let mut capture = LoopbackCapture::new();
        capture
            .start(None, chain, move |buf| {
                for &s in buf {
                    if s < -1.0 || s > 1.0 {
                        flag_clone.store(false, Ordering::Relaxed);
                    }
                }
            })
            .expect("failed to open loopback device");

        thread::sleep(Duration::from_millis(500));
        capture.stop();

        assert!(all_in_range.load(Ordering::Relaxed), "samples exceeded [-1.0, 1.0]");
    }
}
