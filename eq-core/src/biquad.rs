// =============================================================================
// biquad.rs — Biquad filter: coefficient calculation + sample processing
//
// This is the mathematical heart of the entire EQ.
//
// WHAT IS A BIQUAD FILTER?
// A biquad (short for "bi-quadratic") is a second-order IIR (Infinite Impulse
// Response) digital filter. Every EQ band — regardless of type — is implemented
// as one biquad filter. Stack multiple biquads in series and you have a
// parametric EQ with N bands.
//
// THE DIFFERENCE EQUATION (what happens to each audio sample):
//
//   y[n] = b0*x[n] + b1*x[n-1] + b2*x[n-2]
//                  - a1*y[n-1] - a2*y[n-2]
//
//   Where:
//     x[n] = current input sample
//     x[n-1], x[n-2] = the two previous input samples ("feed-forward history")
//     y[n-1], y[n-2] = the two previous output samples ("feedback history")
//     b0,b1,b2,a1,a2 = the five coefficients that define the filter's behavior
//
// The coefficients are derived from the desired frequency, gain, Q, and sample
// rate using the Audio EQ Cookbook formulas by Robert Bristow-Johnson, which are
// the industry standard for this type of work.
//
// Reference: https://www.w3.org/TR/audio-eq-cookbook/
// =============================================================================

use crate::filter_type::{BandConfig, FilterType};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

// ---------------------------------------------------------------------------
// Coefficients — the 5 numbers that fully define a biquad filter
//
// These are computed once when the user changes a band parameter,
// then reused for every audio sample until the next change.
// Pre-computing them is what makes real-time processing efficient.
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Coefficients {
    // Feedforward coefficients (applied to input samples)
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    // Feedback coefficients (applied to previous output samples)
    // Note: a0 is always normalized to 1.0, so we only store a1 and a2
    pub a1: f64,
    pub a2: f64,
}

impl Coefficients {
    /// Returns the "identity" coefficients — passes audio through unmodified.
    /// Used when a band is disabled or gain is 0 dB.
    pub fn passthrough() -> Self {
        // b0=1, all others 0: y[n] = 1*x[n] = x[n]. Perfect bypass.
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 }
    }

    // -------------------------------------------------------------------------
    // Coefficient calculation formulas (Audio EQ Cookbook by R. Bristow-Johnson)
    //
    // All formulas share these intermediate values:
    //   w0 = 2π * frequency / sample_rate   (angular frequency, radians/sample)
    //   cos_w0, sin_w0                        (trig values used in all formulas)
    //   alpha = sin_w0 / (2*Q)               (bandwidth term, used in most filters)
    //   A = 10^(gain_db/40)                  (linear amplitude for shelves/peak)
    // -------------------------------------------------------------------------

    /// Computes coefficients for a Peak (bell) filter.
    /// Boosts or cuts a band of frequencies centered at `freq` Hz.
    pub fn peak(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        let w0    = 2.0 * PI * freq / sample_rate;
        let cos_w = w0.cos();
        let sin_w = w0.sin();
        // alpha controls the bandwidth — how wide the bell is
        let alpha = sin_w / (2.0 * q);
        // A is the linear gain amplitude. /40 because dB is a power ratio,
        // and we want the amplitude (voltage) ratio: A = sqrt(10^(dB/20))
        // which simplifies to 10^(dB/40).
        let a     = (10.0_f64).powf(gain_db / 40.0);

        // Normalize everything by a0 so we don't need to divide during processing
        let a0 =  1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a)   / a0,
            b1: (-2.0 * cos_w)      / a0,
            b2: (1.0 - alpha * a)   / a0,
            a1: (-2.0 * cos_w)      / a0,
            a2: (1.0 - alpha / a)   / a0,
        }
    }

    /// Computes coefficients for a Low Shelf filter.
    /// Boosts or cuts all frequencies below `freq` Hz.
    pub fn low_shelf(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        let w0    = 2.0 * PI * freq / sample_rate;
        let cos_w = w0.cos();
        let sin_w = w0.sin();
        let a     = (10.0_f64).powf(gain_db / 40.0);
        // Shelf slope term — Q of 0.707 gives a maximally flat (Butterworth) shelf
        let alpha = sin_w / 2.0 * ((a + 1.0 / a) * (1.0 / q - 1.0) + 2.0).sqrt();

        let a0 =        (a+1.0) + (a-1.0)*cos_w + 2.0*a.sqrt()*alpha;
        Self {
            b0: (a * ((a+1.0) - (a-1.0)*cos_w + 2.0*a.sqrt()*alpha)) / a0,
            b1: (2.0 * a * ((a-1.0) - (a+1.0)*cos_w))                 / a0,
            b2: (a * ((a+1.0) - (a-1.0)*cos_w - 2.0*a.sqrt()*alpha)) / a0,
            a1: (-2.0 * ((a-1.0) + (a+1.0)*cos_w))                    / a0,
            a2: ((a+1.0) + (a-1.0)*cos_w - 2.0*a.sqrt()*alpha)       / a0,
        }
    }

    /// Computes coefficients for a High Shelf filter.
    /// Boosts or cuts all frequencies above `freq` Hz.
    pub fn high_shelf(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        let w0    = 2.0 * PI * freq / sample_rate;
        let cos_w = w0.cos();
        let sin_w = w0.sin();
        let a     = (10.0_f64).powf(gain_db / 40.0);
        let alpha = sin_w / 2.0 * ((a + 1.0 / a) * (1.0 / q - 1.0) + 2.0).sqrt();

        let a0 =        (a+1.0) - (a-1.0)*cos_w + 2.0*a.sqrt()*alpha;
        Self {
            b0: (a * ((a+1.0) + (a-1.0)*cos_w + 2.0*a.sqrt()*alpha)) / a0,
            b1: (-2.0 * a * ((a-1.0) + (a+1.0)*cos_w))                / a0,
            b2: (a * ((a+1.0) + (a-1.0)*cos_w - 2.0*a.sqrt()*alpha)) / a0,
            a1: (2.0 * ((a-1.0) - (a+1.0)*cos_w))                     / a0,
            a2: ((a+1.0) - (a-1.0)*cos_w - 2.0*a.sqrt()*alpha)       / a0,
        }
    }

    /// Computes coefficients for a Low-Pass filter.
    /// Passes frequencies below `freq`, attenuates above.
    /// Q (0.707 = Butterworth) controls resonance at cutoff.
    pub fn low_pass(freq: f64, q: f64, sample_rate: f64) -> Self {
        let w0    = 2.0 * PI * freq / sample_rate;
        let cos_w = w0.cos();
        let sin_w = w0.sin();
        let alpha = sin_w / (2.0 * q);

        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w) / 2.0) / a0,
            b1: (1.0 - cos_w)          / a0,
            b2: ((1.0 - cos_w) / 2.0) / a0,
            a1: (-2.0 * cos_w)         / a0,
            a2: (1.0 - alpha)          / a0,
        }
    }

    /// Computes coefficients for a High-Pass filter.
    /// Passes frequencies above `freq`, attenuates below.
    pub fn high_pass(freq: f64, q: f64, sample_rate: f64) -> Self {
        let w0    = 2.0 * PI * freq / sample_rate;
        let cos_w = w0.cos();
        let sin_w = w0.sin();
        let alpha = sin_w / (2.0 * q);

        let a0 = 1.0 + alpha;
        Self {
            b0:  ((1.0 + cos_w) / 2.0) / a0,
            b1: (-(1.0 + cos_w))        / a0,
            b2:  ((1.0 + cos_w) / 2.0) / a0,
            a1:  (-2.0 * cos_w)         / a0,
            a2:  (1.0 - alpha)          / a0,
        }
    }

    /// Computes coefficients for a Notch filter.
    /// Creates a very narrow cut at `freq`. Use high Q (e.g. 10–30).
    pub fn notch(freq: f64, q: f64, sample_rate: f64) -> Self {
        let w0    = 2.0 * PI * freq / sample_rate;
        let cos_w = w0.cos();
        let sin_w = w0.sin();
        let alpha = sin_w / (2.0 * q);

        let a0 = 1.0 + alpha;
        Self {
            b0:  1.0         / a0,
            b1: (-2.0*cos_w) / a0,
            b2:  1.0         / a0,
            a1: (-2.0*cos_w) / a0,
            a2: (1.0-alpha)  / a0,
        }
    }

    /// Computes coefficients for a Bandpass filter.
    /// Passes only the band around `freq`, attenuates outside.
    /// The peak gain at `freq` is 0 dB regardless of Q.
    pub fn bandpass(freq: f64, q: f64, sample_rate: f64) -> Self {
        let w0    = 2.0 * PI * freq / sample_rate;
        let cos_w = w0.cos();
        let sin_w = w0.sin();
        let alpha = sin_w / (2.0 * q);

        let a0 = 1.0 + alpha;
        Self {
            b0:  (sin_w / 2.0) / a0,   // = alpha * Q normalised — peak is 0dB
            b1:  0.0,
            b2: -(sin_w / 2.0) / a0,
            a1: (-2.0 * cos_w) / a0,
            a2: (1.0 - alpha)  / a0,
        }
    }

    // -------------------------------------------------------------------------
    // Factory: compute correct coefficients from a BandConfig + sample rate
    //
    // This is the single entry point the filter chain will use.
    // -------------------------------------------------------------------------
    pub fn from_band(band: &BandConfig, sample_rate: f64) -> Self {
        // If the band is disabled or gain is exactly 0 on a gain-bearing filter,
        // return passthrough — no math needed, no coloration.
        if !band.enabled {
            return Self::passthrough();
        }
        if band.filter_type.uses_gain() && band.gain_db == 0.0 {
            return Self::passthrough();
        }

        match band.filter_type {
            FilterType::Peak      => Self::peak(band.frequency, band.gain_db, band.q, sample_rate),
            FilterType::LowShelf  => Self::low_shelf(band.frequency, band.gain_db, band.q, sample_rate),
            FilterType::HighShelf => Self::high_shelf(band.frequency, band.gain_db, band.q, sample_rate),
            FilterType::LowPass   => Self::low_pass(band.frequency, band.q, sample_rate),
            FilterType::HighPass  => Self::high_pass(band.frequency, band.q, sample_rate),
            FilterType::Notch     => Self::notch(band.frequency, band.q, sample_rate),
            FilterType::Bandpass  => Self::bandpass(band.frequency, band.q, sample_rate),
        }
    }
}

// ---------------------------------------------------------------------------
// BiquadFilter — a single stateful filter instance
//
// The "state" (x1, x2, y1, y2) is the delay line — the previous two input
// and output samples needed for the difference equation.
//
// IMPORTANT: State must be maintained between audio buffer calls.
// If we reset state on each buffer, you'd hear a click artifact every buffer
// boundary because the filter "forgets" where it was.
//
// STEREO: Each channel needs its OWN BiquadFilter instance because the left
// and right channels have independent histories. The FilterChain handles this.
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct BiquadFilter {
    pub coeffs: Coefficients,
    // Delay line — previous input samples (x[n-1], x[n-2])
    x1: f64,
    x2: f64,
    // Delay line — previous output samples (y[n-1], y[n-2])
    y1: f64,
    y2: f64,
}

impl BiquadFilter {
    /// Creates a new filter with the given coefficients and zeroed state.
    pub fn new(coeffs: Coefficients) -> Self {
        Self { coeffs, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    /// Creates a passthrough filter — output == input.
    pub fn passthrough() -> Self {
        Self::new(Coefficients::passthrough())
    }

    /// Updates coefficients without resetting the delay line state.
    ///
    /// This is critical for smooth parameter changes. If we reset state here,
    /// changing a knob would cause an audible click/pop. Updating coefficients
    /// mid-stream is safe because the state naturally decays to the new response.
    pub fn update_coefficients(&mut self, coeffs: Coefficients) {
        self.coeffs = coeffs;
        // Do NOT touch x1, x2, y1, y2 — preserving state prevents clicks
    }

    /// Processes a single audio sample through the biquad difference equation.
    ///
    /// This function will be called 48,000+ times per second per channel.
    /// It must be as fast as possible — no allocations, no branches in the hot path.
    ///
    /// The Direct Form I difference equation:
    ///   y[n] = b0*x[n] + b1*x[n-1] + b2*x[n-2] - a1*y[n-1] - a2*y[n-2]
    #[inline(always)] // force inlining — avoids function call overhead in the audio loop
    pub fn process_sample(&mut self, x: f64) -> f64 {
        let c = &self.coeffs;
        // Apply the difference equation
        let y = c.b0 * x
              + c.b1 * self.x1
              + c.b2 * self.x2
              - c.a1 * self.y1
              - c.a2 * self.y2;

        // Shift the delay line: [n-2] = old [n-1], [n-1] = current
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;

        y
    }

    /// Resets the delay line to silence.
    /// Use this when switching to a completely different audio source,
    /// not when changing EQ parameters.
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    /// Computes the magnitude response (gain in dB) at a given frequency.
    /// Used by the frontend to draw the EQ frequency response curve.
    ///
    /// Math: evaluate the transfer function H(z) at z = e^(j*w0)
    /// where w0 = 2π * freq / sample_rate, then take 20*log10(|H(z)|).
    pub fn magnitude_db_at(&self, freq: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * PI * freq / sample_rate;
        let c = &self.coeffs;

        // Evaluate numerator and denominator as complex numbers at z = e^(jw)
        // Using Euler's formula: e^(jw) = cos(w) + j*sin(w)
        // z^-1 = cos(-w) + j*sin(-w) = cos(w) - j*sin(w)
        let cos1 = w.cos();
        let sin1 = w.sin();
        let cos2 = (2.0 * w).cos();
        let sin2 = (2.0 * w).sin();

        // Numerator: b0 + b1*z^-1 + b2*z^-2
        let num_r = c.b0 + c.b1 * cos1 + c.b2 * cos2;
        let num_i =      - c.b1 * sin1 - c.b2 * sin2;

        // Denominator: 1 + a1*z^-1 + a2*z^-2
        let den_r = 1.0  + c.a1 * cos1 + c.a2 * cos2;
        let den_i =      - c.a1 * sin1 - c.a2 * sin2;

        // |H(z)| = |numerator| / |denominator| (magnitudes of complex numbers)
        let num_mag = (num_r * num_r + num_i * num_i).sqrt();
        let den_mag = (den_r * den_r + den_i * den_i).sqrt();

        // Avoid log(0) — if denominator is essentially zero, return a large negative dB
        if den_mag < 1e-10 {
            return -120.0;
        }

        // Convert magnitude ratio to dB: 20 * log10(ratio)
        20.0 * (num_mag / den_mag).log10()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    const SAMPLE_RATE: f64 = 48_000.0;
    const TOLERANCE: f64 = 0.1; // 0.1 dB tolerance for frequency response tests

    #[test]
    fn passthrough_leaves_signal_unchanged() {
        let mut f = BiquadFilter::passthrough();
        // Feed in a known signal — a simple ramp
        let samples = [0.1, 0.5, -0.3, 0.8, -0.9, 0.2];
        for &s in &samples {
            // A passthrough filter must output exactly what goes in
            assert_relative_eq!(f.process_sample(s), s, epsilon = 1e-10);
        }
    }

    #[test]
    fn peak_filter_boosts_at_center_frequency() {
        // A +6 dB peak at 1 kHz should show ~+6 dB at 1 kHz
        let coeffs = Coefficients::peak(1000.0, 6.0, 1.4, SAMPLE_RATE);
        let filter = BiquadFilter::new(coeffs);
        let db = filter.magnitude_db_at(1000.0, SAMPLE_RATE);
        assert_relative_eq!(db, 6.0, epsilon = TOLERANCE);
    }

    #[test]
    fn peak_filter_cuts_at_center_frequency() {
        // A -6 dB peak should show ~-6 dB at its center
        let coeffs = Coefficients::peak(1000.0, -6.0, 1.4, SAMPLE_RATE);
        let filter = BiquadFilter::new(coeffs);
        let db = filter.magnitude_db_at(1000.0, SAMPLE_RATE);
        assert_relative_eq!(db, -6.0, epsilon = TOLERANCE);
    }

    #[test]
    fn peak_filter_is_flat_far_from_center() {
        // A peak at 1 kHz should barely affect 20 Hz (very far away)
        let coeffs = Coefficients::peak(1000.0, 12.0, 1.4, SAMPLE_RATE);
        let filter = BiquadFilter::new(coeffs);
        let db = filter.magnitude_db_at(20.0, SAMPLE_RATE);
        // Should be nearly 0 dB (flat) at 20 Hz
        assert_relative_eq!(db, 0.0, epsilon = 1.0); // 1 dB tolerance — it's far away
    }

    #[test]
    fn low_pass_attenuates_above_cutoff() {
        // A low-pass at 1 kHz should heavily attenuate 10 kHz
        let coeffs = Coefficients::low_pass(1000.0, 0.707, SAMPLE_RATE);
        let filter = BiquadFilter::new(coeffs);
        let db_above = filter.magnitude_db_at(10_000.0, SAMPLE_RATE);
        assert!(db_above < -20.0, "Expected heavy attenuation above cutoff, got {:.1} dB", db_above);
    }

    #[test]
    fn high_pass_attenuates_below_cutoff() {
        // A high-pass at 10 kHz should heavily attenuate 100 Hz
        let coeffs = Coefficients::high_pass(10_000.0, 0.707, SAMPLE_RATE);
        let filter = BiquadFilter::new(coeffs);
        let db_below = filter.magnitude_db_at(100.0, SAMPLE_RATE);
        assert!(db_below < -20.0, "Expected heavy attenuation below cutoff, got {:.1} dB", db_below);
    }

    #[test]
    fn notch_deeply_cuts_at_target_frequency() {
        // A notch at 1 kHz with Q=10 should cut very deeply there
        let coeffs = Coefficients::notch(1000.0, 10.0, SAMPLE_RATE);
        let filter = BiquadFilter::new(coeffs);
        let db = filter.magnitude_db_at(1000.0, SAMPLE_RATE);
        assert!(db < -40.0, "Expected deep notch cut, got {:.1} dB", db);
    }

    #[test]
    fn low_shelf_boosts_at_low_frequency() {
        // A +6 dB low shelf at 200 Hz should boost at 50 Hz (well within shelf)
        let coeffs = Coefficients::low_shelf(200.0, 6.0, 0.707, SAMPLE_RATE);
        let filter = BiquadFilter::new(coeffs);
        let db = filter.magnitude_db_at(50.0, SAMPLE_RATE);
        assert!(db > 4.0, "Expected low shelf boost at 50 Hz, got {:.1} dB", db);
    }

    #[test]
    fn high_shelf_boosts_at_high_frequency() {
        // A +6 dB high shelf at 8 kHz should boost at 18 kHz
        let coeffs = Coefficients::high_shelf(8000.0, 6.0, 0.707, SAMPLE_RATE);
        let filter = BiquadFilter::new(coeffs);
        let db = filter.magnitude_db_at(18_000.0, SAMPLE_RATE);
        assert!(db > 4.0, "Expected high shelf boost at 18 kHz, got {:.1} dB", db);
    }

    #[test]
    fn update_coefficients_does_not_reset_state() {
        let coeffs = Coefficients::peak(1000.0, 6.0, 1.4, SAMPLE_RATE);
        let mut filter = BiquadFilter::new(coeffs);

        // Prime the delay line with some samples
        filter.process_sample(0.5);
        filter.process_sample(-0.3);
        let y1_before = filter.y1;

        // Update coefficients — state should be untouched
        filter.update_coefficients(Coefficients::peak(2000.0, 3.0, 1.4, SAMPLE_RATE));
        assert_relative_eq!(filter.y1, y1_before, epsilon = 1e-10);
    }

    #[test]
    fn from_band_returns_passthrough_when_disabled() {
        let mut band = BandConfig::new(FilterType::Peak, 1000.0);
        band.gain_db = 12.0;
        band.enabled = false; // disabled — should bypass

        let coeffs = Coefficients::from_band(&band, SAMPLE_RATE);
        assert_eq!(coeffs, Coefficients::passthrough());
    }
}
