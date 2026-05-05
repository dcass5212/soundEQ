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

### Code-signing certificate (SmartScreen)
The Tauri `.exe` and NSIS installer must be signed with a CA-trusted certificate. Without it, Windows SmartScreen shows an "unknown publisher" warning when users run the installer. **Azure Trusted Signing** (~$3/month) is the recommended path — Microsoft-backed, pay-per-signature, works with the Tauri build pipeline.

### Installer polish
The NSIS bundle config is in place (`tauri.conf.json`). Before shipping, verify: silent install works, uninstall removes `%APPDATA%\com.soundeq.app\` cleanly, and the Start Menu shortcut is created correctly.

### Multi-device and compatibility testing
Test with headphones, USB audio devices, and Bluetooth. Verify VB-Cable loopback behaves correctly at 44.1 kHz and 48 kHz. Test on a clean Windows 10 machine (not just the dev system).

---

## High Priority

### ~~Add a LICENSE file~~ ✅
MIT license added as `LICENSE` in the project root.

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

---

## Final Steps Before Release v1.0

### ~~Finalize code and run efficiency tests~~ ✅
Audit the audio capture and render loops for any unnecessary work per buffer. Profile with a release build under real listening conditions. Verify no regressions in latency or CPU usage since the last major feature additions (crossfeed, spectrum analyzer, per-app volume).

### ~~Review and update .gitignore~~ ✅
Confirm all build artifacts, dev utilities, downloaded assets, and local scratch files are covered. Remove any entries that no longer apply after the eq-apo removal.

### ~~Documentation review~~ ✅
Read through README.md, ROADMAP.md, DECISIONS.md, and CHANGELOG.md end-to-end with fresh eyes. Check for stale references, broken formatting, inaccurate feature descriptions, and anything that assumes internal context a new visitor wouldn't have. Verify all installation steps still match the current build output.

### ~~Incorporate icon into documentation and GitHub page~~ ✅ (partial)
Add the app icon to the README (header or features section), the GitHub repository's social preview image, and any other public-facing materials. Export a high-resolution PNG from `src-tauri/icons/app-icon-source.png` for use in graphics that don't use the `.ico` format.

- ✅ README header updated — icon rendered above the title at 100 px.
- ⬜ **GitHub social preview** — must be set manually: GitHub repo → **Settings → Social preview → Edit** → upload `src-tauri/icons/app-icon-source.png` (1024×1024). GitHub crops it to the 2:1 social card format automatically.

### ~~Uninstaller cleanup~~ ✅
The NSIS uninstaller removes the app binary and Start Menu shortcut but leaves `%APPDATA%\com.soundeq.app\` (saved profiles, config) on disk. Users who want a clean uninstall must delete this folder manually. Options:
- ~~Add an NSIS `[Run]` step to delete the folder after uninstall (with a confirmation prompt so users can choose to keep their profiles)~~ ✅ Done via `NSIS_HOOK_POSTUNINSTALL` in `src-tauri/nsis-hooks.nsh`.
- Or update the README uninstall instructions to explicitly mention this path alongside the current note
