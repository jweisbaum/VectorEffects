import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

import type { OpenProgress } from "../generated/OpenProgress";
import { barFraction, barLabel, reportOpening, useOpening } from "./opening";

/**
 * The page shown while a project opens (spec.md 4.7).
 *
 * Mounted for the life of the application and empty unless something is
 * opening: the listener has to be in place before the first report, and the
 * command that produces them is begun by the ipc layer, not by this.
 *
 * It covers everything, the start screen and an open project alike — what is
 * under it is either about to be replaced or not yet drawn — and it fades in
 * after a moment's delay (in the stylesheet), so a project that opens within
 * a frame or two never flashes a page at anyone.
 */
export default function LoadingScreen() {
  const opening = useOpening();

  useEffect(() => {
    const pending = listen<OpenProgress>("open://progress", (event) => reportOpening(event.payload));
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  if (opening.title === null) return null;
  const percent = Math.round(barFraction(opening) * 100);
  return (
    <div className="loading" role="dialog" aria-modal="true" aria-label={`Opening ${opening.title}`}>
      <div className="loading-panel">
        <p className="muted loading-verb">Opening</p>
        <h1 className="loading-title" title={opening.title}>{opening.title}</h1>
        <div
          className="progress-bar loading-bar"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent}
        >
          <div className="progress-fill" style={{ width: `${percent}%` }} />
        </div>
        <p className="muted loading-label" aria-live="polite">{barLabel(opening)}</p>
      </div>
    </div>
  );
}
