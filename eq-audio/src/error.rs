// =============================================================================
// error.rs — AudioError type for the eq-audio crate
//
// All public-facing functions in eq-audio return Result<_, AudioError>.
// Using thiserror lets us write clean match arms in the Tauri command
// handlers (Step 3) without boilerplate.
// =============================================================================

use thiserror::Error;
use windows::Win32::Media::Audio::AUDCLNT_E_DEVICE_IN_USE;

/// All the ways an audio operation can fail.
#[derive(Debug, Error)]
pub enum AudioError {
    /// A Windows API call returned a failure HRESULT.
    /// The inner windows::core::Error carries the HRESULT code and a
    /// human-readable message from the OS.
    #[error("Windows API error: {0}")]
    Windows(#[from] windows::core::Error),

    /// The system's audio mix format is not 32-bit IEEE float.
    /// WASAPI loopback delivers whatever format the endpoint is using.
    /// On nearly all modern Windows systems this is float32, but if the
    /// user has an unusual audio driver or has manually changed their
    /// audio format this error will surface.
    ///
    /// Fix: go to Sound Settings → Advanced → choose "32-bit, 48000 Hz (Studio Quality)".
    #[error("unsupported audio format — expected 32-bit float, got: {0}")]
    UnsupportedFormat(String),

    /// The audio endpoint has a channel count we can't process.
    /// The EQ engine currently handles stereo (2-channel) audio.
    #[error("unsupported channel count: {count} (only stereo / 2-channel is supported)")]
    UnsupportedChannelCount { count: u16 },

    /// The capture and render devices are running at different sample rates.
    /// The pipeline copies samples straight from capture to render with no
    /// resampler, so mismatched rates would play audio at the wrong pitch
    /// and glitch constantly as the two sides drift apart.
    ///
    /// Fix: set both devices to the same rate — Sound Control Panel →
    /// (device) → Properties → Advanced → default format.
    #[error(
        "sample rate mismatch: capture device is {capture} Hz but output device is {render} Hz — \
         set both to the same rate in Windows Sound settings (device Properties → Advanced)"
    )]
    SampleRateMismatch { capture: u32, render: u32 },

    /// IAudioClient::Initialize returned AUDCLNT_E_DEVICE_IN_USE (0x8889000A).
    ///
    /// We only ever open devices in SHARED mode, and a shared-mode stream is
    /// refused with this code only when another process holds the endpoint in
    /// EXCLUSIVE mode (DAWs, some games and media players, and OEM audio
    /// suites like Nahimic / Sonic Studio / Dolby). If an exclusive app grabs
    /// the device mid-stream, our stream is invalidated and the auto-restart
    /// fails with this same error until that app lets go.
    ///
    /// Fix: close the app holding the device, or untick "Allow applications to
    /// take exclusive control of this device" for that endpoint.
    #[error(
        "the {device} device is being used exclusively by another application (0x8889000A) — \
         close that app, or untick \"Allow applications to take exclusive control of this device\" \
         in Sound Control Panel → (device) → Properties → Advanced"
    )]
    DeviceInUse { device: &'static str },

    /// The capture thread failed to start (OS-level thread creation failure).
    #[error("capture thread could not be started: {0}")]
    ThreadSpawn(#[from] std::io::Error),

    /// The capture thread crashed before it could report the stream format.
    /// This usually means a WASAPI error occurred during device setup.
    #[error("capture thread exited during setup — see logs for details")]
    ThreadSetupFailed,

    /// Attempted to start a capture session that is already running.
    #[error("capture is already running — call stop() before starting again")]
    AlreadyRunning,
}

impl AudioError {
    /// Converts an `IAudioClient::Initialize` failure, singling out the
    /// "device held in exclusive mode" case so the user gets an actionable
    /// message instead of a bare HRESULT. `device` names which side failed
    /// ("output" or "capture") since the fix applies per endpoint.
    pub(crate) fn from_initialize(e: windows::core::Error, device: &'static str) -> Self {
        if e.code() == AUDCLNT_E_DEVICE_IN_USE {
            AudioError::DeviceInUse { device }
        } else {
            AudioError::Windows(e)
        }
    }
}
