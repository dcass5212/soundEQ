// =============================================================================
// error.rs — AudioError type for the eq-audio crate
//
// All public-facing functions in eq-audio return Result<_, AudioError>.
// Using thiserror lets us write clean match arms in the Tauri command
// handlers (Step 3) without boilerplate.
// =============================================================================

use thiserror::Error;

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
