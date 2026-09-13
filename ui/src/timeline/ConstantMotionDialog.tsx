import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import NumberField from "../NumberField";
import { api } from "../ipc";
import type { ProjectSummary } from "../generated/ProjectSummary";
import { useUnits } from "../settings/units";

export interface MotionSegment { object: number; start: number; end: number; existing: number }

export default function ConstantMotionDialog({ segment, onDone, onClose }: {
  segment: MotionSegment; onDone: (project: ProjectSummary) => void; onClose: () => void;
}) {
  const units = useUnits();
  const [direction, setDirection] = useState(90);
  const [speed, setSpeed] = useState(10);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const key = (e: KeyboardEvent) => { if (e.key === "Escape" && !busy) onClose(); };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose, busy]);
  const submit = () => {
    if (segment.existing > 0 && !confirm) { setConfirm(true); return; }
    setBusy(true);
    void api.addConstantMotion(segment.object, segment.start, direction, units.speedToMps(speed), confirm)
      .then(onDone).then(onClose).catch((err: unknown) => { setError(String(err)); setBusy(false); });
  };
  return createPortal(<div className="modal-backdrop" onClick={() => !busy && onClose()}>
    <div className="modal modal-narrow" role="dialog" aria-modal="true" aria-labelledby="constant-motion-title" onClick={(e) => e.stopPropagation()}>
      <h2 id="constant-motion-title">{confirm ? "Overwrite position keyframes?" : "Add constant motion"}</h2>
      <p>Move from frame {segment.start} to frame {segment.end} at a constant compass bearing.</p>
      {confirm ? <p>This replaces {segment.existing} existing position keyframe{segment.existing === 1 ? "" : "s"} in that interval. You can undo the change.</p> : <>
        <label>Direction toward (° true)<NumberField aria-label="Direction toward" value={direction} onCommit={setDirection} min={0} max={360} /></label>
        <label>Speed ({units.speedUnit})<NumberField aria-label="Motion speed" value={speed} onCommit={setSpeed} min={0} /></label>
      </>}
      {error && <p className="error" role="alert">{error}</p>}
      <div className="modal-actions"><button disabled={busy} onClick={onClose}>Cancel</button>
        <button className={confirm ? "danger" : "primary"} disabled={busy || !Number.isFinite(speed) || speed < 0} onClick={submit}>{busy ? "Adding…" : confirm ? "Overwrite and add motion" : "Add motion"}</button>
      </div>
    </div>
  </div>, document.body);
}
