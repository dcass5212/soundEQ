// =============================================================================
// SetupBanner.tsx — VB-Audio Virtual Cable setup guidance
//
// Shown when no virtual cable device is detected in the system's audio device
// list. Guides the user through the one-time setup needed for soundEQ to work:
// install VB-Cable → set CABLE Input as the Windows default output → pick real
// speakers as the Output device in soundEQ.
// =============================================================================

import { openVbCableDownload } from "../lib/api";

interface SetupBannerProps {
  onDismiss: () => void;
}

export function SetupBanner({ onDismiss }: SetupBannerProps) {
  return (
    <div className="flex-shrink-0 flex items-start gap-3 px-4 py-2.5 bg-amber-950 border-b border-amber-800 text-amber-200 text-xs">
      <span className="mt-0.5 shrink-0 text-amber-400 font-bold">!</span>

      <div className="flex-1 space-y-1.5">
        <p className="font-semibold text-amber-100">VB-Cable not found — required for soundEQ</p>
        <ol className="list-decimal list-inside space-y-0.5 text-amber-300">
          <li>
            Download and install the free{" "}
            <button
              onClick={() => openVbCableDownload().catch(() => {})}
              className="font-mono text-amber-100 underline underline-offset-2 hover:text-white transition-colors"
            >
              VB-Audio Virtual Cable
            </button>{" "}
            driver.
          </li>
          <li>
            Open <span className="font-mono text-amber-100">Windows Sound Settings</span> and set{" "}
            <span className="font-mono text-amber-100">CABLE Input</span> as your default output
            device — this routes all app audio through soundEQ.
          </li>
          <li>
            In the <span className="font-mono text-amber-100">Output</span> dropdown above, select
            your real speakers or headphones.
          </li>
          <li>Press <span className="font-mono text-amber-100">Start</span>.</li>
        </ol>
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
