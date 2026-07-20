// =============================================================================
// rt_priority.rs — real-time scheduling boost for the audio threads
//
// WHAT THIS DOES:
// Provides an RAII guard that makes the current thread suitable for real-time
// audio work. It does two independent things:
//
//   1. MMCSS registration ("Pro Audio" task)
//      The Multimedia Class Scheduler Service (MMCSS) is a Windows service
//      that boosts registered threads into a high scheduling priority band
//      reserved for time-sensitive multimedia work. A thread registered under
//      the "Pro Audio" task runs at a priority above almost every normal
//      application thread, so even when the CPU is saturated by other programs
//      the audio thread still gets scheduled on time.
//
//   2. 1 ms system timer resolution (timeBeginPeriod)
//      Our capture/render loops pace themselves with thread::sleep. On
//      Windows, Sleep() wakes up on the next system timer tick — and the
//      default tick interval is 15.625 ms. A "sleep 5 ms" can therefore
//      actually sleep 15+ ms, which is longer than an entire WASAPI buffer.
//      timeBeginPeriod(1) raises the tick rate to 1 ms so short sleeps are
//      accurate. It must be paired with timeEndPeriod(1), which the guard's
//      Drop impl guarantees.
//
// WHY THIS MATTERS:
// Without these, the audio threads run at normal priority with coarse sleeps.
// Under heavy CPU/RAM load the Windows scheduler can preempt them for tens of
// milliseconds at a time. The capture side then overflows its WASAPI buffer
// (Windows drops loopback data → crackle/static) and the render side
// underruns (we fill silence → stutter). This is exactly the classic
// "audio glitches when the system is busy" failure mode.
//
// FAILURE HANDLING:
// Both boosts are best-effort. If MMCSS is unavailable (service disabled,
// unusual Windows configuration) we log a warning and keep running at normal
// priority — degraded but functional beats refusing to start.
// =============================================================================

use windows::core::w;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
};

/// RAII guard that boosts the current thread for real-time audio.
///
/// Create it at the top of an audio thread's main function (after COM init)
/// and keep it alive for the thread's lifetime. Dropping it reverts both the
/// MMCSS registration and the timer resolution.
///
/// The boost applies to the thread that calls `enable()` — the guard must be
/// created and dropped on the same thread (it is intentionally !Send by way
/// of holding a raw MMCSS handle tied to this thread's registration).
pub(crate) struct RealTimeBoost {
    /// MMCSS registration handle, or None if registration failed.
    /// Needed by AvRevertMmThreadCharacteristics on drop.
    mmcss_handle: Option<HANDLE>,

    /// Whether our timeBeginPeriod(1) call succeeded and needs the matching
    /// timeEndPeriod(1) on drop. (Windows requires the calls to be paired.)
    timer_raised: bool,
}

impl RealTimeBoost {
    /// Boosts the calling thread. Never fails — each sub-boost that cannot be
    /// applied is logged and skipped so audio keeps flowing regardless.
    pub(crate) fn enable(thread_label: &str) -> Self {
        // --- 1 ms timer resolution ---
        // Returns TIMERR_NOERROR (0) on success. Raising the resolution is
        // per-process on modern Windows; pairing begin/end keeps us correct
        // on all versions.
        //
        // unsafe: timeBeginPeriod has no preconditions; it is unsafe only
        // because all raw Win32 calls are.
        let timer_raised = unsafe { timeBeginPeriod(1) } == 0;
        if !timer_raised {
            eprintln!("[eq-audio] {thread_label}: timeBeginPeriod(1) failed — sleeps will be coarse");
        }

        // --- MMCSS "Pro Audio" registration ---
        // "Pro Audio" is the highest-priority built-in MMCSS task class,
        // intended for exactly this kind of low-latency audio pipeline.
        // taskindex 0 means "create a new task group membership".
        //
        // unsafe: the task name is a valid null-terminated wide string (the
        // w! macro produces one at compile time) and taskindex points to a
        // live local. No other preconditions.
        let mut task_index: u32 = 0;
        let mmcss_handle = match unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) } {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("[eq-audio] {thread_label}: MMCSS registration failed ({e}) — running at normal priority");
                None
            }
        };

        Self { mmcss_handle, timer_raised }
    }
}

impl Drop for RealTimeBoost {
    fn drop(&mut self) {
        if let Some(handle) = self.mmcss_handle.take() {
            // unsafe: handle came from a successful AvSetMmThreadCharacteristicsW
            // on this same thread and is reverted exactly once.
            let _ = unsafe { AvRevertMmThreadCharacteristics(handle) };
        }
        if self.timer_raised {
            // unsafe: paired with the successful timeBeginPeriod(1) in enable().
            unsafe { timeEndPeriod(1) };
        }
    }
}
