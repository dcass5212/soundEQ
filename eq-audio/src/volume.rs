// eq-audio/src/volume.rs
//
// VolumeMonitor — mirrors the Windows system volume slider into the render path.
//
// WHY THIS EXISTS:
// In the VB-Cable route, soundEQ captures audio via WASAPI loopback on the
// virtual cable device (the system default output). WASAPI loopback taps the
// stream *before* the endpoint's volume stage is applied, so the Windows volume
// slider has no effect on what soundEQ outputs to the real speakers.
//
// This module polls the endpoint's IAudioEndpointVolume every 50 ms and writes
// the scalar (0.0 = muted, 1.0 = full) into a shared AtomicU32. The render
// loop reads that atomic and multiplies every sample by it, making the slider
// work transparently.
//
// 50 ms polling is fast enough to feel instant (~1 frame at 60 Hz) while being
// cheap — one COM call per poll, no allocation.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::BOOL;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::core::PCWSTR;

/// Polls the Windows endpoint volume of the VB-Cable (or any render) device
/// and writes the current scalar into a shared `AtomicU32`.
///
/// The render thread reads the atomic every buffer and multiplies samples by it.
pub struct VolumeMonitor {
    stop_flag: Arc<AtomicBool>,
    thread_handle: Option<JoinHandle<()>>,
}

impl VolumeMonitor {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        }
    }

    /// Starts polling the volume of `device_id`.
    ///
    /// `device_id` should be the WASAPI capture device ID (the VB-Cable / virtual
    /// cable endpoint). `volume` is the AtomicU32 the render loop reads from —
    /// obtain it via `WasapiRenderer::volume_handle()` before calling `start()`.
    ///
    /// If the device cannot be opened (e.g. device removed), the monitor exits
    /// silently and `volume` stays at its default of 1.0 (full).
    pub fn start(&mut self, device_id: Option<String>, volume: Arc<AtomicU32>) {
        if self.thread_handle.is_some() {
            return;
        }
        self.stop_flag.store(false, Ordering::Relaxed);
        let stop = Arc::clone(&self.stop_flag);

        self.thread_handle = thread::Builder::new()
            .name("volume-monitor".into())
            .spawn(move || volume_thread(stop, volume, device_id))
            .ok();
    }

    /// Signals the polling thread to stop and waits for it.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread_handle.take() {
            let _ = h.join();
        }
    }
}

impl Default for VolumeMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for VolumeMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

// ─── Thread body ─────────────────────────────────────────────────────────────

fn volume_thread(stop: Arc<AtomicBool>, volume: Arc<AtomicU32>, device_id: Option<String>) {
    // COM must be initialised per-thread. On failure, leave volume at 1.0 and exit.
    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
        return;
    }

    if let Some(vc) = open_endpoint_volume(device_id.as_deref()) {
        while !stop.load(Ordering::Relaxed) {
            let scalar = read_volume(&vc);
            volume.store(scalar.to_bits(), Ordering::Relaxed);
            thread::sleep(Duration::from_millis(50));
        }
    }
    // If the device couldn't be opened, volume stays at 1.0 (the renderer's default).

    unsafe { CoUninitialize() };
}

/// Opens `IAudioEndpointVolume` for the given render endpoint ID.
/// Falls back to the system-default render device if `device_id` is None.
fn open_endpoint_volume(device_id: Option<&str>) -> Option<IAudioEndpointVolume> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = if let Some(id) = device_id {
            let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
            enumerator.GetDevice(PCWSTR(wide.as_ptr())).ok()?
        } else {
            enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?
        };
        device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

/// Reads the effective volume scalar. Returns 0.0 if muted, 1.0 on any error.
fn read_volume(vc: &IAudioEndpointVolume) -> f32 {
    unsafe {
        let muted: BOOL = vc.GetMute().unwrap_or(BOOL(0));
        if muted.as_bool() {
            return 0.0;
        }
        // GetMasterVolumeLevelScalar returns [0.0, 1.0]. Clamp defensively.
        vc.GetMasterVolumeLevelScalar().unwrap_or(1.0).clamp(0.0, 1.0)
    }
}
