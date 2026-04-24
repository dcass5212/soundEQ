// =============================================================================
// session.rs — Per-app audio session detection
//
// WHAT THIS DOES:
// Enumerates all active WASAPI audio sessions on the default render endpoint
// and maps each session's process ID to a human-readable executable name.
// This is the mechanism soundEQ uses to route per-app EQ profiles: when
// "spotify.exe" or "chrome.exe" is detected producing audio, its assigned
// profile is applied instead of the global default.
//
// HOW WASAPI SESSIONS WORK:
// Windows groups every audio stream into a "session" — a logical container
// for streams from the same application or Windows subsystem. Each session
// carries a process ID (PID) that created it. IAudioSessionManager2 exposes
// a snapshot enumerator over all sessions currently alive on a given device.
//
// FLOW:
//   IMMDeviceEnumerator → default render device
//   device.Activate::<IAudioSessionManager2>
//   manager.GetSessionEnumerator → IAudioSessionEnumerator (snapshot)
//   for each session: IAudioSessionControl → cast to IAudioSessionControl2
//     GetProcessId → PID → QueryFullProcessImageNameW → "chrome.exe"
//     GetSessionIdentifier → opaque session ID string
//
// API OVERVIEW:
//   - AudioSessionInfo      — pid + process_name + session_id for one session
//   - list_audio_sessions() → Vec<AudioSessionInfo> of all active sessions
// =============================================================================

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{
    IAudioSessionControl2, IAudioSessionEnumerator, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::core::{HRESULT, Interface, PWSTR};

use crate::error::AudioError;

// ---------------------------------------------------------------------------
// AudioSessionInfo
// ---------------------------------------------------------------------------

/// Describes one active WASAPI audio session on the default render device.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioSessionInfo {
    /// Windows process ID that created this session.
    /// PID 0 indicates a system-level or cross-process session (no user app).
    pub pid: u32,

    /// Executable file name only (e.g. "chrome.exe" or "spotify.exe").
    /// Empty when the process cannot be opened (e.g. system processes, PID 0).
    pub process_name: String,

    /// Opaque WASAPI session identifier string. Unique per session instance;
    /// useful for correlating the same session across successive calls.
    pub session_id: String,
}

// ---------------------------------------------------------------------------
// list_audio_sessions
// ---------------------------------------------------------------------------

/// Returns all active WASAPI audio sessions on the default render endpoint.
///
/// Sessions with PID 0 (system/cross-process sessions) are included but will
/// have an empty `process_name`. Filter these out if only user-app sessions
/// are needed.
pub fn list_audio_sessions() -> Result<Vec<AudioSessionInfo>, AudioError> {
    // ComGuard is declared first so it is dropped last — after all COM
    // interface pointers have been released — to satisfy COM lifetime rules.
    let _com = ComGuard::init()?;

    // Reach the default render device; sessions are per-device.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };

    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? };

    // IAudioSessionManager2 is the session-management service interface.
    // Activate() is how WASAPI service interfaces are retrieved from a device.
    let manager: IAudioSessionManager2 =
        unsafe { device.Activate(CLSCTX_ALL, None)? };

    // GetSessionEnumerator returns a point-in-time snapshot of all sessions.
    // New sessions that start after this call are not included.
    let session_enum: IAudioSessionEnumerator =
        unsafe { manager.GetSessionEnumerator()? };

    let count = unsafe { session_enum.GetCount()? };
    let mut sessions = Vec::with_capacity(count as usize);

    for i in 0..count {
        // GetSession yields IAudioSessionControl; we need IAudioSessionControl2
        // for GetProcessId() and GetSessionIdentifier(), which are extensions.
        let ctrl = unsafe { session_enum.GetSession(i)? };
        let ctrl2: IAudioSessionControl2 = ctrl.cast()?;

        // GetProcessId can fail for system-level sessions — treat those as PID 0.
        let pid = unsafe { ctrl2.GetProcessId().unwrap_or(0) };

        // GetSessionIdentifier allocates its string with CoTaskMem; we must free it.
        let session_id = unsafe { read_session_id(&ctrl2) };

        let process_name = if pid == 0 {
            // The System Idle Process (PID 0) cannot be opened — skip name lookup.
            String::new()
        } else {
            process_name_from_pid(pid).unwrap_or_default()
        };

        sessions.push(AudioSessionInfo { pid, process_name, session_id });
    }

    Ok(sessions)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Reads and frees the session identifier string from IAudioSessionControl2.
///
/// # Safety
/// `ctrl2` must be a valid IAudioSessionControl2.
unsafe fn read_session_id(ctrl2: &IAudioSessionControl2) -> String {
    match ctrl2.GetSessionIdentifier() {
        Ok(ptr) => {
            let s = pwstr_to_string(ptr);
            // GetSessionIdentifier allocates via CoTaskMem — caller must free.
            CoTaskMemFree(Some(ptr.0.cast()));
            s
        }
        Err(_) => String::new(),
    }
}

/// Returns the executable file name (e.g. "chrome.exe") for a given PID.
///
/// Uses `PROCESS_QUERY_LIMITED_INFORMATION` — the least-privileged access
/// right that still allows `QueryFullProcessImageName`. This works for any
/// process owned by the current user without administrator rights.
///
/// Returns `None` if the process cannot be opened (access denied, already
/// exited) or if the name cannot be retrieved.
fn process_name_from_pid(pid: u32) -> Option<String> {
    // SAFETY: OpenProcess is an OS call. We check the return value and always
    // close the handle before returning.
    let handle = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?
    };

    let mut buf = [0u16; 260]; // MAX_PATH = 260 characters
    let mut len = 260u32;

    // QueryFullProcessImageNameW fills `buf` with the full Win32 path and
    // sets `len` to the number of characters written (excluding null).
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32, // Win32 path format, not NT device path
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok()
    };

    // Always close the process handle regardless of success.
    unsafe { let _ = CloseHandle(handle); }

    if !ok || len == 0 {
        return None;
    }

    // Full path is e.g. "C:\Program Files\Google\Chrome\Application\chrome.exe".
    // We only want the filename component.
    let full_path = String::from_utf16_lossy(&buf[..len as usize]);
    let name = full_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_string();

    if name.is_empty() { None } else { Some(name) }
}

/// Converts a null-terminated UTF-16 wide string pointer to an owned Rust String.
///
/// # Safety
/// `ptr` must be null or a valid pointer to a null-terminated u16 sequence.
unsafe fn pwstr_to_string(ptr: PWSTR) -> String {
    if ptr.0.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *ptr.0.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr.0, len))
}

// ---------------------------------------------------------------------------
// COM guard
// ---------------------------------------------------------------------------

// RPC_E_CHANGED_MODE: same situation as device.rs — Tauri/WebView2 initialises
// the IPC thread as STA before our code runs. We can still use COM; we just
// don't own this initialisation and must not call CoUninitialize on drop.
const RPC_E_CHANGED_MODE: HRESULT = HRESULT(0x80010106u32 as i32);

struct ComGuard {
    should_uninit: bool,
}

impl ComGuard {
    fn init() -> Result<Self, AudioError> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Self { should_uninit: false });
        }
        hr.ok()?;
        Ok(Self { should_uninit: true })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.should_uninit {
            unsafe { CoUninitialize() };
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires audio hardware and Windows audio service"]
    fn list_audio_sessions_does_not_error() {
        let sessions = list_audio_sessions().expect("list_audio_sessions failed");
        println!("found {} session(s)", sessions.len());
        for s in &sessions {
            println!("  pid={} name={:?} id={}", s.pid, s.process_name, s.session_id);
        }
    }

    #[test]
    #[ignore = "requires audio hardware and Windows audio service"]
    fn non_zero_pid_sessions_have_process_name() {
        for s in list_audio_sessions().expect("list_audio_sessions failed") {
            if s.pid != 0 {
                assert!(
                    !s.process_name.is_empty(),
                    "pid {} has empty process_name",
                    s.pid
                );
            }
        }
    }

    #[test]
    #[ignore = "requires audio hardware and Windows audio service"]
    fn process_name_is_exe_only_not_full_path() {
        for s in list_audio_sessions().expect("list_audio_sessions failed") {
            if !s.process_name.is_empty() {
                assert!(
                    !s.process_name.contains('\\'),
                    "process_name {:?} looks like a full path",
                    s.process_name
                );
            }
        }
    }
}
