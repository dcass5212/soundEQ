// =============================================================================
// device.rs — Audio endpoint enumeration
//
// WHAT THIS DOES:
// Lists the audio output (render) devices available on this machine so the
// user can configure:
//   - Which device soundEQ captures from (the virtual cable the apps output to)
//   - Which device soundEQ renders the EQ'd audio to (the real speakers)
//
// The Tauri UI (Step 3) calls list_render_devices() at startup, shows the
// result in a device-picker dropdown, and persists the user's choice.
//
// API OVERVIEW:
//   - AudioDeviceInfo  — id + name + is_default for one endpoint
//   - list_render_devices() → Vec<AudioDeviceInfo> of all active render endpoints
// =============================================================================

use windows::core::{GUID, HRESULT, PROPVARIANT, PWSTR};
use windows::Win32::Media::Audio::{
    IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};

use crate::error::AudioError;

// ---------------------------------------------------------------------------
// PKEY_Device_FriendlyName
//
// The property key for the human-readable device name shown in Windows Sound
// settings. Defined manually to avoid adding the Win32_Devices_FunctionDiscovery
// feature, since the GUID is stable and well-documented.
//
// GUID: {A45C254E-DF1C-4EFD-8020-67D146A850E0}, pid = 14
// ---------------------------------------------------------------------------
const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID {
        data1: 0xa45c254e,
        data2: 0xdf1c,
        data3: 0x4efd,
        data4: [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0],
    },
    pid: 14,
};

// VT_LPWSTR — PROPVARIANT variant type tag for a null-terminated wide string.
// Value 31 is defined in the Windows Variant Type constants (wtypes.h).
const VT_LPWSTR: u16 = 31;

// ---------------------------------------------------------------------------
// AudioDeviceInfo
// ---------------------------------------------------------------------------

/// Describes one active audio output device on this system.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioDeviceInfo {
    /// Opaque device identifier string returned by IMMDevice::GetId.
    /// Pass this to `WasapiRenderer::start()` to select a specific device.
    pub id: String,

    /// Human-readable name shown in the Windows Sound control panel
    /// (e.g. "Speakers (Realtek High Definition Audio)" or
    /// "CABLE Input (VB-Audio Virtual Cable)").
    pub name: String,

    /// True if this is the current system-default render endpoint.
    pub is_default: bool,
}

// ---------------------------------------------------------------------------
// list_render_devices
// ---------------------------------------------------------------------------

/// Returns all active audio output (render) endpoints on this machine.
///
/// "Active" means present, enabled, and not disconnected. Disabled or
/// unplugged devices are excluded.
pub fn list_render_devices() -> Result<Vec<AudioDeviceInfo>, AudioError> {
    let _com = ComGuard::init()?;

    // unsafe: CoCreateInstance — standard COM object creation.
    let enumerator: IMMDeviceEnumerator = unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?
    };

    // Get the default device's ID so we can flag it in the list.
    let default_id = unsafe {
        enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .ok()
            .map(|dev| read_device_id(&dev))
    };

    // Enumerate all active render endpoints.
    let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
    let count = unsafe { collection.GetCount()? };

    let mut devices = Vec::with_capacity(count as usize);
    for i in 0..count {
        let device = unsafe { collection.Item(i)? };
        let id   = unsafe { read_device_id(&device) };
        let name = unsafe { read_device_name(&device) }.unwrap_or_else(|| id.clone());
        let is_default = default_id.as_deref() == Some(id.as_str());

        devices.push(AudioDeviceInfo { id, name, is_default });
    }

    Ok(devices)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Reads and returns the device ID string, freeing the COM-allocated PWSTR.
///
/// # Safety
/// `device` must be a valid IMMDevice.
unsafe fn read_device_id(device: &IMMDevice) -> String {
    match device.GetId() {
        Ok(ptr) => {
            let s = pwstr_to_string(ptr);
            // IMMDevice::GetId allocates via CoTaskMem — caller must free.
            CoTaskMemFree(Some(ptr.0.cast()));
            s
        }
        Err(_) => String::new(),
    }
}

/// Reads the friendly display name from an IMMDevice's property store.
///
/// Returns None if the property store can't be opened or the name isn't a
/// wide string variant (VT_LPWSTR).
///
/// # Safety
/// `device` must be a valid IMMDevice.
unsafe fn read_device_name(device: &IMMDevice) -> Option<String> {
    // STGM_READ = 0 — open for read access only.
    let store: IPropertyStore = device.OpenPropertyStore(STGM_READ).ok()?;

    // GetValue returns an owned PROPVARIANT. windows-rs implements Drop on
    // PROPVARIANT to call PropVariantClear, so we don't free it manually.
    let pv: PROPVARIANT = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME as *const _).ok()?;

    // Extract a PWSTR from the PROPVARIANT without depending on PropVariantToStringAlloc.
    // PROPVARIANT layout (x86_64, little-endian):
    //   offset 0: vt (u16)  — variant type tag
    //   offset 2: padding   — 3 reserved u16 fields
    //   offset 8: data      — the payload union (pointer-sized on x64)
    //
    // For VT_LPWSTR, the data union holds a *mut u16 (a PWSTR).
    // We read vt directly from offset 0 and the pointer from offset 8.
    // This avoids navigating the deeply nested anonymous union in the
    // windows-rs type definition, which varies across crate versions.
    let base = &pv as *const PROPVARIANT as *const u8;

    // unsafe: base points to a valid PROPVARIANT on our stack.
    let vt = base.cast::<u16>().read_unaligned();
    if vt != VT_LPWSTR {
        return None;
    }

    // unsafe: at offset 8, VT_LPWSTR stores the *mut u16 string pointer.
    let pwsz_ptr: *mut u16 = base.add(8).cast::<*mut u16>().read_unaligned();
    if pwsz_ptr.is_null() {
        return None;
    }

    // PWSTR is borrowed from the PROPVARIANT's data — do NOT free it here;
    // it will be freed when pv is dropped (PropVariantClear frees VT_LPWSTR data).
    let name = pwstr_to_string(PWSTR(pwsz_ptr));
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

// RPC_E_CHANGED_MODE: returned by CoInitializeEx when COM is already
// initialized on this thread with a different apartment model.
// Tauri v2 / WebView2 initializes the IPC thread as STA before our code
// runs, so our COINIT_MULTITHREADED request gets this code back.
// We can still use COM objects just fine — we just don't own the init.
const RPC_E_CHANGED_MODE: HRESULT = HRESULT(0x80010106u32 as i32);

struct ComGuard {
    should_uninit: bool,
}

impl ComGuard {
    fn init() -> Result<Self, AudioError> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            // COM already initialized by the host (Tauri/WebView2) — safe to proceed.
            return Ok(Self { should_uninit: false });
        }
        hr.ok()?;
        // S_OK or S_FALSE — we hold a COM ref, balance it on drop.
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
    fn list_render_devices_returns_at_least_one() {
        let devices = list_render_devices().expect("list_render_devices failed");
        assert!(!devices.is_empty());
    }

    #[test]
    #[ignore = "requires audio hardware and Windows audio service"]
    fn exactly_one_device_is_default() {
        let devices = list_render_devices().expect("list_render_devices failed");
        let n = devices.iter().filter(|d| d.is_default).count();
        assert_eq!(n, 1, "expected one default device, got {n}");
    }

    #[test]
    #[ignore = "requires audio hardware and Windows audio service"]
    fn all_devices_have_non_empty_id_and_name() {
        for d in list_render_devices().expect("list_render_devices failed") {
            assert!(!d.id.is_empty());
            assert!(!d.name.is_empty(), "device '{}' has empty name", d.id);
        }
    }
}
