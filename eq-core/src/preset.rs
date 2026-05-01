// =============================================================================
// preset.rs — Built-in EQ presets
//
// Presets are factory-supplied Profile instances that ship inside the binary.
// They are NOT stored on disk; they are reconstructed from code on every
// startup. This means they are always available even on a fresh install, and
// they cannot be corrupted by a bad JSON write.
//
// The Tauri layer loads these at startup and inserts them into the ProfileStore
// alongside any user-created profiles. The user can duplicate a preset, rename
// the copy, and edit it — the originals remain unchanged.
//
// EQ DESIGN NOTES:
// All frequency/gain values are based on standard audio engineering practice.
// Frequencies are in Hz, gains in dB, Q follows the parametric EQ convention
// where Q = center_frequency / bandwidth.
// =============================================================================

use crate::filter_type::{BandConfig, FilterType};
use crate::profile::Profile;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns all built-in presets as owned Profile instances.
///
/// The list is ordered from most general (Flat) to most specific, which is
/// the order the UI should display them in the preset picker.
pub fn builtin_presets() -> Vec<Profile> {
    vec![
        flat(),
        bass_boost(),
        treble_boost(),
        vocal_clarity(),
        gaming(),
        classical(),
        lo_fi(),
    ]
}

// ---------------------------------------------------------------------------
// Preset definitions
//
// Each function returns a Profile. Band parameters are commented so a reader
// can understand the audio rationale without needing to hear it.
// ---------------------------------------------------------------------------

/// Flat — no EQ applied. The identity preset.
///
/// Having a named flat preset lets users assign it to specific apps to
/// explicitly suppress the default profile, rather than having to remove
/// the app assignment entirely.
fn flat() -> Profile {
    Profile::new("Flat")
}

/// Bass Boost — lifts the low end for more warmth and impact.
///
/// Typical use: music listening, movies, speakers that lack bass extension.
fn bass_boost() -> Profile {
    Profile::with_bands("Bass Boost", vec![
        // Sub-bass shelf: adds body and rumble.
        // Low shelf boosts everything below the cutoff frequency, so all
        // the sub-bass region (20–80 Hz) gets lifted together.
        band(FilterType::LowShelf, 80.0,   5.0, 0.707),

        // Upper-bass peak: adds punch and kick-drum presence.
        // Narrower than the shelf to avoid making the mix sound muddy.
        band(FilterType::Peak,     200.0,  3.0, 1.0),
    ])
}

/// Treble Boost — adds brightness and air to dull-sounding audio.
///
/// Typical use: headphones that roll off in the highs, acoustic recordings.
fn treble_boost() -> Profile {
    Profile::with_bands("Treble Boost", vec![
        // Presence peak: brings out consonants, guitar pick attack, cymbals.
        band(FilterType::Peak,      5_000.0, 3.0, 1.4),

        // Air shelf: adds sparkle above 10 kHz — the "airy" quality in
        // well-recorded vocals and acoustic instruments.
        band(FilterType::HighShelf, 10_000.0, 4.0, 0.707),
    ])
}

/// Vocal Clarity — brings vocals forward in the mix.
///
/// Typical use: voice calls, podcasts, games with heavy voice chat,
/// music where vocals are sitting too far back in the mix.
fn vocal_clarity() -> Profile {
    Profile::with_bands("Vocal Clarity", vec![
        // Remove low-frequency rumble (room noise, desk vibration, plosives).
        // High-pass at 100 Hz — rolls off everything below without affecting
        // the body of most voices (fundamental starts around 85–180 Hz for men,
        // 165–255 Hz for women, but the body of speech sits above 150 Hz).
        band(FilterType::HighPass, 100.0,   0.0, 0.707),

        // Cut the "boxy" or "nasal" region that makes voices sound muffled.
        band(FilterType::Peak,     300.0,  -2.0, 1.0),

        // Presence boost: the 2–5 kHz range is where speech intelligibility
        // lives. Boosting here makes words clearer, especially over headphones.
        band(FilterType::Peak,     3_000.0, 3.0, 1.4),

        // Clarity peak: adds definition to sibilants (s, t, sh sounds).
        band(FilterType::Peak,     6_000.0, 2.0, 2.0),
    ])
}

/// Gaming — optimized for positional audio awareness in games.
///
/// Boosts the frequency ranges that carry footsteps, voice callouts, and
/// directional cues. Pulls back the mid-bass that can mask these details.
fn gaming() -> Profile {
    Profile::with_bands("Gaming", vec![
        // Reduce mid-bass mud that masks directional audio cues.
        // The 200–400 Hz range can build up and make soundscapes feel congested.
        band(FilterType::Peak,      250.0,  -3.0, 0.8),

        // Boost voice presence — keeps teammate callouts cutting through
        // even when ambient game audio is loud.
        band(FilterType::Peak,      3_000.0, 3.0, 1.4),

        // Footstep range: shoes on different surfaces have transients here.
        // A modest boost helps hear movement around corners.
        band(FilterType::Peak,      8_000.0, 2.0, 1.4),

        // High shelf for openness and spatial width.
        band(FilterType::HighShelf, 12_000.0, 2.0, 0.707),
    ])
}

/// Classical — a gentle, wide curve for orchestral and acoustic music.
///
/// Adds warmth in the low strings, presence in the strings and woodwinds,
/// and air for a concert-hall sense of space. Nothing sharp or aggressive.
fn classical() -> Profile {
    Profile::with_bands("Classical", vec![
        // Gentle low shelf for cello/bass warmth without excessive boom.
        band(FilterType::LowShelf,  120.0,  2.0, 0.707),

        // Very subtle mid dip to open up the soundstage slightly.
        band(FilterType::Peak,      500.0, -1.0, 0.5),

        // String presence — violins and violas live in this range.
        band(FilterType::Peak,      4_000.0, 1.5, 1.4),

        // Cymbal/triangle sparkle and hall air.
        band(FilterType::HighShelf, 10_000.0, 2.0, 0.707),
    ])
}

/// Lo-Fi — warm, slightly degraded vintage character.
///
/// Rolls off the high end (like old tape or vinyl), adds a touch of
/// low-mid warmth. Used intentionally for aesthetic effect.
fn lo_fi() -> Profile {
    Profile::with_bands("Lo-Fi", vec![
        // Cut sub-bass below 60 Hz — vintage gear couldn't reproduce it.
        band(FilterType::HighPass,  60.0,   0.0, 0.707),

        // Low-mid warmth: the "vinyl" body that lo-fi listeners expect.
        band(FilterType::LowShelf,  200.0,  3.0, 0.707),

        // Roll off the top end like a worn-out tape head would.
        // The gradual high-shelf cut starts at 6 kHz and reduces everything
        // above, dulling harsh digital transients.
        band(FilterType::HighShelf, 6_000.0, -6.0, 0.707),
    ])
}

// ---------------------------------------------------------------------------
// Helper — builds a BandConfig with all parameters explicit
//
// This is an internal helper used only by the preset definitions above.
// Using a helper instead of struct literal syntax means if BandConfig gains
// a new field in the future, only this one function needs updating.
// ---------------------------------------------------------------------------
fn band(filter_type: FilterType, frequency: f64, gain_db: f64, q: f64) -> BandConfig {
    BandConfig {
        filter_type,
        frequency,
        gain_db,
        q,
        enabled: true,
        color: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_presets_have_valid_bands() {
        for preset in builtin_presets() {
            preset.validate().unwrap_or_else(|e| {
                panic!("preset '{}' failed validation: {}", preset.name, e);
            });
        }
    }

    #[test]
    fn builtin_presets_have_unique_names() {
        let presets = builtin_presets();
        let mut names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        let original_len = names.len();
        names.dedup();
        // If dedup removed any entries, there were duplicates
        assert_eq!(names.len(), original_len, "duplicate preset names found");
    }

    #[test]
    fn flat_preset_has_no_bands() {
        let f = flat();
        assert_eq!(f.bands.len(), 0);
    }

    #[test]
    fn bass_boost_lifts_low_frequencies() {
        // The Bass Boost preset should produce positive gain at 80 Hz
        let preset = bass_boost();
        let chain = preset.to_filter_chain(48_000.0);
        let db_at_80hz = chain.magnitude_db_at(80.0);
        assert!(db_at_80hz > 0.0, "Bass Boost should lift 80 Hz, got {:.2} dB", db_at_80hz);
    }

    #[test]
    fn treble_boost_lifts_high_frequencies() {
        let preset = treble_boost();
        let chain = preset.to_filter_chain(48_000.0);
        let db_at_10khz = chain.magnitude_db_at(10_000.0);
        assert!(db_at_10khz > 0.0, "Treble Boost should lift 10 kHz, got {:.2} dB", db_at_10khz);
    }

    #[test]
    fn vocal_clarity_attenuates_below_highpass() {
        // The high-pass at 100 Hz should significantly reduce very low frequencies
        let preset = vocal_clarity();
        let chain = preset.to_filter_chain(48_000.0);
        let db_at_30hz = chain.magnitude_db_at(30.0);
        assert!(db_at_30hz < -6.0, "Vocal Clarity should attenuate 30 Hz, got {:.2} dB", db_at_30hz);
    }

    #[test]
    fn lo_fi_cuts_high_frequencies() {
        let preset = lo_fi();
        let chain = preset.to_filter_chain(48_000.0);
        let db_at_15khz = chain.magnitude_db_at(15_000.0);
        assert!(db_at_15khz < -3.0, "Lo-Fi should cut 15 kHz, got {:.2} dB", db_at_15khz);
    }

    #[test]
    fn preset_count_matches_expectation() {
        // Update this if you add or remove presets — it acts as a guard
        // against accidentally deleting a preset from the builtin_presets() list.
        assert_eq!(builtin_presets().len(), 7);
    }
}
