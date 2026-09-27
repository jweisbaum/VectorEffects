import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { onReveal } from "../help/highlight";
import { msg, TranslatableError, useT } from "../i18n";
import { MAP_PROJECTIONS, projectionOf, type ProjectionId } from "./projection";
import { customMap } from "./projections/general";

/** The heading of the built-in cylindrical projections, which carry no group of their own. */
const CYLINDRICAL = msg("Cylindrical maps");

/** The same searchable catalogue serves world views, regional grids and UTM. */
export default function ProjectionPicker({ value, disabled, onChange }: {
  value: string; disabled: boolean; onChange: (id: string) => Promise<unknown>;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [definition, setDefinition] = useState("");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (open) input.current?.focus(); }, [open]);
  // The Help search opens the picker to show what is inside it.
  useEffect(() => onReveal("projection:open", () => { if (!disabled) setOpen(true); }), [disabled]);
  const close = () => { setOpen(false); trigger.current?.focus(); };
  const choose = async (id: string) => {
    setPending(true); setError("");
    try { await onChange(id); close(); } catch (e) { setError(String(e)); }
    finally { setPending(false); }
  };
  const groups = new Map<string, typeof MAP_PROJECTIONS[number][]>();
  for (const p of MAP_PROJECTIONS) {
    const group = p.general?.group ?? CYLINDRICAL;
    // Searched in English and in the language on screen, so either finds it.
    const text = `${p.label} ${t(p.label)} ${group} ${t(group)} ${p.general?.keywords ?? ""} ${p.general?.code ? `EPSG:${p.general.code}` : ""}`.toLowerCase();
    if (!query.toLowerCase().split(/\s+/).every(word => text.includes(word))) continue;
    const list = groups.get(group) ?? []; list.push(p); groups.set(group, list);
  }
  return <>
    <button className="projection-trigger" ref={trigger} disabled={disabled} onClick={() => setOpen(true)}
      data-feature="map:projection"
      title={t("Choose a world, polar or regional projection")} aria-label={t("Choose map projection")}>
      {t(projectionOf(value as ProjectionId).label)}
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
        <h2 id="projection-title">{t("Map projection")}</h2>
        <p className="muted">{t("{count} views. Regional maps open at their area of use. Drag the globe or an azimuthal view to change its centre.", { count: MAP_PROJECTIONS.length })}</p>
        <input ref={input} type="search" aria-label={t("Search projections")} placeholder={t("Search name, country or EPSG code")} value={query} onChange={e => setQuery(e.target.value)} />
        <div className="projection-list">
          {[...groups].map(([group, entries]) => <section key={group}>
            <h3>{t(group)}</h3>
            {entries.map(p => <button key={p.id} disabled={pending} aria-pressed={p.id === value} onClick={() => void choose(p.id)}>
              <span>{t(p.label)}</span>{p.general?.code && <small>EPSG:{p.general.code}</small>}
            </button>)}
          </section>)}
          {groups.size === 0 && <p>{t("No matching projections.")}</p>}
        </div>
        <details data-feature="map:custom-projection"><summary>{t("Custom projection")}</summary>
          <label>{t("PROJ definition, WKT, or bundled EPSG code")}
            <textarea aria-label={t("Custom projection definition")} rows={3} value={definition} onChange={e => setDefinition(e.target.value)}
              /* i18n-ignore: an example PROJ definition, which is code */
              placeholder="+proj=lcc +lat_1=33 +lat_2=45 +lat_0=39 +lon_0=-96 +datum=WGS84 +units=m" />
          </label>
          <button disabled={pending || !definition.trim()} onClick={() => {
            try { void choose(customMap(definition).id); } catch (e) { setError(e instanceof TranslatableError ? t(e.key, e.params) : e instanceof Error ? e.message : String(e)); }
          }}>{t("Use definition")}</button>
          <p className="muted">{t("Definitions work offline. Map coordinates use bundled datum parameters; survey correction grids are not included.")}</p>
        </details>
        {error && <p className="modal-error" role="alert">{error}</p>}
        <div className="modal-actions"><button onClick={close}>{t("Close")}</button></div>
      </section>
    </div>, document.body)}
  </>;
}
