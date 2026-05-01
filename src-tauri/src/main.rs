// Windows subsystem = "windows" suppresses the console window entirely.
// Without this, Windows spawns a cmd.exe terminal alongside the app process.
#![windows_subsystem = "windows"]

fn main() {
    soundeq_lib::run();
}
