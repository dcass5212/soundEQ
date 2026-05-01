// =============================================================================
// crossfeed.rs — Headphone crossfeed processor
//
// WHAT IS CROSSFEED?
// When listening through headphones, each ear hears only one channel:
//   Left ear  → left channel only
//   Right ear → right channel only
//
// This is not how speakers work. With speakers, each ear hears BOTH speakers —
// the left ear hears the left speaker directly AND the right speaker with a
// slight delay (~0.3 ms) and some high-frequency attenuation (the head blocks
// some HF energy from the opposite side). This natural "crosstalk" makes stereo
// feel wider and more relaxed compared to headphones.
//
// Crossfeed recreates that crosstalk for headphones by:
//   1. Low-pass filtering each channel (simulating the head's HF shadow)
//   2. Slightly delaying the filtered copy (~0.3 ms inter-aural time difference)
//   3. Blending the result into the opposite channel at a reduced level
//
// This makes stereo imaging feel more like speakers in a room and reduces the
// "ear fatigue" of extreme left/right panning on headphones.
//
// ALGORITHM (Linkwitz-style):
//   lp_L = low_pass_filter(L)           — HP shadow on left channel
//   lp_R = low_pass_filter(R)           — HP shadow on right channel
//   cross_L = delay(lp_L)               — delayed LP'd left  → feeds right ear
//   cross_R = delay(lp_R)               — delayed LP'd right → feeds left ear
//
//   L_out = (1 - c) * L  +  c * cross_R
//   R_out = (1 - c) * R  +  c * cross_L
//
// The blend coefficient c ranges from 0.25 (Mild) to 0.55 (Strong).
// At c=0.25: 75% direct + 25% cross — subtle, open stereo image.
// At c=0.55: 45% direct + 55% cross — more speaker-like, narrower image.
// =============================================================================

use serde::{Deserialize, Serialize};

use crate::biquad::{BiquadFilter, Coefficients};

// ---------------------------------------------------------------------------
// CrossfeedLevel — the three preset strength settings
// ---------------------------------------------------------------------------

/// How strongly crossfeed blends the channels together.
///
/// Higher levels produce a more speaker-like sound but reduce stereo width.
/// "Mild" is a good starting point for most listeners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CrossfeedLevel {
    #[default]
    Mild,
    Moderate,
    Strong,
}

impl CrossfeedLevel {
    /// Returns the blend coefficient `c` for this level.
    ///
    /// c = 0.0 → no crossfeed (pure stereo separation)
    /// c = 1.0 → full mono (both channels identical)
    ///
    /// Practical range: 0.25–0.55. Below 0.25 is barely audible;
    /// above 0.55 collapses the stereo image to the point of discomfort.
    pub fn blend(self) -> f64 {
        match self {
            CrossfeedLevel::Mild     => 0.25,
            CrossfeedLevel::Moderate => 0.40,
            CrossfeedLevel::Strong   => 0.55,
        }
    }
}

// ---------------------------------------------------------------------------
// CrossfeedConfig — stored inside each Profile
//
// Per-profile so a "Speakers" profile can have crossfeed off while a
// "Headphones" profile has it on. Old profiles without this field load
// cleanly via #[serde(default)] → crossfeed disabled, level = Mild.
// ---------------------------------------------------------------------------

/// Crossfeed settings stored inside a Profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossfeedConfig {
    /// Whether crossfeed processing is active for this profile.
    pub enabled: bool,
    /// Blend strength — how much of the opposite channel is mixed in.
    pub level: CrossfeedLevel,
}

impl Default for CrossfeedConfig {
    fn default() -> Self {
        Self { enabled: false, level: CrossfeedLevel::Mild }
    }
}

// ---------------------------------------------------------------------------
// CrossfeedProcessor — the live DSP state
//
// Held behind Arc<Mutex<>> in AudioEngine, same pattern as FilterChain.
// The capture thread locks it once per audio buffer after the EQ chain.
//
// HEAP ALLOCATION RULE: none in process_interleaved. All state is
// inline fixed-size arrays — the struct is stack-allocatable.
// ---------------------------------------------------------------------------

/// Maximum delay buffer size in samples.
///
/// 0.3 ms at 192 kHz = 57.6 samples → round up to 64 for headroom.
/// At 48 kHz (typical) the delay is only 14 samples.
const DELAY_CAPACITY: usize = 64;

/// Low-pass cutoff for the cross-path, in Hz.
///
/// 700 Hz is where the head's HF shadowing becomes significant.
/// Below this, both ears hear the far speaker at nearly equal level;
/// above it, the head increasingly attenuates the cross-channel sound.
const LP_CUTOFF_HZ: f64 = 700.0;

/// Q factor for the cross-path low-pass filter.
/// 0.707 (1/√2) = maximally flat (Butterworth) response — no resonance peak,
/// just a smooth −12 dB/octave rolloff above the cutoff.
const LP_Q: f64 = 0.707;

/// Inter-aural time delay in milliseconds.
/// 0.3 ms ≈ 18 cm head width / 340 m/s speed of sound,
/// which is the approximate extra travel time from the far speaker to the
/// near ear compared to the direct speaker-to-near-ear path.
const DELAY_MS: f64 = 0.3;

/// Processes interleaved stereo audio with headphone crossfeed.
///
/// All state is inline (fixed-size arrays, no heap). Safe to hold behind
/// `Arc<Mutex<>>` and update from the control thread between audio buffers.
pub struct CrossfeedProcessor {
    config: CrossfeedConfig,

    // Low-pass biquad on the left channel cross-path.
    // The LP'd left signal is delayed then blended into the right output,
    // simulating the head's HF shadow for far-left content reaching the right ear.
    lp_left: BiquadFilter,
    // Symmetric: LP'd right signal blends into the left output.
    lp_right: BiquadFilter,

    // Ring buffer for delayed LP'd left channel (feeds the right ear).
    delay_left: [f64; DELAY_CAPACITY],
    // Ring buffer for delayed LP'd right channel (feeds the left ear).
    delay_right: [f64; DELAY_CAPACITY],
    // Shared write head for both rings (they always advance together).
    delay_write: usize,
    // Delay in samples, computed from the sample rate. Updated by set_sample_rate.
    delay_samples: usize,

    // Pre-computed blend from config.level so we avoid an enum match per sample.
    blend: f64,
}

impl CrossfeedProcessor {
    /// Creates a new processor with crossfeed disabled, sized for 48 kHz.
    /// Call `update_config` or `set_sample_rate` before first use.
    pub fn new() -> Self {
        let passthrough = BiquadFilter::new(Coefficients::passthrough());
        Self {
            config: CrossfeedConfig::default(),
            lp_left:  passthrough.clone(),
            lp_right: passthrough,
            delay_left:  [0.0; DELAY_CAPACITY],
            delay_right: [0.0; DELAY_CAPACITY],
            delay_write: 0,
            delay_samples: Self::compute_delay(48_000.0),
            blend: CrossfeedLevel::Mild.blend(),
        }
    }

    /// Updates the LP coefficients and delay for a new sample rate.
    ///
    /// Called by the capture thread at startup, after the WASAPI mix format is
    /// known, to match the delay and LP filter to the actual device rate.
    /// LP filter state is reset here (inaudible: brief startup transient only).
    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.delay_samples = Self::compute_delay(sample_rate);
        let coeffs = Coefficients::low_pass(LP_CUTOFF_HZ, LP_Q, sample_rate);
        self.lp_left  = BiquadFilter::new(coeffs);
        self.lp_right = BiquadFilter::new(coeffs);
    }

    /// Applies a new crossfeed configuration.
    ///
    /// Filter state (biquad delay lines, ring buffer) is preserved on enable/
    /// level changes so there are no audible clicks mid-stream. The ring buffer
    /// holds only ~0.3 ms of audio, so any brief mismatch is inaudible.
    ///
    /// LP coefficients are rebuilt only when the level changes (since LP_CUTOFF_HZ
    /// and LP_Q are fixed; only the sample rate matters, and that is separate).
    pub fn update_config(&mut self, config: &CrossfeedConfig, sample_rate: f64) {
        let level_changed = config.level != self.config.level;
        self.config = config.clone();
        self.blend  = config.level.blend();
        self.delay_samples = Self::compute_delay(sample_rate);

        if level_changed {
            // Rebuild LP filters with the current sample rate.
            // State is deliberately reset: level changes are large steps where
            // a brief reset transient (< 1 ms) is less jarring than a filter
            // whose state reflects the wrong Q history.
            let coeffs = Coefficients::low_pass(LP_CUTOFF_HZ, LP_Q, sample_rate);
            self.lp_left  = BiquadFilter::new(coeffs);
            self.lp_right = BiquadFilter::new(coeffs);
        }
    }

    /// Processes an interleaved stereo buffer in-place.
    ///
    /// `buffer` is `[L, R, L, R, ...]`. Samples should be in `[-1.0, 1.0]`;
    /// output is clamped to the same range.
    ///
    /// No-op (immediate return, zero work) when crossfeed is disabled.
    pub fn process_interleaved(&mut self, buffer: &mut [f32]) {
        if !self.config.enabled {
            return;
        }

        let c  = self.blend;
        let c1 = 1.0 - c; // direct-path weight

        for frame in buffer.chunks_exact_mut(2) {
            let l = frame[0] as f64;
            let r = frame[1] as f64;

            // Step 1: low-pass filter each channel.
            // Attenuates HF on the cross-path, mimicking head shadow.
            let lp_l = self.lp_left.process_sample(l);
            let lp_r = self.lp_right.process_sample(r);

            // Step 2: write LP'd samples into the delay rings.
            self.delay_left[self.delay_write]  = lp_l;
            self.delay_right[self.delay_write] = lp_r;

            // Step 3: read the delayed cross-channel values.
            // read_pos = write - delay_samples (mod DELAY_CAPACITY).
            let read = (self.delay_write + DELAY_CAPACITY - self.delay_samples) % DELAY_CAPACITY;
            // delayed LP'd R feeds the left output (cross from right speaker → left ear)
            let cross_to_l = self.delay_right[read];
            // delayed LP'd L feeds the right output (cross from left speaker → right ear)
            let cross_to_r = self.delay_left[read];

            // Advance the write head (ring buffer wrap-around).
            self.delay_write = (self.delay_write + 1) % DELAY_CAPACITY;

            // Step 4: blend direct + cross.
            //   L_out = (1-c)*L + c*(delayed LP'd R)
            //   R_out = (1-c)*R + c*(delayed LP'd L)
            let l_out = c1 * l + c * cross_to_l;
            let r_out = c1 * r + c * cross_to_r;

            frame[0] = l_out.clamp(-1.0, 1.0) as f32;
            frame[1] = r_out.clamp(-1.0, 1.0) as f32;
        }
    }

    /// Converts a wall-clock delay (DELAY_MS) to sample count at `sample_rate`.
    /// Clamped to [1, DELAY_CAPACITY − 1] to keep the ring buffer valid.
    fn compute_delay(sample_rate: f64) -> usize {
        let samples = (DELAY_MS * 0.001 * sample_rate).round() as usize;
        samples.clamp(1, DELAY_CAPACITY - 1)
    }
}

impl Default for CrossfeedProcessor {
    fn default() -> Self {
        Self::new()
    }
}
