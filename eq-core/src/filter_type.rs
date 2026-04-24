// =============================================================================
// filter_type.rs — EQ band configuration types
//
// Defines what a single EQ band looks like from the user's perspective.
// These are the "settings" the frontend will send to the DSP engine.
// The DSP engine then converts these settings into biquad math coefficients.
// =============================================================================

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// FilterType — the shape/behavior of one EQ band
//
// Each variant corresponds to a different biquad coefficient formula.
// Think of these as different "modes" for a single band.
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterType {
    /// Peak (bell curve) — boosts or cuts around a center frequency.
    /// The Q controls how wide or narrow the bell is.
    /// Most common filter type — used for surgical cuts and boosts.
    Peak,

    /// Low shelf — boosts or cuts ALL frequencies below the cutoff.
    /// Classic "bass boost" shape.
    LowShelf,

    /// High shelf — boosts or cuts ALL frequencies above the cutoff.
    /// Classic "treble boost" shape.
    HighShelf,

    /// Low-pass — allows frequencies BELOW the cutoff through, blocks above.
    /// Used to remove harshness, hiss, or high-frequency noise.
    LowPass,

    /// High-pass — allows frequencies ABOVE the cutoff through, blocks below.
    /// Used to remove rumble, low-end mud, or mic handling noise.
    HighPass,

    /// Notch — a very narrow, deep cut at a specific frequency.
    /// Q should be high (narrow). Used to kill a specific problem frequency
    /// like a fan whine (e.g., 60 Hz hum) or a resonance peak.
    Notch,

    /// Bandpass — only allows a narrow band of frequencies through.
    /// Everything outside the band is attenuated.
    /// Useful for isolating vocals or a specific instrument range.
    Bandpass,
}

impl FilterType {
    /// Returns a human-readable display name for the UI.
    pub fn display_name(&self) -> &'static str {
        match self {
            FilterType::Peak      => "Peak",
            FilterType::LowShelf  => "Low Shelf",
            FilterType::HighShelf => "High Shelf",
            FilterType::LowPass   => "Low Pass",
            FilterType::HighPass  => "High Pass",
            FilterType::Notch     => "Notch",
            FilterType::Bandpass  => "Bandpass",
        }
    }

    /// Returns whether gain is relevant for this filter type.
    /// Low-pass, high-pass, notch, and bandpass are gain-neutral —
    /// they don't boost or cut, they just pass or block.
    /// The UI should hide the gain slider when this returns false.
    pub fn uses_gain(&self) -> bool {
        matches!(self, FilterType::Peak | FilterType::LowShelf | FilterType::HighShelf)
    }
}

// ---------------------------------------------------------------------------
// BandConfig — the complete settings for one EQ band
//
// This is what gets stored in a preset/profile JSON file,
// and what the user edits in the UI.
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BandConfig {
    /// Which filter shape this band uses
    pub filter_type: FilterType,

    /// Center/cutoff frequency in Hz.
    /// Valid range: 20 Hz (lowest audible) to 20,000 Hz (highest audible).
    /// Stored as f64 for precision; audio math works in f64 throughout.
    pub frequency: f64,

    /// Gain in decibels (dB). Only meaningful for Peak, LowShelf, HighShelf.
    /// Typical range: -24.0 to +24.0 dB.
    /// 0.0 = no change (flat/bypass for this band).
    pub gain_db: f64,

    /// Q factor — controls bandwidth/resonance.
    ///
    /// For Peak: Q = center_freq / bandwidth. Higher Q = narrower bell.
    ///   - Q 0.7 = very wide (musical, gentle)
    ///   - Q 1.4 = medium (typical parametric EQ starting point)
    ///   - Q 10+ = surgical narrow cut
    ///
    /// For shelves: Q affects the slope steepness.
    ///   - Q 0.707 = Butterworth (maximally flat, no overshoot)
    ///
    /// For Low/High Pass: Q controls resonance at the cutoff.
    ///   - Q 0.707 = Butterworth (smooth rolloff)
    ///   - Q > 1.0 = resonant peak at cutoff (can sound dramatic)
    ///
    /// For Notch/Bandpass: Q controls how narrow the notch/band is.
    pub q: f64,

    /// Whether this band is active. Disabled bands are skipped in the chain,
    /// which is more efficient than processing a flat filter.
    pub enabled: bool,
}

impl BandConfig {
    /// Constructs a new band with sensible defaults.
    /// `frequency` is in Hz, `filter_type` sets the shape.
    pub fn new(filter_type: FilterType, frequency: f64) -> Self {
        // Default Q of 1.4 is a good musical starting point for parametric EQ.
        // Default gain of 0.0 dB means no change until the user adjusts it.
        Self {
            filter_type,
            frequency,
            gain_db: 0.0,
            q: 1.4,
            enabled: true,
        }
    }

    /// Validates that all parameter values are within safe/sensible ranges.
    /// Returns Err with a description if anything is out of range.
    pub fn validate(&self) -> Result<(), String> {
        if self.frequency < 20.0 || self.frequency > 20_000.0 {
            return Err(format!(
                "Frequency {} Hz is outside the audible range (20–20,000 Hz)",
                self.frequency
            ));
        }
        if self.gain_db < -24.0 || self.gain_db > 24.0 {
            return Err(format!(
                "Gain {:.1} dB is outside the allowed range (-24 to +24 dB)",
                self.gain_db
            ));
        }
        // Q must be positive and non-zero to avoid division by zero in the math
        if self.q <= 0.0 {
            return Err(format!("Q must be positive, got {}", self.q));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_config_defaults_are_valid() {
        let band = BandConfig::new(FilterType::Peak, 1000.0);
        assert!(band.validate().is_ok());
    }

    #[test]
    fn frequency_out_of_range_fails_validation() {
        let mut band = BandConfig::new(FilterType::Peak, 1000.0);
        band.frequency = 10.0; // below 20 Hz — inaudible / dangerous for speakers
        assert!(band.validate().is_err());
    }

    #[test]
    fn gain_uses_gain_filter_types() {
        assert!(FilterType::Peak.uses_gain());
        assert!(FilterType::LowShelf.uses_gain());
        assert!(!FilterType::LowPass.uses_gain());
        assert!(!FilterType::Notch.uses_gain());
    }

    #[test]
    fn serde_roundtrip() {
        let band = BandConfig::new(FilterType::HighShelf, 8000.0);
        let json = serde_json::to_string(&band).unwrap();
        let back: BandConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(band, back);
    }
}
