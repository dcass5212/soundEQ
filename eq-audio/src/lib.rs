// =============================================================================
// lib.rs — eq-audio crate root
//
// eq-audio is the Windows audio I/O layer. It depends on eq-core for DSP
// and on windows-rs for WASAPI.
//
// Public API surface:
//   - LoopbackCapture      — WASAPI loopback capture (Step 2a)
//   - WasapiRenderer       — WASAPI render output (Step 2b)
//   - RenderSender         — write handle for pushing audio into the render queue
//   - StreamFormat         — sample rate + channel count of a WASAPI stream
//   - AudioDeviceInfo      — id + name + is_default for one audio endpoint
//   - list_render_devices  — enumerate active output devices (Step 2b)
//   - AudioError           — all audio I/O error variants
//
// Future modules (Step 2c):
//   - session              — detect per-app audio sessions via IAudioSessionManager
// =============================================================================

pub mod error;
pub mod capture;
pub mod render;
pub mod device;
pub mod session;
pub mod spectrum;
pub mod volume;

pub use error::AudioError;
pub use capture::{LoopbackCapture, StreamFormat};
pub use render::{WasapiRenderer, RenderSender};
pub use device::{AudioDeviceInfo, list_render_devices};
pub use session::{AudioSessionInfo, list_audio_sessions, get_foreground_process_name, set_process_volume};
pub use spectrum::{SpectrumAnalyzer, SPECTRUM_BANDS};
pub use volume::VolumeMonitor;
