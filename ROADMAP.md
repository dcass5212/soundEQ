# soundEQ — Feature Roadmap

## Completed
- **EQ bypass toggle** — amber button passes audio through unmodified; canvas dims + dashed 0 dB line
- **Spectrum analyzer** — real-time FFT display behind the EQ curve, fast-attack/slow-release smoothing
- **Auto-start engine on launch** — resumes last device selection automatically; skips if VB-Cable not detected
- **Device change detection / auto-restart** — polls every 2 s; falls back to Windows default if render device disappears
- **Per-band color customization** — color swatch per band, EQ curve becomes a horizontal gradient between band colors
- **Band list redesign + drag-to-reorder** — dark card rows, 3 px accent bar, SVG grip handle, mouse-event drag (HTML5 DnD dropped)
- **Undo / redo for band edits** — Ctrl+Z / Ctrl+Y, ↩ / ↪ header buttons, 50-step history; canvas dot drags count as one step
- **Band solo / mute** — M button mutes a band live (non-destructive), S button solos it (silences all others); resets on profile switch
- **Profile rename** — pencil icon inline edit in the sidebar
- **Single-instance guard** — second launch brings the existing window to foreground
- **Focus-based per-app profile routing** — foreground window polling at 250 ms, app chips glow green when focused
- **Resizable panels** — EQ canvas, profile sidebar, and apps bar are all user-resizable
- **APO Phase 1** — COM shell compiles, registers as MFX on all render endpoints, passthrough confirmed
- **APO Phase 2 (code)** — DSP wired: FilterChain built in LockForProcess, APOProcess applies EQ in-place, reads active_profile.json from %PUBLIC%\soundEQ\
- **Global hotkey to bypass** — Ctrl+Shift+B toggles EQ bypass system-wide via `RegisterHotKey`; UI button stays in sync via 250 ms polling (Tauri events are unreliable from OS shortcut callback threads)
- **Tray icon status** — tooltip and context menu status item update dynamically: ○ Stopped / ⊘ Bypassed / ● Active — Profile Name
- **Output gain control** — Vol −/+ in the header compensates for VB-Cable's lower signal level vs. direct output; 50–200% range, 5% steps, hold-to-repeat after 400 ms, persisted in config.json
- **Per-app volume control** — range slider per app in the Apps panel; adjusts `ISimpleAudioVolume` on the WASAPI session live; persisted in `ProfileStore.app_volumes`; resets to 100% on app removal
- **Keyboard navigation for band rows** — roving tabindex makes the list Tab-reachable; ↑/↓ move rows, Shift+↑/↓ reorder, Enter/F2 enter edit mode, Delete removes, Space toggles enabled, m mutes, s solos, Escape exits edit mode
- **Headphone crossfeed** — per-profile Mild / Moderate / Strong preset levels; Linkwitz-style algorithm (LP at 700 Hz + ~0.3 ms inter-aural delay) applied after the EQ chain; segmented button bar above the band list; stored in the profile JSON (`#[serde(default)]` for backward compat)
- **Profile import / export** — `↓` button per profile row downloads the profile as `<name>.json`; `↑ Import` button loads a `.json` file and resolves name collisions with a numeric suffix; two Tauri commands (`export_profile`, `import_profile`) handle serialization and validation
- **Windows startup delay (fixed 10 s)** — when launched via the Windows startup registry entry (`--minimized` flag), the engine auto-start is deferred 10 seconds to let the audio subsystem finish initializing; the UI is fully usable during the wait; implemented via `is_startup_launch` Tauri command

---

## Before Shipping (blocking — must be done before distributing to others)

### CA code-signing certificate
Self-signed certs are rejected by Windows Memory Integrity (HVCI), which is on by default on modern Windows 11 hardware. audiodg.exe will silently refuse to load an unsigned or self-signed APO DLL. **Azure Trusted Signing** (~$3/month) is the recommended path — Microsoft-backed, broadly trusted, pay-per-signature.

### APO Phase 2 — confirm DSP works end-to-end
The code is written but has never actually run inside audiodg.exe (HVCI blocked all test runs). Once signed, verify: the filter chain applies audibly, the correct bands load from active_profile.json, APOProcess is being called (check apo_trace.log), and restarting the audio service doesn't crash audiodg.exe.

### APO Phase 2 — fix sample rate in LockForProcess
The APO currently reads sample rate from active_profile.json. This is wrong if the negotiated device rate differs (44.1 kHz, 96 kHz, etc.) — the biquad coefficients will be offset and the EQ curve will be incorrect. LockForProcess receives the actual negotiated format via `APO_CONNECTION_DESCRIPTOR`; read the sample rate from there instead of the JSON file.

### APO Phase 3 — live profile reload
The APO reads active_profile.json once at Initialize time. Changing the EQ curve in the UI has no effect until the audio service is restarted. Phase 3 adds a file watcher (ReadDirectoryChangesW or similar) so profile changes apply immediately without a service restart.

### APO Phase 4 — remove VB-Cable
Strip the WASAPI loopback capture and render pipeline from eq-audio. Remove VB-Cable detection and the setup banner from the UI. Update the first-run experience to reflect the APO-based architecture. Users should not need VB-Cable installed at all.

### APO Phase 5 — installer
Ship a proper installer (WiX or NSIS) that:
- Copies eq_apo.dll to a stable location (e.g. `Program Files\soundEQ\`)
- Runs APO registration (equivalent of register.ps1) with admin elevation at install time
- Handles uninstall cleanly — unregisters the COM class and removes FxProperties entries from all endpoints
- Creates %APPDATA%\soundEQ\ and the %PUBLIC%\soundEQ\ shared directory
- Adds the Tauri app to Windows startup

### App signing (SmartScreen)
The Tauri .exe and installer must also be signed with the same CA cert. Without it, Windows SmartScreen shows an "unknown publisher" warning when users run the installer, which is a significant trust barrier for new users.

### Multi-device and compatibility testing
Test with headphones, USB audio devices, and Bluetooth (Bluetooth APOs have quirks). Verify the APO is registered and active on newly connected devices. Test on a clean Windows 10 machine (not just the dev system).

---

## High Priority

### AutoEQ headphone correction ⭐
The [AutoEQ project](https://github.com/jaakkopasanen/AutoEq) has frequency response measurements and correction EQ curves for 2,000+ headphones. Let users pick their headphone model from a searchable list and auto-load the correction profile. The DSP is the same parametric EQ already in eq-core — this is mostly a data pipeline and UI problem. Sonarworks SoundID does this and charges $100+/year; offering it at a fraction of that price is a strong differentiator.

### Per-device automatic profiles
Automatically switch to a different EQ profile when the user plugs in headphones vs. using speakers. The Tauri app already listens for device change events — the missing piece is associating profiles with specific audio endpoints rather than just apps.

---

## Medium Priority

### Profile scheduling
Apply different profiles automatically based on time of day (e.g. "Night Mode" after 22:00 that cuts bass and limits volume). Stored as time-range rules in `ProfileStore`; a background tick checks and swaps.

### Loudness normalization
Measure the average loudness of what's playing and apply a gain offset to keep volume consistent across apps and content (ReplayGain-style). Eliminates jarring volume jumps when switching between a quiet podcast and a loud game.

### REW / room correction import
Accept `.frd` or `.txt` frequency response files exported from Room EQ Wizard and auto-generate a correction EQ profile. Targets the serious audiophile segment who already measures their room or headphones.

---

## Lower Priority / Polish

### Dynamic EQ
Bands that react to the signal level — boost at low volumes (loudness contour), cut harshly sibilant frequencies only when they peak. More advanced DSP than static parametric EQ; requires a gain computer and level detector per band.

### Mid-side processing
Separate EQ for the center (mid) channel vs. the stereo width (side). Useful for widening or narrowing stereo image, taming harshness that only appears in centered content, etc.

### MIDI controller support
Map physical knobs/faders on a MIDI controller to band gain or frequency. Niche but compelling for producers who want tactile EQ adjustment.

### Export profile as shareable string
Encode a profile as a compact base64 URL fragment so users can paste it into a forum post or chat and others can import it with one click.

### Configurable startup delay
The current fixed 10-second delay covers most systems. A user-settable value (0–30 s) in a settings panel would let people on fast machines skip the wait and people on slow machines extend it.
