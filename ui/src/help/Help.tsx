import { useEffect, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { TOPICS } from "./topics";

export default function Help({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const [topicId, setTopicId] = useState("workspace");
  const [query, setQuery] = useState("");
  const [zoom, setZoom] = useState(false);
  useEffect(() => {
    const listening = listen("help://open", () => setOpen(true));
    return () => { void listening.then((off) => off()); };
  }, []);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "F1") { e.preventDefault(); e.stopImmediatePropagation(); setOpen(true); }
      else if (open) {
        e.stopImmediatePropagation();
        if (e.key === "Escape") { e.preventDefault(); if (zoom) setZoom(false); else setOpen(false); }
      }
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, [open, zoom]);
  const found = TOPICS.filter((topic) => `${topic.title} ${topic.paragraphs.join(" ")} ${topic.steps?.join(" ") ?? ""}`.toLowerCase().includes(query.toLowerCase()));
  const topic = found.find((t) => t.id === topicId) ?? found[0];
  return <>
    <div style={{ display: "contents" }} inert={open || undefined}>{children}</div>
    {open && <div className="modal-backdrop help-backdrop" onClick={() => setOpen(false)}>
      <section className="help-dialog" role="dialog" aria-modal="true" aria-labelledby="help-title" onClick={(e) => e.stopPropagation()}>
        <header><h2 id="help-title">VectorEffects Help</h2><button onClick={() => setOpen(false)} aria-label="Close help">Close</button></header>
        <div className="help-body"><nav aria-label="Help topics"><input autoFocus aria-label="Search help" placeholder="Search help…" value={query} onChange={(e) => setQuery(e.target.value)} />
          {found.map((t) => <button key={t.id} aria-current={topic?.id === t.id ? "page" : undefined} onClick={() => { setTopicId(t.id); setZoom(false); }}>{t.title}</button>)}
        </nav><article key={topic?.id}>
          {topic ? <><h3>{topic.title}</h3>{topic.paragraphs.map((p) => <p key={p}>{p}</p>)}
            {topic.steps && <ol>{topic.steps.map((step) => <li key={step}>{step}</li>)}</ol>}
            {topic.image && <figure><button className="help-screenshot" onClick={() => setZoom(!zoom)} aria-label={zoom ? "Reduce screenshot" : "Enlarge screenshot"}><img className={zoom ? "zoomed" : ""} src={`/help/${topic.image}.png`} alt={topic.caption} /></button><figcaption>{topic.caption} Click to enlarge.</figcaption></figure>}
          </> : <p>No topics match “{query}”.</p>}
        </article></div>
      </section>
    </div>}
  </>;
}
