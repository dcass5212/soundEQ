// =============================================================================
// filter_chain.rs — Multi-band stereo filter chain
//
// A FilterChain holds up to MAX_BANDS biquad filters and processes audio
// buffers through all of them in series. This is the component that the
// WASAPI audio loop will call directly — it's the bridge between the
// DSP math (biquad.rs) and the real audio stream.
//
// STEREO DESIGN:
// Audio is interleaved: [L, R, L, R, ...] in a flat Vec<f32>.
// Each channel gets its OWN independent filter state because L and R channels
// have different sample histories. We keep two FilterBand instances per band
// (one per channel) internally.
//
// WHY f32 AT THE BOUNDARY?
// WASAPI delivers and expects f32 samples. We do internal math in f64 for
// numerical precision (important for near-DC frequencies and high-gain
// operations), then convert back to f32 at the output boundary.
// =============================================================================

use crate::biquad::{BiquadFilter, Coefficients};
use crate::filter_type::BandConfig;

/// Maximum number of EQ bands. 16 covers the most demanding parametric setups.
/// This is a compile-time constant so the filter bank can be stack-allocated,
/// avoiding heap allocation in the audio thread.
pub const MAX_BANDS: usize = 16;

// ---------------------------------------------------------------------------
// FilterBand — one EQ band with independent L and R channel state
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
struct FilterBand {
    left:  BiquadFilter,
    right: BiquadFilter,
}

impl FilterBand {
    fn passthrough() -> Self {
        Self {
            left:  BiquadFilter::passthrough(),
            right: BiquadFilter::passthrough(),
        }
    }

    /// Updates both channels with new coefficients (no state reset).
    fn update(&mut self, coeffs: Coefficients) {
        self.left.update_coefficients(coeffs);
        self.right.update_coefficients(coeffs);
    }

    fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }
}

// ---------------------------------------------------------------------------
// FilterChain — the full stereo EQ engine
//
// Holds up to MAX_BANDS bands, each with L and R filter state.
// The active_count field controls how many bands are actually processed —
// unused slots are skipped entirely.
// ---------------------------------------------------------------------------
pub struct FilterChain {
    // Fixed-size array of bands — no heap allocation, safe for audio thread
    bands: [FilterBand; MAX_BANDS],

    /// How many bands are currently active (0..=MAX_BANDS).
    /// Bands beyond this index are ignored during processing.
    active_count: usize,

    /// Sample rate in Hz — needed to recompute coefficients when bands change.
    /// Stored here so the chain is self-contained; caller doesn't need to pass
    /// it on every update.
    sample_rate: f64,
}

impl FilterChain {
    /// Creates a new chain with no active bands (pure passthrough).
    pub fn new(sample_rate: f64) -> Self {
        Self {
            // Initialize all MAX_BANDS slots as passthrough filters
            bands: std::array::from_fn(|_| FilterBand::passthrough()),
            active_count: 0,
            sample_rate,
        }
    }

    // -------------------------------------------------------------------------
    // Configuration API — called from the Tauri command handlers
    // (NOT called from the audio thread — these are control-plane operations)
    // -------------------------------------------------------------------------

    /// Rebuilds the entire filter bank from a list of BandConfigs.
    /// Called when the user switches profiles or loads a preset.
    ///
    /// This replaces all bands atomically — in the full app we'll swap
    /// the chain pointer atomically so the audio thread never sees a
    /// half-updated state.
    pub fn set_bands(&mut self, bands: &[BandConfig]) {
        let count = bands.len().min(MAX_BANDS);
        for (i, band) in bands.iter().enumerate().take(count) {
            let coeffs = Coefficients::from_band(band, self.sample_rate);
            self.bands[i].update(coeffs);
        }
        // Reset any slots that are no longer used (edge case: user deleted bands)
        for i in count..self.active_count {
            self.bands[i].update(Coefficients::passthrough());
        }
        self.active_count = count;
    }

    /// Updates a single band by index without touching other bands.
    /// Called when the user tweaks a knob in real time.
    /// This is the hot path for live editing — must be fast.
    pub fn update_band(&mut self, index: usize, band: &BandConfig) {
        debug_assert!(index < MAX_BANDS, "update_band: index {index} out of range (MAX_BANDS={MAX_BANDS})");
        if index >= MAX_BANDS {
            return; // silent guard — index validated on the frontend
        }
        let coeffs = Coefficients::from_band(band, self.sample_rate);
        self.bands[index].update(coeffs);
        // Extend active_count if we're adding to the end
        if index >= self.active_count {
            self.active_count = index + 1;
        }
    }

    /// Removes a band by index, shifting subsequent bands down.
    /// After removal, active_count decreases by 1.
    pub fn remove_band(&mut self, index: usize) {
        if index >= self.active_count {
            return;
        }
        // Shift all bands after `index` one position to the left
        for i in index..self.active_count - 1 {
            // Copy the filter state and coefficients from band i+1 to band i.
            // We clone the whole FilterBand to avoid partial-move issues.
            self.bands[i] = self.bands[i + 1].clone();
        }
        // Zero out the now-unused last slot
        self.bands[self.active_count - 1] = FilterBand::passthrough();
        self.active_count -= 1;
    }

    /// Updates the sample rate (e.g., when the audio device changes).
    /// Resets all filter state — brief silence artifact is acceptable here
    /// because this only happens on device switch, not during normal use.
    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        for band in &mut self.bands {
            band.reset();
        }
    }

    /// Bypasses all filtering — output == input.
    pub fn bypass_all(&mut self) {
        for band in &mut self.bands {
            band.update(Coefficients::passthrough());
        }
        self.active_count = 0;
    }

    /// Returns the current number of active bands.
    pub fn band_count(&self) -> usize {
        self.active_count
    }

    // -------------------------------------------------------------------------
    // Audio processing — called from the WASAPI audio thread
    //
    // These methods must be:
    //   - Allocation-free (no Vec, no Box, no String in the hot path)
    //   - Branch-predictable (the loop count is known at entry)
    //   - Numerically stable (f64 internal, f32 I/O)
    // -------------------------------------------------------------------------

    /// Processes an interleaved stereo buffer in-place.
    ///
    /// `buffer` is a mutable slice of interleaved f32 samples:
    ///   [sample0_L, sample0_R, sample1_L, sample1_R, ...]
    ///
    /// The buffer is modified in place — this avoids allocating a second
    /// buffer on every callback (which would cause memory pressure and GC-like
    /// pauses if we had a GC — one of the reasons we're using Rust).
    pub fn process_interleaved(&mut self, buffer: &mut [f32]) {
        // Nothing to do if no bands are active — fast path
        if self.active_count == 0 {
            return;
        }

        // Walk through sample pairs (L, R)
        // chunk_exact_mut(2) is safe here because WASAPI always gives us
        // even-length interleaved stereo buffers. We assert this in the
        // WASAPI integration layer.
        for chunk in buffer.chunks_exact_mut(2) {
            // Promote f32 → f64 for internal processing
            let mut l = chunk[0] as f64;
            let mut r = chunk[1] as f64;

            // Run through each active band in series.
            // The output of band N is the input to band N+1.
            // This is called a "cascaded" filter topology.
            for band in &mut self.bands[..self.active_count] {
                l = band.left.process_sample(l);
                r = band.right.process_sample(r);
            }

            // Clamp to [-1.0, 1.0] before demoting back to f32.
            // Without clamping, a large boost could produce values > 1.0,
            // which wraps around in f32 and causes harsh digital clipping.
            // Soft clamping here prevents that; the UI also enforces gain limits.
            chunk[0] = (l as f32).clamp(-1.0, 1.0);
            chunk[1] = (r as f32).clamp(-1.0, 1.0);
        }
    }

    /// Processes a mono buffer in-place (for mono audio sources).
    pub fn process_mono(&mut self, buffer: &mut [f32]) {
        if self.active_count == 0 {
            return;
        }
        for sample in buffer.iter_mut() {
            let mut s = *sample as f64;
            for band in &mut self.bands[..self.active_count] {
                // For mono, use only the left channel filter state
                s = band.left.process_sample(s);
            }
            *sample = (s as f32).clamp(-1.0, 1.0);
        }
    }

    // -------------------------------------------------------------------------
    // Frequency response query — used by the frontend to draw the EQ curve
    //
    // NOT called from the audio thread. The UI calls this via IPC to get
    // the combined gain-vs-frequency curve to display on screen.
    // -------------------------------------------------------------------------

    /// Computes the combined magnitude response (in dB) at a given frequency.
    /// The total response is the SUM of each band's dB response at that frequency.
    /// (In linear terms, cascaded filters multiply; in dB they add.)
    pub fn magnitude_db_at(&self, freq: f64) -> f64 {
        self.bands[..self.active_count]
            .iter()
            .map(|band| band.left.magnitude_db_at(freq, self.sample_rate))
            .sum()
    }

    /// Computes the magnitude response curve across a range of frequencies.
    /// Returns a Vec of (frequency_hz, gain_db) pairs suitable for the UI chart.
    ///
    /// `points` controls the resolution. 256 points looks smooth on screen
    /// and is fast to compute (called on UI thread, not audio thread).
    pub fn frequency_response_curve(&self, points: usize) -> Vec<(f64, f64)> {
        // Distribute points logarithmically from 20 Hz to 20 kHz.
        // Log spacing matches how humans perceive pitch — equal spacing would
        // cram all the interesting musical content into a tiny left portion.
        let log_min = (20.0_f64).log10();
        let log_max = (20_000.0_f64).log10();
        let step    = (log_max - log_min) / (points - 1) as f64;

        (0..points)
            .map(|i| {
                let freq = 10.0_f64.powf(log_min + i as f64 * step);
                let db   = self.magnitude_db_at(freq);
                (freq, db)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter_type::{BandConfig, FilterType};
    use approx::assert_relative_eq;

    const SR: f64 = 48_000.0;

    fn make_peak_band(freq: f64, gain_db: f64) -> BandConfig {
        let mut b = BandConfig::new(FilterType::Peak, freq);
        b.gain_db = gain_db;
        b
    }

    #[test]
    fn empty_chain_is_passthrough() {
        let mut chain = FilterChain::new(SR);
        let mut buf = vec![0.5f32, -0.5f32, 0.3f32, -0.3f32];
        let original = buf.clone();
        chain.process_interleaved(&mut buf);
        for (a, b) in buf.iter().zip(original.iter()) {
            assert_relative_eq!(a, b, epsilon = 1e-6);
        }
    }

    #[test]
    fn single_band_chain_applies_eq() {
        let mut chain = FilterChain::new(SR);
        chain.set_bands(&[make_peak_band(1000.0, 12.0)]);
        // The combined response at 1 kHz should be near +12 dB
        let db = chain.magnitude_db_at(1000.0);
        assert_relative_eq!(db, 12.0, epsilon = 0.2);
    }

    #[test]
    fn two_bands_responses_add_in_db() {
        let mut chain = FilterChain::new(SR);
        // Two non-overlapping peaks: one at 200 Hz and one at 8 kHz
        let bands = [
            make_peak_band(200.0,  6.0),
            make_peak_band(8000.0, 3.0),
        ];
        chain.set_bands(&bands);
        // At 200 Hz, we should see ~+6 dB (the 8 kHz band contributes ~0)
        let db_200 = chain.magnitude_db_at(200.0);
        assert_relative_eq!(db_200, 6.0, epsilon = 0.5);
        // At 8 kHz, we should see ~+3 dB
        let db_8k = chain.magnitude_db_at(8000.0);
        assert_relative_eq!(db_8k, 3.0, epsilon = 0.5);
    }

    #[test]
    fn frequency_response_curve_has_correct_length() {
        let chain = FilterChain::new(SR);
        let curve = chain.frequency_response_curve(256);
        assert_eq!(curve.len(), 256);
        // First point should be near 20 Hz
        assert_relative_eq!(curve[0].0, 20.0, epsilon = 0.5);
        // Last point should be near 20 kHz
        assert_relative_eq!(curve[255].0, 20_000.0, epsilon = 100.0);
    }

    #[test]
    fn remove_band_shifts_correctly() {
        let mut chain = FilterChain::new(SR);
        let bands = [
            make_peak_band(200.0,  3.0),
            make_peak_band(1000.0, 6.0),
            make_peak_band(8000.0, 9.0),
        ];
        chain.set_bands(&bands);
        assert_eq!(chain.band_count(), 3);
        chain.remove_band(1); // remove the 1 kHz band
        assert_eq!(chain.band_count(), 2);
        // The 8 kHz band should have shifted into slot 1 and still give ~9 dB
        let db = chain.magnitude_db_at(8000.0);
        assert_relative_eq!(db, 9.0, epsilon = 0.5);
    }

    #[test]
    fn bypass_all_makes_chain_passthrough() {
        let mut chain = FilterChain::new(SR);
        chain.set_bands(&[make_peak_band(1000.0, 12.0)]);
        chain.bypass_all();
        let db = chain.magnitude_db_at(1000.0);
        assert_relative_eq!(db, 0.0, epsilon = 0.01);
    }

    #[test]
    fn processing_clamps_output_to_safe_range() {
        let mut chain = FilterChain::new(SR);
        // Maximum gain on a peak — this could theoretically push output above 1.0
        let mut band = make_peak_band(1000.0, 24.0);
        band.q = 10.0; // narrow, very hot
        chain.set_bands(&[band]);
        // Feed a full-scale sine-like signal
        let mut buf: Vec<f32> = (0..256).map(|i| {
            let t = i as f32 / 48_000.0;
            (2.0 * std::f32::consts::PI * 1000.0 * t).sin()
        }).flat_map(|s| [s, s]).collect();

        chain.process_interleaved(&mut buf);
        // All samples must be within [-1.0, 1.0] after clamping
        for &s in &buf {
            assert!(s >= -1.0 && s <= 1.0, "Sample {} out of range", s);
        }
    }
}
