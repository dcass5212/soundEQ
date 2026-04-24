# soundEQ — Feature Roadmap

## High Priority

### Auto-start engine on launch
Right now the user must click Start every time the app opens. The app should remember whether the engine was running and the last device selection, then start automatically. Device config is already persisted — this is mostly wiring.

### EQ bypass toggle
A quick on/off button that passes audio through unprocessed. Essential for A/B comparing EQ settings vs. flat. Should be visually obvious (e.g. green/grey) so the user always knows the current state.

### Level meter
A simple input/output dB meter showing audio is flowing through the engine. Without this there is no visual confirmation the EQ is working, making it hard to debug silence or misconfigured devices.

---

## Medium Priority

### Device change detection / auto-restart
If the user plugs in headphones or switches output devices, Windows changes the default endpoint and the engine breaks silently. The app should detect this and either restart automatically or show a clear error with a one-click restart.

### Import / export profiles
Allow users to save profiles as `.json` files and load them back. Useful for backups and sharing presets between machines. The internal format is already JSON so this is mainly a file-picker dialog.

### Spectrum analyzer
Real-time frequency visualization of the audio passing through the engine. Makes tuning EQ bands significantly more intuitive. Already flagged as Phase 2 in the project plan.

---

## Lower Priority / Polish

### Global hotkey to bypass
Toggle the EQ on/off system-wide with a configurable keyboard shortcut, without needing to open the window. Useful while gaming or watching video.

### Per-app volume control
Since audio sessions are already detected per-app, adding a volume slider per session is a natural extension of the existing per-app profile system.

### Tray icon state indicator
Change the tray icon appearance (color, badge, tooltip) to reflect whether the engine is running or stopped, so the user can tell at a glance from the taskbar.
