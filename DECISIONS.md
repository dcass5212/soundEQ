# Architectural Decision Log

## ADR-001: Tauri over Electron
Electron bundles a full Chromium instance (~150MB+, high RAM usage).
Tauri uses the OS native webview (WebView2 on Windows), resulting in a
~5MB binary. For a background audio app, minimal resource usage is a
core requirement.

## ADR-002: Rust for DSP
Real-time audio processing requires zero GC pauses. Rust's ownership
model enforces memory safety at compile time with no runtime overhead.
A GC pause in the audio thread causes audible glitches.

## ADR-003: Parametric over graphic EQ
Parametric gives users precise control (frequency, gain, Q per band).
Graphic EQ has fixed frequency points which limits usefulness for
problem-solving (e.g. cutting a specific resonance at an exact frequency).

## ADR-004: Per-app profiles with global default
Users want different EQ for games vs music vs calls. Global default
ensures apps without a custom profile still get equalized rather than
bypassing the DSP chain.

## ADR-005: f64 internal math, f32 I/O boundary
WASAPI delivers f32 samples. We promote to f64 for DSP math to avoid
numerical precision loss (especially at near-DC frequencies and high
gain values), then demote back to f32 for output.

## ADR-006: No allocation in audio thread
Heap allocation in a real-time audio thread risks priority inversion and
unpredictable latency spikes. All audio thread data structures are
pre-allocated and stack-based. FilterChain uses a fixed [FilterBand; 16]
array for this reason.

## ADR-007: Spectrum analyzer deferred to Phase 2, then shipped
Adds significant complexity (FFT, ring buffer, thread-safe visualization
pipeline). Core EQ functionality shipped first; spectrum analyzer implemented
in Phase 2 (`eq-audio/src/spectrum.rs`) once the audio pipeline was proven
stable. Uses a 2048-sample Hann-windowed FFT mapped to 80 log-spaced bands
(20 Hz – 20 kHz) with fast-attack/slow-release smoothing. Allocation-free
hot path — all buffers pre-allocated at construction.

## ADR-008: Virtual audio cable (VB-Audio) over a custom kernel driver
Windows only exposes audio endpoints to apps that ship a signed kernel-mode
driver — something that requires Microsoft approval and is far out of scope
for this project. The alternatives were:

- **Custom kernel driver**: requires driver signing certificate, WDK expertise,
  and WHQL certification. Effectively impossible for an indie app.
- **Windows APO (Audio Processing Object)**: a COM DLL injected into an
  existing device's processing pipeline. Avoids a kernel driver but requires
  registry-level device topology edits, breaks per-app profiles (APOs sit
  below the session layer), and makes uninstall fragile. Would invalidate the
  entire `eq-audio` crate.
- **VB-Audio Virtual Cable** (chosen): free, widely used, signed by the vendor.
  The user sets `CABLE Input` as their Windows default output; soundEQ captures
  from it via WASAPI loopback and renders to real speakers. No code changes to
  the DSP or capture pipeline; the existing `LoopbackCapture` device-ID
  parameter handles device selection cleanly.

Trade-off: requires the user to install a third-party driver and change their
default audio device. Mitigated by auto-detection of the cable device on
startup and a guided setup banner in the UI.