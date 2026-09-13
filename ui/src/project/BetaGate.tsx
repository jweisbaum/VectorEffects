import { useEffect, useState, type ReactNode } from "react";
import { api } from "../ipc";

/** Do not mount the editor, or issue its commands, before the native clock check. */
export default function BetaGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<string | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let active = true;
    const check = () => void api.betaStatus().then((message) => {
      if (active) { setStatus(message); setError(null); }
    }).catch((err: unknown) => {
      if (active) { setStatus(undefined); setError(String(err)); }
    });
    check();
    const timer = window.setInterval(check, 60_000);
    return () => { active = false; window.clearInterval(timer); };
  }, [attempt]);
  if (status === null) return children;
  return <main className="beta-gate" aria-live="polite">
    <div><h1>VectorEffects</h1>
      <p>{status ?? (error ? "Unable to check this version’s expiry date." : "Starting VectorEffects…")}</p>
      {error && <><p className="muted">{error}</p><button onClick={() => setAttempt(attempt + 1)}>Try again</button></>}
    </div>
  </main>;
}
