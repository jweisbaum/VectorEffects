import { useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { TOPICS, searchTopics, type HelpImage, type Parameter } from "./topics";

function Parameters({ rows }: { rows: readonly Parameter[] | undefined }) {
  if (!rows?.length) return null;
  return <table className="help-parameters"><thead><tr><th>Parameter or option</th><th>What it does</th></tr></thead>
    <tbody>{rows.map(([name, description]) => <tr key={name}><th scope="row">{name}</th><td>{description}</td></tr>)}</tbody></table>;
}
function Steps({ steps }: { steps: string[] | undefined }) {
  return steps && <ol>{steps.map(step => <li key={step}>{step}</li>)}</ol>;
}

export default function Help({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const [topicId, setTopicId] = useState("workspace");
  const [query, setQuery] = useState("");
  const [zoom, setZoom] = useState<string | null>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const show = () => {
    previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setOpen(true);
  };
  const close = () => { setOpen(false); setZoom(null); };
  useEffect(() => {
    const listening = listen("help://open", show);
    return () => { void listening.then(off => off()); };
  }, []);
  useEffect(() => {
    if (!open) previousFocus.current?.focus({ preventScroll: true });
    const key = (e: KeyboardEvent) => {
      if (e.key === "F1") { e.preventDefault(); e.stopImmediatePropagation(); if (!open) show(); }
      else if (open) {
        e.stopImmediatePropagation();
        if (e.key === "Escape") { e.preventDefault(); if (zoom) setZoom(null); else close(); }
      }
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, [open, zoom]);
  const found = searchTopics(query);
  const topic = found.find(t => t.id === topicId) ?? found[0];
  const choose = (id: string, clearQuery = false) => {
    if (clearQuery) setQuery("");
    setTopicId(id); setZoom(null);
  };
  const pictures = (images?: HelpImage[]) => images?.map(({ path, caption }) => <figure key={path}>
    <button className="help-screenshot" onClick={() => setZoom(zoom === path ? null : path)}
      aria-label={`${zoom === path ? "Reduce" : "Enlarge"} screenshot: ${caption}`} aria-expanded={zoom === path}>
      <img loading="lazy" className={zoom === path ? "zoomed" : ""} src={`/help/${path}`} alt={caption} />
    </button><figcaption>{caption} Click to enlarge.</figcaption>
  </figure>);
  return <>
    <div style={{ display: "contents" }} inert={open || undefined}>{children}</div>
    {open && <div className="modal-backdrop help-backdrop" onClick={close}>
      <section className="help-dialog" role="dialog" aria-modal="true" aria-labelledby="help-title" onClick={e => e.stopPropagation()}>
        <header><h2 id="help-title">VectorEffects Help</h2><button onClick={close} aria-label="Close help">Close</button></header>
        <div className="help-body"><nav aria-label="Help topics">
          <input autoFocus type="search" aria-label="Search help" placeholder="Search tools and parameters…" value={query}
            onChange={e => { setQuery(e.target.value); setZoom(null); }} />
          <p className="help-count" role="status">{found.length} of {TOPICS.length} pages</p>
          {found.map((t, i) => <div key={t.id}>
            {(i === 0 || found[i - 1]?.group !== t.group) && <h3>{t.group}</h3>}
            <button aria-current={topic?.id === t.id ? "page" : undefined} onClick={() => choose(t.id)}>{t.title}</button>
          </div>)}
        </nav><article key={topic?.id} tabIndex={0} aria-label={topic?.title ?? "Search results"}>
          {topic ? <><h3>{topic.title}</h3>{topic.paragraphs.map(p => <p key={p}>{p}</p>)}
            <Steps steps={topic.steps} />
            <Parameters rows={topic.parameters} />
            {pictures(topic.images)}
            {topic.sections?.map(section => <section key={section.heading}>
              <h4>{section.heading}</h4>{section.paragraphs?.map(p => <p key={p}>{p}</p>)}
              <Steps steps={section.steps} /><Parameters rows={section.parameters} />{pictures(section.images)}
            </section>)}
            {topic.related && <footer className="help-related"><h4>Related pages</h4>
              {topic.related.map(id => <button key={id} onClick={() => choose(id, true)}>{TOPICS.find(t => t.id === id)?.title}</button>)}
            </footer>}
          </> : <p>No topics match “{query}”. Try a tool name, such as Clone, or a parameter, such as Feather.</p>}
        </article></div>
      </section>
    </div>}
  </>;
}
