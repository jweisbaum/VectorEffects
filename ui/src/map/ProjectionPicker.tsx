import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { MAP_PROJECTIONS, projectionOf, type ProjectionId } from "./projection";
import { customMap } from "./projections/general";

/** The same searchable catalogue serves world views, regional grids and UTM. */
export default function ProjectionPicker({ value, disabled, onChange }: {
  value: string; disabled: boolean; onChange: (id: string) => Promise<unknown>;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [definition, setDefinition] = useState("");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (open) input.current?.focus(); }, [open]);
  const close = () => { setOpen(false); trigger.current?.focus(); };
  const choose = async (id: string) => {
    setPending(true); setError("");
    try { await onChange(id); close(); } catch (e) { setError(String(e)); }
    finally { setPending(false); }
  };
  const groups = new Map<string, typeof MAP_PROJECTIONS[number][]>();
  for (const p of MAP_PROJECTIONS) {
    const group = p.general?.group ?? "Cylindrical maps";
    const text = `${p.label} ${group} ${p.general?.keywords ?? ""} ${p.general?.code ? `EPSG:${p.general.code}` : ""}`.toLowerCase();
    if (!query.toLowerCase().split(/\s+/).every(word => text.includes(word))) continue;
    const list = groups.get(group) ?? []; list.push(p); groups.set(group, list);
  }
  return <>
    <button className="projection-trigger" ref={trigger} disabled={disabled} onClick={() => setOpen(true)}
      title="Choose a world, polar or regional projection" aria-label="Choose map projection">
      {projectionOf(value as ProjectionId).label}
    </button>
    {open && createPortal(<div className="modal-backdrop" onClick={close}>
      <section className="modal projection-picker" role="dialog" aria-modal="true" aria-labelledby="projection-title"
        onClick={e => e.stopPropagation()} onKeyDown={e => {
          e.stopPropagation();
          if (e.key === "Escape") close();
          if (e.key === "Tab") {
            const buttons = [...e.currentTarget.querySelectorAll<HTMLElement>("button:not(:disabled), input, textarea")];
            const first = buttons[0], last = buttons[buttons.length - 1];
            if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
            else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
          }
        }}>
        <h2 id="projection-title">Map projection</h2>
        <p className="muted">{MAP_PROJECTIONS.length} views. Regional maps open at their area of use. Drag the globe or an azimuthal view to change its centre.</p>
        <input ref={input} type="search" aria-label="Search projections" placeholder="Search name, country or EPSG code" value={query} onChange={e => setQuery(e.target.value)} />
        <div className="projection-list">
          {[...groups].map(([group, entries]) => <section key={group}>
            <h3>{group}</h3>
            {entries.map(p => <button key={p.id} disabled={pending} aria-pressed={p.id === value} onClick={() => void choose(p.id)}>
              <span>{p.label}</span>{p.general?.code && <small>EPSG:{p.general.code}</small>}
            </button>)}
          </section>)}
          {groups.size === 0 && <p>No matching projections.</p>}
        </div>
        <details><summary>Custom projection</summary>
          <label>PROJ definition, WKT, or bundled EPSG code
            <textarea aria-label="Custom projection definition" rows={3} value={definition} onChange={e => setDefinition(e.target.value)} placeholder="+proj=lcc +lat_1=33 +lat_2=45 +lat_0=39 +lon_0=-96 +datum=WGS84 +units=m" />
          </label>
          <button disabled={pending || !definition.trim()} onClick={() => {
            try { void choose(customMap(definition).id); } catch (e) { setError(String(e)); }
          }}>Use definition</button>
          <p className="muted">Definitions work offline. Map coordinates use bundled datum parameters; survey correction grids are not included.</p>
        </details>
        {error && <p className="modal-error" role="alert">{error}</p>}
        <div className="modal-actions"><button onClick={close}>Close</button></div>
      </section>
    </div>, document.body)}
  </>;
}
