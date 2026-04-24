// =============================================================================
// SetupBanner.tsx — VB-Audio Virtual Cable setup guidance
//
// Shown when no virtual cable device is detected in the system's audio device
// list. Guides the user through the one-time setup needed for soundEQ to work:
// install VB-Cable → set CABLE Input as the Windows default output → pick real
// speakers as the Output device in soundEQ.
// =============================================================================

interface SetupBannerProps {
  onDismiss: () => void;
}

export function SetupBanner({ onDismiss }: SetupBannerProps) {
  return (
    <div className="flex-shrink-0 flex items-start gap-3 px-4 py-2.5 bg-amber-950 border-b border-amber-800 text-amber-200 text-xs">
      <span className="mt-0.5 shrink-0 text-amber-400 font-bold">!</span>

      <div className="flex-1 space-y-1">
        <p className="font-semibold text-amber-100">Virtual audio cable not found</p>
        <p className="text-amber-300 leading-relaxed">
          soundEQ needs a virtual audio cable to intercept system audio. Follow
          these steps:
        </p>
        <ol className="list-decimal list-inside space-y-0.5 text-amber-300">
          <li>
            Search for <span className="font-mono text-amber-100">VB-Audio Virtual Cable</span> and
            install the free driver.
          </li>
          <li>
            Open <span className="font-mono text-amber-100">Windows Sound settings</span> and set{" "}
            <span className="font-mono text-amber-100">CABLE Input</span> as your default output
            device — this routes all app audio through soundEQ.
          </li>
          <li>
            In the <span className="font-mono text-amber-100">Output</span> dropdown above, select
            your real speakers or headphones.
          </li>
          <li>Press <span className="font-mono text-amber-100">Start</span>.</li>
        </ol>
        <p className="text-amber-500 text-[11px]">
          Once VB-Cable is installed, the{" "}
          <span className="font-mono">Capture</span> dropdown will automatically
          select it.
        </p>
      </div>

      <button
        onClick={onDismiss}
        title="Dismiss"
        className="shrink-0 mt-0.5 text-amber-500 hover:text-amber-200 transition-colors"
      >
        ✕
      </button>
    </div>
  );
}
