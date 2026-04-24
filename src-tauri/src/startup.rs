// =============================================================================
// startup.rs — Windows startup registration
//
// WHAT THIS DOES:
// Writes / removes a value under:
//   HKCU\Software\Microsoft\Windows\CurrentVersion\Run
// so Windows launches soundEQ automatically when the user logs in.
//
// We use HKCU (current user) rather than HKLM (local machine) so no
// administrator rights are needed.
//
// The startup command line includes --minimized so the app starts hidden
// in the system tray instead of popping up a window at login.
// =============================================================================

use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE},
    RegKey,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APP_NAME: &str = "soundEQ";

/// Returns true if the soundEQ startup entry currently exists.
pub fn is_enabled() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|k| k.get_value::<String, _>(APP_NAME))
        .is_ok()
}

/// Writes the startup registry entry pointing to the current executable.
///
/// The `--minimized` flag tells `run()` to start without showing the window.
pub fn enable() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    // Quote the path in case it contains spaces.
    let value = format!("\"{}\" --minimized", exe.display());

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _disposition) = hkcu.create_subkey(RUN_KEY)?;
    key.set_value(APP_NAME, &value)
}

/// Removes the startup registry entry. No-op if it doesn't exist.
pub fn disable() -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
        Ok(key) => match key.delete_value(APP_NAME) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
