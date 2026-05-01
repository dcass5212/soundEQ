// =============================================================================
// spectrum.rs — Real-time FFT-based spectrum analyzer
//
// WHAT THIS DOES:
// Accumulates processed audio samples on the WASAPI capture thread, computes
// a Hann-windowed FFT when enough samples are collected, maps the frequency
// bins into SPECTRUM_BANDS log-spaced output bands (20 Hz – 20 kHz), and
// publishes the magnitude results to a shared Arc<Mutex<Vec<f32>>> that the
// Tauri command layer can read at any time.
//
// WHY FFT:
// An FFT (Fast Fourier Transform) decomposes a short block of time-domain
// samples (amplitude over time) into frequency-domain data (amplitude at each
// frequency). For audio visualization this tells us how much energy is at
// 100 Hz vs 10 kHz — exactly what we need to draw a live spectrum display.
//
// WINDOWING:
// A raw FFT assumes the analysis block repeats forever. Real audio doesn't,
// so there are discontinuities at the block edges that create fake high-
// frequency content (called spectral leakage). Multiplying by a Hann window
// tapers the signal smoothly to zero at both edges, greatly reducing this
// artefact and giving a much cleaner spectrum.
//
// DESIGN CONSTRAINT — NO ALLOCATION IN THE HOT PATH:
// push_samples() is called from the WASAPI capture callback thread.
// All buffers are pre-allocated in new(); push_samples() and compute_fft()
// never call Vec::push, Box::new, String::new, or any other allocator.
// The only runtime allocation is inside try_lock() (kernel mutex), which is
// a single compare-and-swap and is acceptable in an audio callback.
//
// SMOOTHING:
// Raw FFT output flickers wildly because each 43 ms frame is independent.
// We apply fast-attack / slow-release smoothing: the spectrum jumps up
// immediately when a new peak arrives but decays slowly afterward, giving
// the classic "falling needles" look without distracting flicker.
// =============================================================================

use std::sync::{Arc, Mutex};

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of samples per FFT analysis window.
///
/// At 48 kHz: 2048 frames ≈ 42.7 ms per window.
/// Bin width (frequency resolution): 48000 / 2048 ≈ 23.4 Hz.
///
/// Must be a power of two — rustfft optimises radix-2 FFTs.
const FFT_SIZE: usize = 2048;

/// Number of output frequency bands, log-spaced from 20 Hz to 20 kHz.
///
/// 80 bands distributes evenly across the log-frequency axis used by the
/// EqCanvas, giving a dense but not cluttered spectrum display.
pub const SPECTRUM_BANDS: usize = 80;

/// Slow-release decay factor applied each analysis frame to bands that have
/// dropped below their previous peak value.
///
/// 0.85 ≈ the band loses ~15% of its remaining gap each frame.
/// At the ~23 Hz frame rate this gives a ~0.5-second fall time.
const DECAY: f32 = 0.85;

// ---------------------------------------------------------------------------
// SpectrumAnalyzer
// ---------------------------------------------------------------------------

/// Computes a real-time frequency spectrum from a live audio stream.
///
/// Constructed once when the engine starts; dropped when the engine stops.
/// All internal buffers are allocated in new() and never reallocated during
/// operation. See module-level docs for the full design rationale.
pub struct SpectrumAnalyzer {
    // --- Pre-allocated FFT working buffers ---

    /// Complex-valued input buffer for the FFT. We write windowed real samples
    /// here (imaginary parts stay 0 — audio is a real-valued signal).
    fft_input: Vec<Complex<f32>>,

    /// Scratch buffer required by rustfft's in-place transform.
    /// Length is determined by the planner (often 0 for power-of-2 sizes).
    fft_scratch: Vec<Complex<f32>>,

    /// Hann window coefficients: w[n] = 0.5 × (1 − cos(2πn / (N−1))).
    /// Pre-computed once at construction, multiplied into each FFT input frame.
    window: Vec<f32>,

    /// Mono sample accumulator. Incoming interleaved stereo is downmixed here.
    /// Fills up to FFT_SIZE then triggers compute_fft().
    accumulator: Vec<f32>,

    /// How many mono samples have been written into `accumulator` so far.
    /// Resets to 0 after each FFT.
    acc_pos: usize,

    /// Temporary per-FFT band magnitude results (in dB) before smoothing.
    /// Written by compute_fft(), consumed immediately when writing to output.
    band_buf: Vec<f32>,

    /// Sample rate of the audio stream (Hz). Used to map FFT bin indices to Hz.
    sample_rate: f32,

    /// The planned FFT instance. Planning is done once at construction;
    /// process_with_scratch() is allocation-free at runtime.
    fft: Arc<dyn rustfft::Fft<f32>>,

    /// Shared magnitude buffer read by the Tauri `get_spectrum` command.
    /// Values are in dB; SPECTRUM_BANDS elements.
    output: Arc<Mutex<Vec<f32>>>,
}

impl SpectrumAnalyzer {
    /// Creates a new SpectrumAnalyzer and pre-allocates all internal buffers.
    ///
    /// `sample_rate` — the sample rate of the audio stream being analyzed
    ///                 (from `StreamFormat::sample_rate` after WASAPI setup).
    ///
    /// `output` — shared Vec written after each FFT and read by `get_spectrum`.
    ///            Caller must initialize it to SPECTRUM_BANDS elements.
    pub fn new(sample_rate: u32, output: Arc<Mutex<Vec<f32>>>) -> Self {
        // Plan the FFT once. The planner chooses the fastest algorithm for
        // this size (radix-2 Cooley-Tukey for power-of-2 inputs).
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let scratch_len = fft.get_inplace_scratch_len();

        // Pre-compute Hann window coefficients.
        // w[n] = 0.5 × (1 − cos(2πn / (N−1))) ranges from 0 at the edges to
        // 1 at the center. Multiplying the signal by this before the FFT
        // eliminates the discontinuity at the block boundary.
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|n| {
                0.5 * (1.0
                    - (2.0 * std::f32::consts::PI * n as f32 / (FFT_SIZE - 1) as f32).cos())
            })
            .collect();

        Self {
            fft_input:   vec![Complex::new(0.0, 0.0); FFT_SIZE],
            fft_scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            window,
            accumulator: vec![0.0; FFT_SIZE],
            acc_pos:     0,
            band_buf:    vec![-100.0; SPECTRUM_BANDS],
            sample_rate: sample_rate as f32,
            fft,
            output,
        }
    }

    /// Called from the WASAPI capture callback with the latest processed samples.
    ///
    /// `samples` is an interleaved stereo f32 buffer: [L₀, R₀, L₁, R₁, ...].
    ///
    /// Downmixes to mono, fills the accumulator, and calls compute_fft() once
    /// per FFT_SIZE mono frames. Never allocates.
    pub fn push_samples(&mut self, samples: &[f32]) {
        let mut i = 0;
        while i + 1 < samples.len() {
            // Average L and R to get mono. Averaging (not summing) preserves
            // amplitude: a full-scale tone in one channel stays full-scale mono.
            let mono = (samples[i] + samples[i + 1]) * 0.5;
            self.accumulator[self.acc_pos] = mono;
            self.acc_pos += 1;

            if self.acc_pos == FFT_SIZE {
                self.compute_fft();
                self.acc_pos = 0;
            }

            i += 2; // advance one stereo frame (L+R = 2 f32 values)
        }
    }

    /// Runs the windowed FFT on the current accumulator contents and updates
    /// the shared output buffer with smoothed log-band magnitudes.
    ///
    /// Called once every FFT_SIZE mono frames (~42.7 ms at 48 kHz).
    fn compute_fft(&mut self) {
        // Apply the Hann window and pack real samples into the complex input.
        // The imaginary part is always 0 for real-valued audio.
        for n in 0..FFT_SIZE {
            self.fft_input[n] = Complex::new(self.accumulator[n] * self.window[n], 0.0);
        }

        // Execute the forward FFT in-place.
        // After this, fft_input[k] holds the complex amplitude at frequency
        // k × (sample_rate / FFT_SIZE) Hz. process_with_scratch never allocates.
        self.fft
            .process_with_scratch(&mut self.fft_input, &mut self.fft_scratch);

        // Only bins 0 .. FFT_SIZE/2 contain unique information.
        // The upper half mirrors the lower half (Hermitian symmetry for real input).
        // Bin i ↔ frequency: i × sample_rate / FFT_SIZE.
        let bin_hz       = self.sample_rate / FFT_SIZE as f32;
        let nyquist_bins = FFT_SIZE / 2;

        // Map FFT bins into SPECTRUM_BANDS log-spaced frequency bands.
        // Log spacing means equal numbers of bands per octave (20→40 Hz gets
        // the same visual width as 10 kHz→20 kHz), which matches how human
        // hearing works and how the EqCanvas frequency axis is laid out.
        let log_min = 20.0_f32.log10();       // log₁₀(20 Hz)
        let log_max = 20_000.0_f32.log10();   // log₁₀(20 000 Hz)

        for b in 0..SPECTRUM_BANDS {
            // Frequency edges of this band on the log scale.
            let t_lo = b as f32 / SPECTRUM_BANDS as f32;
            let t_hi = (b + 1) as f32 / SPECTRUM_BANDS as f32;
            let f_lo = 10.0_f32.powf(log_min + t_lo * (log_max - log_min));
            let f_hi = 10.0_f32.powf(log_min + t_hi * (log_max - log_min));

            // Convert Hz to FFT bin indices.
            // Skip bin 0 (DC offset — not musically meaningful).
            // Clamp to [1, nyquist_bins) so we stay within the unique range.
            let bin_lo = ((f_lo / bin_hz) as usize).max(1).min(nyquist_bins - 1);
            let bin_hi = ((f_hi / bin_hz) as usize).max(bin_lo + 1).min(nyquist_bins);

            // Take the peak magnitude across all FFT bins in this band.
            // .norm() = sqrt(re² + im²), i.e. the amplitude at that frequency.
            // Peak-hold (vs. average) better represents transient content.
            let mut peak: f32 = 0.0;
            for bin in bin_lo..bin_hi {
                let mag = self.fft_input[bin].norm();
                if mag > peak {
                    peak = mag;
                }
            }

            // Normalize by FFT_SIZE/2 to remove the size dependency, then
            // convert to dB: 20 × log₁₀(amplitude).
            // Anything below 1e-10 is treated as silence (−100 dB floor).
            let norm = peak / (FFT_SIZE as f32 / 2.0);
            self.band_buf[b] = if norm > 1e-10 {
                20.0 * norm.log10()
            } else {
                -100.0
            };
        }

        // Write smoothed results into the shared output.
        //
        // try_lock (non-blocking): if the command thread is reading right now,
        // skip this frame rather than stalling the audio callback.
        // One dropped frame (~43 ms) is invisible at human perception timescales.
        if let Ok(mut out) = self.output.try_lock() {
            for (i, &new_db) in self.band_buf.iter().enumerate() {
                // Fast attack: jump immediately to a louder level.
                // Slow release: blend toward a quieter level with DECAY,
                //               giving the "falling needles" visual behaviour.
                if new_db >= out[i] {
                    out[i] = new_db;
                } else {
                    out[i] = out[i] * DECAY + new_db * (1.0 - DECAY);
                }
            }
        }
    }
}
