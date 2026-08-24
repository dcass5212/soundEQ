// =============================================================================
// render.rs — WASAPI render output
//
// WHAT THIS DOES:
// Writes EQ-processed audio to an audio output device via WASAPI.
// This is the "output" side of the routing pipeline; it is the counterpart
// to LoopbackCapture in capture.rs.
//
// HOW IT FITS IN THE SYSTEM:
// The full signal path (once Step 2c and Step 3 are complete):
//
//   App audio → Virtual Cable (system default output)
//       ↓  (WASAPI loopback capture — capture.rs)
//   EQ processing (FilterChain)
//       ↓  (RenderSender — this file)
//   Real speakers/headphones (WASAPI render — this file)
//
// Without a virtual cable installed, the loopback captures from the real
// device and the renderer also targets the real device, which causes double
// playback. The Tauri layer (Step 3) guides the user through virtual cable
// setup and stores the chosen device IDs.
//
// THREADING MODEL:
// Same pattern as capture.rs: a dedicated background thread owns all WASAPI
// objects. The control thread communicates via:
//   - Arc<AtomicBool>        — stop signal
//   - Arc<Mutex<SampleBuffer>> — the audio data queue (filled by capture thread)
//
// UNDERRUN HANDLING:
// If the capture thread falls behind (e.g. CPU spike), the render loop fills
// the remaining render buffer with zeros (silence). This produces a brief
// quiet gap rather than a harsh click or crash. In practice, with 200 ms of
// pre-allocated buffer this should be inaudible.
//
// PACING (EVENT-DRIVEN):
// The render loop does not sleep-and-poll. WASAPI is initialised with
// AUDCLNT_STREAMFLAGS_EVENTCALLBACK and given an event handle; the Windows
// audio engine signals that event exactly once per device period, when
// buffer space becomes available. WaitForSingleObject on the event wakes the
// thread at precisely the right moment — tighter timing than a sleep, no
// dependency on system timer resolution, and zero CPU while idle.
//
// LATENCY CONTROL (DRIFT TRIM):
// The capture device and the render device run on independent clocks. Even
// at the same nominal sample rate they drift by a few parts per million,
// so over minutes the ring backlog slowly grows (capture faster) or drains
// (render faster). Left alone, a growing backlog means audio latency creeps
// up toward the full ring capacity before samples are dropped mid-stream.
// The render loop therefore trims the backlog back to a small target
// whenever it exceeds a threshold, and masks the resulting waveform
// discontinuity with a short fade-in. Result: latency stays bounded to tens
// of milliseconds and the rare trim is inaudible instead of a click.
// =============================================================================

use std::collections::VecDeque;
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
    IAudioClient, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, WAVEFORMATEX, eConsole, eRender,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};
use windows::core::PCWSTR;

use crate::capture::{StreamFormat, WAVE_FORMAT_EXTENSIBLE, WAVE_FORMAT_IEEE_FLOAT};
use crate::error::AudioError;
use crate::rt_priority::RealTimeBoost;

// ---------------------------------------------------------------------------
// SampleBuffer — the thread-safe queue between capture and render
//
// The capture callback writes processed samples here; the render thread reads
// them out at the rate demanded by the hardware.
//
// We use VecDeque (pre-allocated ring buffer) rather than a channel of Vecs
// so that steady-state operation is allocation-free. Both ends hold an
// Arc<Mutex<SampleBuffer>>, and the lock is held for microseconds at a time.
// ---------------------------------------------------------------------------

/// Ring capacity in milliseconds of stereo audio. This is the hard ceiling on
/// backlog; the drift trim (see run_render_loop) keeps the steady-state
/// backlog far below it, so this is purely emergency headroom.
const RING_CAPACITY_MS: usize = 200;

/// Moves the trim-masking ramp one frame toward `target`, in integer positions.
///
/// The ramp (see `run_render_loop`) is stored as an integer position in
/// `0..=fade_total_frames` and converted to a gain of `pos / total` only where
/// it is used. Accumulating an `f32` multiplier instead would drift: 240 steps
/// of `1.0 / 240.0` lands on 7.9e-7 rather than 0.0, so the `pos == 0` test that
/// releases the pending trim would never fire and the backlog would grow
/// forever. Integer positions make both endpoints exact by construction.
#[inline]
fn step_ramp_pos(pos: usize, target: usize) -> usize {
    match pos.cmp(&target) {
        std::cmp::Ordering::Less => pos + 1,
        std::cmp::Ordering::Greater => pos - 1,
        std::cmp::Ordering::Equal => pos,
    }
}

// ---------------------------------------------------------------------------
// TrimStats — drift-trim telemetry
//
// The render loop used to report each trim with `eprintln!`. That formats a
// String (a heap allocation) and takes the process-wide stderr lock, both from
// inside the render period — against this project's rule that the audio thread
// never allocates, and a real hazard: if stderr is a slow or blocked console or
// pipe, the write stalls the render thread past its device period and causes
// exactly the underrun the message was reporting.
//
// Instead the render thread does a pair of relaxed atomic adds — wait-free, no
// allocation, no lock, no syscall — and any control thread reads the totals at
// its leisure. A single human-readable summary is printed once when the render
// loop exits, which is off the hot path entirely.
// ---------------------------------------------------------------------------

/// Cumulative counters describing drift trims performed by the render loop.
///
/// Shared between the render thread (writer) and any control thread (reader)
/// via `WasapiRenderer::trim_stats()`. `Relaxed` ordering is sufficient: these
/// are independent counters used for diagnostics, with no happens-before
/// relationship to any other state.
#[derive(Default)]
pub struct TrimStats {
    count: AtomicU32,
    total_ms: AtomicU32,
}

impl TrimStats {
    /// Records one trim. Called from the render thread on the hot path, so this
    /// must stay allocation-free and lock-free — two atomic adds, nothing else.
    #[inline]
    fn record(&self, dropped_ms: u32) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.total_ms.fetch_add(dropped_ms, Ordering::Relaxed);
    }

    /// Trims so far, as `(count, total milliseconds of audio discarded)`.
    ///
    /// A steadily climbing count is normal — clock drift between the capture and
    /// render devices makes a trim inevitable every few minutes. A count that
    /// climbs *fast* means the capture thread is being starved and dumping
    /// bursts into the ring, which is worth investigating.
    pub fn snapshot(&self) -> (u32, u32) {
        (
            self.count.load(Ordering::Relaxed),
            self.total_ms.load(Ordering::Relaxed),
        )
    }
}

/// Pre-allocated sample queue connecting the capture thread to the render thread.
pub struct SampleBuffer {
    inner: VecDeque<f32>,
    capacity: usize,
}

impl SampleBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Appends samples to the queue. If adding `samples` would exceed capacity,
    /// the oldest samples are silently dropped. This is preferable to blocking
    /// the capture thread or panicking.
    fn write(&mut self, samples: &[f32]) {
        // If the incoming slice alone exceeds the whole capacity, only its
        // newest samples can survive anyway — clamp before the drain below so
        // the excess computation can never exceed what is actually queued.
        let samples = if samples.len() > self.capacity {
            &samples[samples.len() - self.capacity..]
        } else {
            samples
        };
        if self.inner.len() + samples.len() > self.capacity {
            let excess = (self.inner.len() + samples.len()) - self.capacity;
            self.inner.drain(..excess);
        }
        self.inner.extend(samples.iter().copied());
    }

    /// Reads up to `out.len()` samples into `out`. Returns how many were read.
    /// Uses bulk copy (via VecDeque slices) rather than per-element pop for speed.
    fn read_into(&mut self, out: &mut [f32]) -> usize {
        let n = out.len().min(self.inner.len());
        if n == 0 {
            return 0;
        }
        // VecDeque may wrap around its ring buffer — it exposes two contiguous slices.
        let (front, back) = self.inner.as_slices();
        if front.len() >= n {
            out[..n].copy_from_slice(&front[..n]);
        } else {
            let from_front = front.len();
            out[..from_front].copy_from_slice(front);
            let from_back = n - from_front;
            out[from_front..n].copy_from_slice(&back[..from_back]);
        }
        self.inner.drain(..n);
        n
    }

    /// How many samples are currently queued.
    pub fn available(&self) -> usize {
        self.inner.len()
    }

    /// If more than `max_len` samples are queued, drops the oldest so that
    /// only `target_len` remain. Returns the number of samples dropped
    /// (0 when the backlog is within bounds — the common case).
    ///
    /// Used by the render loop to bound pipeline latency: clock drift between
    /// the capture and render devices makes the backlog grow slowly, and
    /// without trimming, audio latency would creep up to the full ring
    /// capacity. Dropping down to `target_len` (rather than `max_len`) means
    /// trims are infrequent — the backlog must re-grow the whole gap before
    /// the next one.
    fn trim_excess(&mut self, max_len: usize, target_len: usize) -> usize {
        if self.inner.len() <= max_len {
            return 0;
        }
        let excess = self.inner.len() - target_len.min(max_len);
        self.inner.drain(..excess);
        excess
    }

    /// Re-sizes the buffer to a new capacity, dropping the oldest samples if
    /// the current contents exceed it.
    ///
    /// Called once at render startup, when the device's real sample rate
    /// becomes known — the buffer is first allocated assuming 48 kHz, and a
    /// 96/192 kHz device needs proportionally more samples to hold the same
    /// RING_CAPACITY_MS of audio. This allocates, so it must never be called
    /// from the steady-state audio path.
    fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        self.trim_excess(capacity, capacity);
        self.inner.reserve(capacity.saturating_sub(self.inner.len()));
    }
}

// ---------------------------------------------------------------------------
// RenderSender — the write-only handle given to the capture callback
//
// The capture thread calls `write(buf)` after each EQ-processed packet.
// This type is Clone so multiple producers are possible (though in practice
// there is only one — the capture callback).
// ---------------------------------------------------------------------------

/// Write handle for pushing processed audio into the render queue.
/// Obtained from `WasapiRenderer::sender()` and passed to the capture callback.
#[derive(Clone)]
pub struct RenderSender(Arc<Mutex<SampleBuffer>>);

impl RenderSender {
    /// Writes samples into the queue. Returns immediately; never blocks.
    /// Excess samples are dropped if the queue is full.
    pub fn write(&self, samples: &[f32]) {
        if let Ok(mut buf) = self.0.lock() {
            buf.write(samples);
        }
    }
}

// ---------------------------------------------------------------------------
// WasapiRenderer — the public API
// ---------------------------------------------------------------------------

/// Renders audio to an output device via WASAPI on a dedicated background thread.
///
/// Lifecycle:
/// 1. `WasapiRenderer::new()` — create the handle.
/// 2. `sender()` — obtain a `RenderSender` to pass to the capture callback.
/// 3. `start(device_id)` — begin rendering. Returns the render device's `StreamFormat`.
/// 4. `stop()` — signal the render thread to exit and wait for it.
pub struct WasapiRenderer {
    stop_flag: Arc<AtomicBool>,
    thread_handle: Option<JoinHandle<()>>,
    ring: Arc<Mutex<SampleBuffer>>,
    /// Current volume scalar as f32 bits. Written by VolumeMonitor every 50 ms;
    /// read by the render loop every buffer. Default 1.0 = full volume.
    volume: Arc<AtomicU32>,
    /// User-controlled output gain as f32 bits. Multiplied with `volume` in
    /// the render loop. Default 1.0. Allows compensating for VB-Cable's lower
    /// inherent signal level relative to direct device output.
    output_gain: Arc<AtomicU32>,
    /// Drift-trim telemetry. Written by the render thread with wait-free atomic
    /// adds instead of printing from the hot path; read by control threads.
    trim_stats: Arc<TrimStats>,
}

impl WasapiRenderer {
    /// Creates a new (idle) renderer with a pre-allocated 200 ms sample buffer.
    ///
    /// 200 ms at 48 kHz stereo = 48 000 × 2 × 0.2 = 19 200 f32 samples.
    /// This absorbs timing jitter between the capture and render threads.
    /// The render thread re-sizes it via set_capacity() once the device's
    /// actual sample rate is known (a 192 kHz device needs 4× the samples).
    pub fn new() -> Self {
        let ring_capacity = 48_000 * 2 * RING_CAPACITY_MS / 1000; // samples (not bytes)
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
            ring: Arc::new(Mutex::new(SampleBuffer::new(ring_capacity))),
            volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            output_gain: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            trim_stats: Arc::new(TrimStats::default()),
        }
    }

    /// Returns the volume Arc so `VolumeMonitor` can write into it.
    /// Call this after `new()` and before `start()`.
    pub fn volume_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.volume)
    }

    /// Returns the output-gain Arc so callers can update the gain live.
    /// Write a f32 value (clamped to [0.0, 4.0] by the render loop) as bits.
    pub fn gain_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.output_gain)
    }

    /// Returns the drift-trim counters so a control thread can observe how often
    /// the render loop has had to discard backlog. See `TrimStats::snapshot`.
    pub fn trim_stats(&self) -> Arc<TrimStats> {
        Arc::clone(&self.trim_stats)
    }

    /// Returns a `RenderSender` that can be used to push audio into the render queue.
    /// Call this BEFORE `start()` so the capture callback has the sender ready.
    pub fn sender(&self) -> RenderSender {
        RenderSender(Arc::clone(&self.ring))
    }

    /// Starts the render thread.
    ///
    /// `device_id` — an audio endpoint ID string returned by `list_render_devices()`.
    /// Pass `None` to use the current system-default output device.
    ///
    /// Returns the `StreamFormat` of the opened device so the caller can verify
    /// it matches the capture device's format.
    pub fn start(&mut self, device_id: Option<String>) -> Result<StreamFormat, AudioError> {
        if self.thread_handle.is_some() {
            return Err(AudioError::AlreadyRunning);
        }
        self.stop_flag.store(false, Ordering::Relaxed);

        let (format_tx, format_rx) = mpsc::sync_channel::<Result<StreamFormat, AudioError>>(1);
        let stop_flag = Arc::clone(&self.stop_flag);
        let ring = Arc::clone(&self.ring);

        let volume = Arc::clone(&self.volume);
        let output_gain = Arc::clone(&self.output_gain);
        let trim_stats = Arc::clone(&self.trim_stats);
        let handle = thread::Builder::new()
            .name("wasapi-render".to_string())
            .spawn(move || {
                render_thread_main(stop_flag, ring, device_id, format_tx, volume, output_gain, trim_stats);
            })?;

        self.thread_handle = Some(handle);
        format_rx.recv().map_err(|_| AudioError::ThreadSetupFailed)?
    }

    /// Signals the render thread to stop and waits for it to exit.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }

    /// Returns true if the render thread is currently active.
    pub fn is_running(&self) -> bool {
        self.thread_handle.is_some()
    }

    /// Returns true if the render thread exited on its own due to a device
    /// error — i.e., the thread finished but the stop flag was never set.
    ///
    /// Mirrors `LoopbackCapture::has_crashed()`. See that doc for details.
    pub fn has_crashed(&self) -> bool {
        self.thread_handle
            .as_ref()
            .is_some_and(|h| h.is_finished())
            && !self.stop_flag.load(Ordering::Relaxed)
    }
}

impl Default for WasapiRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WasapiRenderer {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Render thread
// ---------------------------------------------------------------------------

fn render_thread_main(
    stop_flag: Arc<AtomicBool>,
    ring: Arc<Mutex<SampleBuffer>>,
    device_id: Option<String>,
    format_tx: SyncSender<Result<StreamFormat, AudioError>>,
    volume: Arc<AtomicU32>,
    output_gain: Arc<AtomicU32>,
    trim_stats: Arc<TrimStats>,
) {
    // COM must be initialised per-thread.
    // _com must be declared FIRST so it is dropped LAST (after all COM objects).
    let _com = match ComGuard::init() {
        Ok(g) => g,
        Err(e) => { let _ = format_tx.send(Err(e)); return; }
    };

    // Boost this thread into the MMCSS "Pro Audio" scheduling class and raise
    // the system timer to 1 ms resolution, so the thread keeps getting
    // scheduled promptly even when other programs saturate the CPU.
    // The guard reverts both boosts when the thread exits.
    let _boost = RealTimeBoost::enable("render");

    // Create the event WASAPI will signal once per device period.
    // Auto-reset (bManualReset = false): each WaitForSingleObject consumes
    // one signal, which is exactly the one-wake-per-period semantics we want.
    //
    // unsafe: CreateEventW with default security, no name — no preconditions.
    let event = match unsafe { CreateEventW(None, false, false, PCWSTR::null()) } {
        Ok(h) => h,
        Err(e) => { let _ = format_tx.send(Err(AudioError::Windows(e))); return; }
    };
    // RAII: the OS event handle is closed when the thread exits, whatever path
    // it takes. Declared after _com/_boost so it drops before them — order is
    // irrelevant here, but the handle must be wrapped before any early return.
    let _event_guard = EventHandleGuard(event);

    let (audio_client, render_client, format) =
        match open_render_device(device_id.as_deref(), event) {
            Ok(v) => v,
            Err(e) => { let _ = format_tx.send(Err(e)); return; }
        };

    // The ring was pre-allocated assuming 48 kHz. Now that the device's real
    // rate is known, re-size it so it holds RING_CAPACITY_MS of audio at the
    // actual rate (the ring always stores stereo, hence × 2). This is the one
    // permitted allocation — it happens once, before Start(), never in the
    // steady-state loop.
    //
    // This MUST happen before `format_tx.send` below. That send is what unblocks
    // `WasapiRenderer::start()`, which lets `AudioEngine::start()` go on to spawn
    // the capture thread — and that thread's callback writes straight into this
    // ring. Publishing the format first would leave a window where early capture
    // writes are clipped to the stale 48 kHz capacity (four times too small on a
    // 192 kHz device), and where this `reserve` allocates while holding the very
    // lock the capture thread is contending for.
    {
        let mut buf = ring.lock().unwrap();
        buf.set_capacity(format.sample_rate as usize * 2 * RING_CAPACITY_MS / 1000);
    }

    let _ = format_tx.send(Ok(format));

    // unsafe: Start() is valid after a successful Initialize() (done in open_render_device).
    if let Err(e) = unsafe { audio_client.Start() } {
        eprintln!("[eq-audio] IAudioClient::Start (render) failed: {e}");
        return;
    }

    run_render_loop(
        &stop_flag,
        &ring,
        &audio_client,
        &render_client,
        &format,
        event,
        &volume,
        &output_gain,
        &trim_stats,
    );

    let _ = unsafe { audio_client.Stop() };
}

#[allow(clippy::too_many_arguments)] // internal hot-path fn; bundling into a struct adds no value
fn run_render_loop(
    stop_flag: &AtomicBool,
    ring: &Mutex<SampleBuffer>,
    audio_client: &IAudioClient,
    render_client: &IAudioRenderClient,
    format: &StreamFormat,
    event: HANDLE,
    volume: &AtomicU32,
    output_gain: &AtomicU32,
    trim_stats: &TrimStats,
) {
    // GetBufferSize returns the total WASAPI buffer capacity in frames, which
    // is fixed after Initialize() and never changes. Query it once here so the
    // hot loop only calls the cheaper GetCurrentPadding() each iteration.
    //
    // unsafe: audio_client is valid — guaranteed by render_thread_main calling
    // IAudioClient::Start() successfully before entering this function.
    let buf_size = match unsafe { audio_client.GetBufferSize() } {
        Ok(n) => n,
        Err(e) => { eprintln!("[eq-audio] GetBufferSize: {e}"); return; }
    };

    // --- Drift-trim thresholds (all in ring samples; ring is always stereo) ---
    //
    // The backlog normally hovers near zero. If the capture clock runs even
    // slightly faster than the render clock, it grows without bound — each
    // extra queued sample is a sample of added latency.
    //
    // Critically, a backlog spike never drains on its own. Capture and render
    // run at the same nominal rate, so whatever burst the capture thread dumps
    // into the ring after a stall just sits there as permanent added latency
    // until something trims it. MAX is therefore a deliberate latency budget,
    // not a jitter guard: we accept up to 60 ms of pipeline backlog, and past
    // that we choose to drop audio rather than let the delay keep creeping up.
    // TARGET (20 ms) is where a trim lands; the 40 ms gap means ordinary clock
    // drift (~100 ppm) triggers a trim only every several minutes, and TARGET
    // stays above zero so a cushion remains against ordinary jitter.
    let samples_per_ms = ((format.sample_rate as usize * 2) / 1000).max(1);
    let max_backlog    = 60 * samples_per_ms;
    let target_backlog = 20 * samples_per_ms;

    // --- Trim masking ramp ---
    //
    // Dropping samples mid-stream creates a waveform discontinuity — a click.
    // A fade-IN after the cut cannot remove it: the previous buffer is already
    // with the driver, ending at some arbitrary sample value V, so starting the
    // next buffer from zero merely swaps one step (V -> W) for another (V -> 0).
    // The cut has to happen when the output is ALREADY at silence.
    //
    // So a trim runs as a three-phase state machine driven by a continuous
    // per-frame gain ramp, held as an integer position in 0..=fade_total_frames
    // (gain = pos / fade_total_frames):
    //   1. backlog over budget -> ramp_target_pos = 0, the output fades out
    //   2. pos reaches 0       -> splice the ring (0.0 joined to 0.0: no click)
    //   3. ramp_target_pos = fade_total_frames -> the output fades back in
    //
    // Because the position moves by at most one frame per sample frame and
    // carries across buffer boundaries, the rendered waveform stays continuous
    // throughout. The audible result is a ~10 ms dip rather than a click.
    let fade_total_frames = (format.sample_rate as usize * 5 / 1000).max(1);
    let mut ramp_pos = fade_total_frames;        // current position (full level)
    let mut ramp_target_pos = fade_total_frames; // where the ramp is heading
    let mut awaiting_trim = false;

    while !stop_flag.load(Ordering::Relaxed) {
        // Block until the audio engine signals that buffer space is available
        // (once per device period). The 200 ms timeout is a safety net: if the
        // device stalls (e.g. invalidated mid-stream), we still wake up to
        // check the stop flag and let GetCurrentPadding surface the error.
        //
        // unsafe: event is a valid handle owned by this thread's EventHandleGuard.
        let wait = unsafe { WaitForSingleObject(event, 200) };
        if wait != WAIT_OBJECT_0 && wait != WAIT_TIMEOUT {
            eprintln!("[eq-audio] render event wait failed: {:?}", wait);
            break;
        }

        // unsafe: GetCurrentPadding is valid on a started client.
        let padding = match unsafe { audio_client.GetCurrentPadding() } {
            Ok(n) => n,
            Err(e) => { eprintln!("[eq-audio] GetCurrentPadding: {e}"); break; }
        };

        let available_frames = buf_size.saturating_sub(padding);

        // Phase 2 of the trim state machine: the output has finished fading
        // out, so the ring can be spliced now — at silence, where the cut joins
        // 0.0 to 0.0 and produces no discontinuity at all.
        //
        // This sits ABOVE the `available_frames == 0` early-continue on
        // purpose: on a device whose buffer happens to be full at wake time,
        // skipping the trim would let the backlog grow unbounded while the trim
        // never got a chance to run.
        if awaiting_trim && ramp_pos == 0 {
            let dropped = {
                let mut buf = ring.lock().unwrap();
                buf.trim_excess(max_backlog, target_backlog)
            };
            awaiting_trim = false;
            ramp_target_pos = fade_total_frames; // phase 3: fade back in
            if dropped > 0 {
                // Two relaxed atomic adds. Deliberately NOT an eprintln!: that
                // would allocate a String and take the process-wide stderr lock
                // inside the render period. A summary is printed once when the
                // loop exits, and TrimStats::snapshot exposes the live totals.
                trim_stats.record((dropped / samples_per_ms) as u32);
            }
        }

        if available_frames == 0 {
            // Nothing to render this period, so the post-drain backlog check
            // further down never runs. Do the over-budget check here instead:
            // with nothing draining, a high reading is unambiguously real.
            if !awaiting_trim && ring.lock().unwrap().available() > max_backlog {
                ramp_target_pos = 0;
                awaiting_trim = true;
            }
            continue;
        }

        // unsafe: GetBuffer returns a pointer to available_frames * channels * 4 bytes
        // of writable render buffer memory. Valid until ReleaseBuffer is called below.
        let data_ptr = match unsafe { render_client.GetBuffer(available_frames) } {
            Ok(p) => p,
            Err(e) => { eprintln!("[eq-audio] render GetBuffer: {e}"); break; }
        };

        let n_samples = available_frames as usize * format.channels as usize;

        // unsafe: data_ptr is valid for n_samples f32 values until ReleaseBuffer.
        let render_slice =
            unsafe { slice::from_raw_parts_mut(data_ptr as *mut f32, n_samples) };

        // Drain the ring (which always holds stereo samples) into the render buffer.
        if format.channels == 2 {
            // Stereo output: read directly into the render slice.
            let samples_read = {
                let mut buf = ring.lock().unwrap();
                buf.read_into(render_slice)
            };
            if samples_read < n_samples {
                render_slice[samples_read..].fill(0.0);
            }
        } else {
            // Multi-channel output (e.g. 5.1, 7.1): read stereo from the ring into
            // the front of the render slice, then expand in-place back-to-front so
            // each frame gets L in ch0, R in ch1, and silence in the extra channels.
            // Working backwards avoids overwriting stereo data before reading it.
            // No heap allocation — render_slice is the WASAPI-managed buffer.
            let ch = format.channels as usize;
            let stereo_count = available_frames as usize * 2;

            let stereo_read = {
                let mut buf = ring.lock().unwrap();
                buf.read_into(&mut render_slice[..stereo_count])
            };
            if stereo_read < stereo_count {
                render_slice[stereo_read..stereo_count].fill(0.0);
            }

            for frame_idx in (0..available_frames as usize).rev() {
                let l = render_slice[frame_idx * 2];
                let r = render_slice[frame_idx * 2 + 1];
                let dst = frame_idx * ch;
                render_slice[dst] = l;
                render_slice[dst + 1] = r;
                render_slice[dst + 2..dst + ch].fill(0.0);
            }
        }

        // Apply volume scaling: VolumeMonitor scalar (mirrors the Windows slider)
        // multiplied by the user-controlled output gain (compensates for VB-Cable's
        // inherent lower signal level vs. direct device output).
        //
        // Both are stored as f32 bits in atomics so the render thread reads them
        // without locking. We clamp after multiplying to prevent digital clipping
        // when gain > 1.0 boosts a loud signal above 0 dBFS.
        let vol  = f32::from_bits(volume.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let gain = f32::from_bits(output_gain.load(Ordering::Relaxed)).clamp(0.0, 4.0);
        let combined = vol * gain;
        // The trim ramp (see the state machine with the constants above) is
        // folded into the same multiply. With no trim in flight the ramp sits
        // at 1.0 and this collapses to the plain scalar path, so the common
        // case costs exactly what it did before.
        if ramp_pos != fade_total_frames || ramp_target_pos != fade_total_frames {
            let ch = format.channels as usize;
            let inv_total = 1.0 / fade_total_frames as f32;
            for frame in render_slice.chunks_exact_mut(ch) {
                // Advance the ramp one frame toward its target, clamping so the
                // final step cannot overshoot. Every channel of a frame shares
                // the same gain, so the stereo image is unaffected.
                ramp_pos = step_ramp_pos(ramp_pos, ramp_target_pos);
                let g = combined * (ramp_pos as f32 * inv_total);
                for s in frame.iter_mut() {
                    *s = (*s * g).clamp(-1.0, 1.0);
                }
            }
        } else if (combined - 1.0).abs() > 0.0001 {
            for s in render_slice.iter_mut() {
                *s = (*s * combined).clamp(-1.0, 1.0);
            }
        }

        // Phase 1 of the trim state machine: is the backlog over budget?
        //
        // Measured AFTER the drain above, never before. A pre-drain reading is
        // misleading on the first wake following a render-side stall: the device
        // buffer has emptied, `available_frames` is unusually large, and this
        // very iteration is about to consume the apparent excess. Only what
        // survives the drain is genuinely accumulated latency.
        if !awaiting_trim && ramp_target_pos != 0 {
            let backlog = ring.lock().unwrap().available();
            if backlog > max_backlog {
                ramp_target_pos = 0; // fade out; the cut follows at silence
                awaiting_trim = true;
            }
        }

        // unsafe: available_frames matches the value passed to GetBuffer.
        // The 0 flag means "no discontinuity / not silence" — we handle silence by
        // filling with 0.0 rather than using AUDCLNT_BUFFERFLAGS_SILENT, because the
        // SILENT flag skips the data entirely (the driver generates silence internally)
        // but we want our filled silence to propagate correctly in all driver versions.
        if let Err(e) = unsafe { render_client.ReleaseBuffer(available_frames, 0) } {
            eprintln!("[eq-audio] ReleaseBuffer (render): {e}");
            break;
        }
    }

    // Off the hot path: the loop has exited and this thread is winding down, so
    // formatting and stderr are free here. One line beats one per trim, and it
    // is the only place in this function permitted to print during normal
    // operation. (The `eprintln!`s inside the loop above are all on terminal
    // error paths that `break` immediately — they fire once, on the way out.)
    let (trim_count, trim_ms) = trim_stats.snapshot();
    if trim_count > 0 {
        eprintln!(
            "[eq-audio] render loop exiting: {trim_count} latency trim(s), \
             {trim_ms} ms of drift backlog discarded in total"
        );
    }
}

// ---------------------------------------------------------------------------
// WASAPI render device setup
// ---------------------------------------------------------------------------

/// Opens the requested render endpoint (or the system default if `device_id` is None),
/// initialises it for event-driven shared-mode output, and returns the client,
/// render client, and stream format.
///
/// `event` — an auto-reset event handle (owned by the caller) that WASAPI
/// will signal once per device period when buffer space becomes available.
fn open_render_device(
    device_id: Option<&str>,
    event: HANDLE,
) -> Result<(IAudioClient, IAudioRenderClient, StreamFormat), AudioError> {
    // unsafe: CoCreateInstance — same rationale as in capture.rs.
    let enumerator: IMMDeviceEnumerator = unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?
    };

    // Open the requested device or fall back to the system default.
    let device = if let Some(id) = device_id {
        // Convert the UTF-8 device ID to a null-terminated UTF-16 string.
        let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
        // unsafe: PCWSTR borrows wide; wide is alive for this scope.
        unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr()))? }
    } else {
        // unsafe: enumerator is valid.
        unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? }
    };

    // unsafe: device is valid.
    let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };

    // Query and parse the mix format — same pattern as in capture.rs.
    let fmt_ptr: *mut WAVEFORMATEX = unsafe { audio_client.GetMixFormat()? };
    let parse_result = unsafe { parse_render_format(fmt_ptr) };

    // Initialise using the ORIGINAL format pointer from GetMixFormat.
    // This correctly handles multi-channel devices (5.1, 7.1) without needing
    // to reconstruct WAVEFORMATEXTENSIBLE with the correct channel mask.
    //
    // AUDCLNT_STREAMFLAGS_EVENTCALLBACK: instead of us guessing when to
    // refill (sleep-and-poll), the audio engine signals our event exactly
    // when buffer space becomes available each period.
    let init_result = unsafe {
        audio_client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            0,    // buffer duration: let the engine choose its default
            0,    // periodicity: 0 for shared mode
            fmt_ptr as *const WAVEFORMATEX,
            None,
        )
    };

    // Free the COM-allocated format struct.
    unsafe { CoTaskMemFree(Some(fmt_ptr.cast())) };

    let stream_format = parse_result?;
    init_result?;

    // Register the caller's event with the audio client. Required when
    // EVENTCALLBACK is set; must happen after Initialize and before Start.
    // unsafe: audio_client is initialised and event is a valid handle.
    unsafe { audio_client.SetEventHandle(event)? };

    // unsafe: audio_client is successfully initialised.
    let render_client: IAudioRenderClient = unsafe { audio_client.GetService()? };

    Ok((audio_client, render_client, stream_format))
}

// ---------------------------------------------------------------------------
// OS event handle guard
// ---------------------------------------------------------------------------

/// RAII wrapper that closes a Win32 event handle on drop.
///
/// The render thread creates one event for its lifetime; this guard
/// guarantees the handle is released on every exit path (setup failure,
/// device error, normal stop).
struct EventHandleGuard(HANDLE);

impl Drop for EventHandleGuard {
    fn drop(&mut self) {
        // unsafe: self.0 came from a successful CreateEventW and is closed
        // exactly once (the guard is never cloned).
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Validates that the render device's mix format is float32 stereo.
///
/// # Safety
/// `ptr` must be a valid non-null WAVEFORMATEX (or WAVEFORMATEXTENSIBLE) pointer
/// allocated by Windows (from GetMixFormat).
unsafe fn parse_render_format(ptr: *const WAVEFORMATEX) -> Result<StreamFormat, AudioError> {
    let fmt = &*ptr;
    let channels = fmt.nChannels;
    let bits     = fmt.wBitsPerSample;
    let tag      = fmt.wFormatTag;
    let rate     = fmt.nSamplesPerSec;

    let is_float32 = match tag {
        WAVE_FORMAT_IEEE_FLOAT => bits == 32,
        WAVE_FORMAT_EXTENSIBLE => {
            let ext = ptr::read_unaligned(ptr as *const windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE);
            let sub_format = ptr::addr_of!(ext.SubFormat).read_unaligned();
            sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT && bits == 32
        }
        _ => false,
    };

    if !is_float32 {
        return Err(AudioError::UnsupportedFormat(format!(
            "render device: tag=0x{tag:04X}, bits={bits}"
        )));
    }

    Ok(StreamFormat { sample_rate: rate, channels })
}

// ---------------------------------------------------------------------------
// COM initialisation guard (same pattern as in capture.rs)
// ---------------------------------------------------------------------------

struct ComGuard;

impl ComGuard {
    fn init() -> Result<Self, AudioError> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        hr.ok()?;
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn new_renderer_is_not_running() {
        let r = WasapiRenderer::new();
        assert!(!r.is_running());
    }

    #[test]
    fn sample_buffer_write_and_read() {
        let mut buf = SampleBuffer::new(16);
        buf.write(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(buf.available(), 4);

        let mut out = [0.0f32; 4];
        let n = buf.read_into(&mut out);
        assert_eq!(n, 4);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(buf.available(), 0);
    }

    #[test]
    fn sample_buffer_partial_read() {
        let mut buf = SampleBuffer::new(16);
        buf.write(&[1.0, 2.0, 3.0, 4.0]);

        let mut out = [0.0f32; 2];
        let n = buf.read_into(&mut out);
        assert_eq!(n, 2);
        assert_eq!(out, [1.0, 2.0]);
        assert_eq!(buf.available(), 2); // 3.0 and 4.0 remain
    }

    #[test]
    fn sample_buffer_drops_oldest_on_overflow() {
        let mut buf = SampleBuffer::new(4);
        buf.write(&[1.0, 2.0, 3.0, 4.0]); // fills exactly
        buf.write(&[5.0, 6.0]);             // overflow: drops 1.0 and 2.0

        let mut out = [0.0f32; 4];
        buf.read_into(&mut out);
        assert_eq!(out, [3.0, 4.0, 5.0, 6.0]);
    }

    // --- TrimStats ---

    #[test]
    fn trim_stats_accumulate_count_and_total() {
        let stats = TrimStats::default();
        assert_eq!(stats.snapshot(), (0, 0), "a fresh renderer has recorded nothing");

        stats.record(40);
        stats.record(37);
        assert_eq!(
            stats.snapshot(),
            (2, 77),
            "each trim must bump the count and add its milliseconds to the total"
        );
    }

    #[test]
    fn trim_excess_is_noop_within_bounds() {
        let mut buf = SampleBuffer::new(16);
        buf.write(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(buf.trim_excess(8, 2), 0); // 4 ≤ 8 → untouched
        assert_eq!(buf.available(), 4);
    }

    #[test]
    fn trim_excess_drops_oldest_down_to_target() {
        let mut buf = SampleBuffer::new(16);
        buf.write(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let dropped = buf.trim_excess(4, 2); // 6 > 4 → keep newest 2
        assert_eq!(dropped, 4);

        let mut out = [0.0f32; 2];
        buf.read_into(&mut out);
        assert_eq!(out, [5.0, 6.0]);
        assert_eq!(buf.available(), 0);
    }

    // --- Trim-masking ramp (step_ramp_pos) ---
    //
    // The ramp is what makes a drift trim inaudible: the render loop must be
    // able to reach *exactly* position 0 (so the ring is spliced at silence)
    // and then return to exactly full level. Integer positions are what make
    // both endpoints exact — an accumulated f32 ramp lands on 7.9e-7 instead
    // of 0.0 after 240 steps, and the pending trim would never be released.

    #[test]
    fn step_ramp_pos_fades_out_to_exactly_zero() {
        let total = 240; // 5 ms at 48 kHz
        let mut pos = total;
        for _ in 0..total {
            pos = step_ramp_pos(pos, 0);
        }
        assert_eq!(pos, 0, "must land exactly on 0 so the pending trim can fire");
    }

    #[test]
    fn step_ramp_pos_fades_in_to_exactly_full_level() {
        let total = 240;
        let mut pos = 0;
        for _ in 0..total {
            pos = step_ramp_pos(pos, total);
        }
        assert_eq!(pos, total, "must settle exactly at full level (the cheap path)");
    }

    #[test]
    fn step_ramp_pos_is_continuous_and_idempotent_at_target() {
        // Never moves more than one position per frame — this is what keeps the
        // rendered waveform continuous across buffer boundaries.
        assert_eq!(step_ramp_pos(240, 0), 239);
        assert_eq!(step_ramp_pos(0, 240), 1);
        // Already at target: a fixed point, so the cheap path stays cheap.
        assert_eq!(step_ramp_pos(240, 240), 240);
        assert_eq!(step_ramp_pos(0, 0), 0);
        // Reversing mid-fade is safe: no underflow at the bottom of the range.
        assert_eq!(step_ramp_pos(1, 0), 0);
        assert_eq!(step_ramp_pos(0, 0), 0);
    }

    #[test]
    fn step_ramp_pos_gain_endpoints_are_exact() {
        // The loop converts position to gain as `pos / total`; both ends must be
        // exact so a faded-out buffer is true silence and a restored one is unity.
        let total = 240usize;
        let inv = 1.0f32 / total as f32;
        assert_eq!(0usize as f32 * inv, 0.0);
        assert_eq!(total as f32 * inv, 1.0);
    }

    #[test]
    fn set_capacity_shrink_drops_oldest() {
        let mut buf = SampleBuffer::new(16);
        buf.write(&[1.0, 2.0, 3.0, 4.0]);
        buf.set_capacity(2);
        assert_eq!(buf.available(), 2);

        let mut out = [0.0f32; 2];
        buf.read_into(&mut out);
        assert_eq!(out, [3.0, 4.0]); // newest survive

        // Writes now overflow at the new, smaller capacity.
        buf.write(&[1.0, 2.0, 3.0]);
        assert_eq!(buf.available(), 2);
    }

    #[test]
    fn set_capacity_grow_allows_more_samples() {
        let mut buf = SampleBuffer::new(2);
        buf.set_capacity(6);
        buf.write(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(buf.available(), 6); // no longer clipped at 2
    }

    #[test]
    fn render_sender_writes_through_arc() {
        let ring = Arc::new(Mutex::new(SampleBuffer::new(16)));
        let sender = RenderSender(Arc::clone(&ring));

        sender.write(&[0.5, -0.5]);

        let mut out = [0.0f32; 2];
        ring.lock().unwrap().read_into(&mut out);
        assert_eq!(out, [0.5, -0.5]);
    }

    #[test]
    #[ignore = "requires audio hardware and Windows audio service"]
    fn can_start_render_on_default_device() {
        let mut renderer = WasapiRenderer::new();
        let _sender = renderer.sender();

        let format = renderer.start(None).expect("render start failed");
        assert!(format.sample_rate > 0);
        assert_eq!(format.channels, 2);
        assert!(renderer.is_running());

        thread::sleep(Duration::from_millis(200));
        renderer.stop();
        assert!(!renderer.is_running());
    }
}
