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

## ADR-007: Spectrum analyzer deferred
Adds significant complexity (FFT, ring buffer, thread-safe visualization
pipeline). Core EQ functionality ships first; spectrum analyzer added in
Phase 2 once the audio pipeline is proven stable.