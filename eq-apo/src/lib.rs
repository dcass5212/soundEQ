// eq-apo/src/lib.rs
//
// Windows Audio Processing Object (APO) DLL for soundEQ.
//
// An APO is a user-mode COM DLL that Windows loads into the audiodg.exe
// process (the audio engine) before the audio reaches your speakers. By
// sitting inside the Windows audio pipeline we receive audio *after*
// endpoint volume is applied, so system volume works correctly — unlike
// the WASAPI loopback approach where we capture pre-volume audio.
//
// Phase 1: COM shell that compiles and registers cleanly. All audio
//           processing is passthrough — no DSP yet. This proves the DLL
//           loads successfully into audiodg.exe.
// Phase 2: Wire in eq-core FilterChain; read active profile from the
//           same JSON file that the Tauri app writes.
//
// Exported symbols required by the COM loader:
//   DllGetClassObject   — returns IClassFactory for our CLSID
//   DllCanUnloadNow     — tells COM whether it's safe to unload us
//
// CLSID: {8C2A5F3E-B47D-4A1C-9E8F-D0C3B6A2E1F4}
// This GUID is unique to soundEQ. It must match the value in register.ps1.

#![allow(non_snake_case)]

mod apo;
mod factory;

use windows::{
    core::{Interface, GUID, HRESULT, IUnknown},
    Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, S_FALSE, S_OK},
    Win32::System::Com::IClassFactory,
};

use factory::ApoFactory;

// The COM class identifier for SoundEqApo. Must match register.ps1.
pub const CLSID_SOUND_EQ_APO: GUID = GUID {
    data1: 0x8C2A5F3E,
    data2: 0xB47D,
    data3: 0x4A1C,
    data4: [0x9E, 0x8F, 0xD0, 0xC3, 0xB6, 0xA2, 0xE1, 0xF4],
};

// COM reference count for DllCanUnloadNow.
// windows-rs manages per-object ref-counts internally; this global tracks
// whether *any* live COM object still exists in our DLL.
static OBJECTS_ALIVE: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

pub fn increment_object_count() {
    OBJECTS_ALIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}
pub fn decrement_object_count() {
    OBJECTS_ALIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

// ─── DLL load hook ──────────────────────────────────────────────────────────
// DllMain runs the instant Windows loads our DLL into any process.
//
// TWO diagnostics run here so we can distinguish the two possible failure modes:
//
//   A. DLL is not loaded at all:
//      Neither OutputDebugStringA nor dllmain.log ever appears.
//      → code signing rejected — audiodg.exe drops the DLL before calling DllMain.
//
//   B. DLL loads but COM/Initialize is never called:
//      OutputDebugStringA and dllmain.log appear, but apo_debug.log does not.
//      → registration issue (CLSID lookup, interface negotiation).
//
// OutputDebugStringA is safe under the Windows loader lock (it only calls into
// kernel32.dll which is always fully initialized before our DLL attaches).
// Capture its output with SysInternals DebugView (run as Admin, enable
// "Capture Kernel" + "Capture Win32"). No file permissions needed.
//
// Remove both diagnostics after load is confirmed.

extern "system" {
    fn OutputDebugStringA(lp_output_string: *const core::ffi::c_char);
}

#[no_mangle]
pub unsafe extern "system" fn DllMain(
    _hinstance: *mut core::ffi::c_void,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        OutputDebugStringA(b"[soundEQ] DllMain DLL_PROCESS_ATTACH\0".as_ptr() as _);

        // Include the PID so we can tell whether audiodg.exe or AudioEndpointBuilder
        // loaded us — both services cycle on an audio restart and both load APO DLLs.
        extern "system" { fn GetCurrentProcessId() -> u32; }
        let pid = GetCurrentProcessId();
        let msg = format!("DllMain DLL_PROCESS_ATTACH pid={}\n", pid);
        let _ = std::fs::write("C:\\Users\\Public\\soundEQ\\dllmain.log", msg.as_bytes());
    }
    1 // TRUE — tell Windows the DLL initialized successfully
}

// ─── DLL entry points ───────────────────────────────────────────────────────

/// Called by COM when another process (audiodg.exe) asks for an object from
/// this DLL. We only support our one CLSID; everything else gets
/// CLASS_E_CLASSNOTAVAILABLE.
///
/// # Safety
/// This is a raw Windows API callback. All pointer arguments are guaranteed
/// valid for the duration of the call by the COM loader contract.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    let clsid_match = unsafe { *rclsid } == CLSID_SOUND_EQ_APO;
    {
        use std::io::Write;
        let msg = format!("DllGetClassObject called, clsid_match={}\n", clsid_match);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
            .open("C:\\Users\\Public\\soundEQ\\apo_trace.log")
        {
            let _ = f.write_all(msg.as_bytes());
        }
    }

    if !clsid_match {
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    let factory: IClassFactory = ApoFactory::new().into();
    let unk: IUnknown = factory.into();
    let hr = unsafe { unk.query(riid, ppv) };
    {
        use std::io::Write;
        let msg = format!("DllGetClassObject QI result={:?}\n", hr);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
            .open("C:\\Users\\Public\\soundEQ\\apo_trace.log")
        {
            let _ = f.write_all(msg.as_bytes());
        }
    }
    hr
}

/// Called by COM to ask whether the DLL can be unloaded. Return S_OK (0)
/// when no live COM objects remain; S_FALSE (1) when objects still exist.
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS_ALIVE.load(std::sync::atomic::Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}
