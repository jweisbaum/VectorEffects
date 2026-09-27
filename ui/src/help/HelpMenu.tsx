import { useEffect, useId, useRef, useState } from "react";
import { IS_MAC } from "../chords";
import { reportError } from "../hint";
import { useT } from "../i18n";
import { searchFeatures, type Feature } from "./features";
import { locateFeature } from "./highlight";

const present = (id: string) => document.querySelector(`[data-feature="${CSS.escape(id)}"]`) !== null;
import { openHelp } from "./open";
import { searchTopics, type HelpTopic } from "./topics";

/** How many of each kind the dropdown lists. */
const FEATURE_LIMIT = 8;
const TOPIC_LIMIT = 4;

type Result = { kind: "feature"; feature: Feature } | { kind: "topic"; topic: HelpTopic } | { kind: "reference" };

/** Whether a key event is the search chord: Cmd-F on a Mac, Ctrl-F elsewhere. */
export function isSearchChord(event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">,
  mac: boolean = IS_MAC): boolean {
  if (event.key.toLowerCase() !== "f" || event.altKey || event.shiftKey) return false;
  return mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}

/**
 * The Help menu (spec.md 5.8): a search over the interface's features and the
 * help reference, in the language on screen. Choosing a feature brings it on
 * screen and flashes a rectangle around it; choosing a page opens the
 * reference there. Cmd-F (Ctrl-F) opens it with the search focused.
 */
export default function HelpMenu() {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const id = useId();
  const container = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (!isSearchChord(event)) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      // The reference, when it is up, has a search of its own.
      const reference = document.querySelector<HTMLInputElement>(".help-dialog input[type=search]");
      if (reference) { reference.focus(); reference.select(); return; }
      setOpen(true);
      requestAnimationFrame(() => { input.current?.focus(); input.current?.select(); });
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, []);

  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => {
      if (event.target instanceof Node && !container.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("focusin", outside);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("focusin", outside);
    };
  }, [open]);

  // A feature with no way on screen from here — the start page's controls
  // in the editor, an editor control with nothing to reveal it on the start
  // page — would only lead to its help page, which the pages below already do.
  const features = searchFeatures(query).filter(m => m.feature.reveal?.length || present(m.feature.id))
    .slice(0, FEATURE_LIMIT).map(m => ({ kind: "feature", feature: m.feature }) as const);
  const topics = query.trim() ? searchTopics(query).slice(0, TOPIC_LIMIT).map(topic => ({ kind: "topic", topic }) as const) : [];
  const results: Result[] = [...features, ...topics, { kind: "reference" }];
  const selected = Math.min(active, results.length - 1);

  const choose = (result: Result) => {
    setOpen(false);
    setQuery("");
    setActive(0);
    if (result.kind === "reference") { openHelp(); return; }
    if (result.kind === "topic") { openHelp(result.topic.id); return; }
    const feature = result.feature;
    void locateFeature(feature).then(found => {
      if (found) return;
      if (feature.topic) openHelp(feature.topic);
      else reportError(t("{feature} is not available here. Open a project to use it.", { feature: t(feature.label) }));
    });
  };

  const optionId = (index: number) => `${id}-option-${index}`;
  const row = (result: Result, index: number) => {
    const props = {
      id: optionId(index), role: "option", "aria-selected": index === selected,
      className: index === selected ? "active" : undefined,
      onPointerMove: () => setActive(index),
      onClick: () => choose(result),
    } as const;
    if (result.kind === "feature") {
      return <li key={`f:${result.feature.id}`} {...props}>
        <span className="help-search-label">{t(result.feature.label)}</span>
        {result.feature.description && <span className="help-search-detail">{t(result.feature.description)}</span>}
      </li>;
    }
    if (result.kind === "topic") {
      return <li key={`t:${result.topic.id}`} {...props}>
        <span className="help-search-label">{result.topic.title}</span>
        <span className="help-search-detail">{result.topic.group}</span>
      </li>;
    }
    return <li key="reference" {...props}>
      <span className="help-search-label">{t("Open the help reference")}</span>
      <span className="help-search-detail">F1</span>
    </li>;
  };

  return <div className="help-menu" ref={container}>
    <button data-feature="shell:help" aria-haspopup="listbox" aria-expanded={open}
      title={t("Search features and help ({chord})", { chord: IS_MAC ? "Cmd+F" : "Ctrl+F" })}
      onClick={() => {
        setOpen(!open);
        if (!open) requestAnimationFrame(() => input.current?.focus());
      }}>
      {t("Help")} <span aria-hidden="true">▾</span>
    </button>
    {open && <div className="help-menu-popup">
      <input ref={input} autoFocus type="search" role="combobox" aria-expanded="true" aria-controls={`${id}-list`}
        aria-activedescendant={optionId(selected)} aria-label={t("Search features and help")}
        placeholder={t("Search features and help…")} value={query}
        onChange={event => { setQuery(event.target.value); setActive(0); }}
        onKeyDown={event => {
          event.stopPropagation();
          if (event.key === "ArrowDown") { event.preventDefault(); setActive((selected + 1) % results.length); }
          else if (event.key === "ArrowUp") { event.preventDefault(); setActive((selected + results.length - 1) % results.length); }
          else if (event.key === "Enter") { event.preventDefault(); const r = results[selected]; if (r) choose(r); }
          else if (event.key === "Escape") { event.preventDefault(); setOpen(false); }
        }} />
      <ul id={`${id}-list`} role="listbox" aria-label={t("Search results")}>
        {features.length > 0 && <li role="presentation" className="help-search-heading">{t("Features")}</li>}
        {features.map((result, index) => row(result, index))}
        {topics.length > 0 && <li role="presentation" className="help-search-heading">{t("Help pages")}</li>}
        {topics.map((result, index) => row(result, features.length + index))}
        {query.trim() && features.length === 0 && topics.length === 0
          && <li role="presentation" className="help-search-empty">{t("Nothing matches “{query}”.", { query })}</li>}
        {row({ kind: "reference" }, results.length - 1)}
      </ul>
    </div>}
  </div>;
}
