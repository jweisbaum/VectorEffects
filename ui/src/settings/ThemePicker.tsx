import { useEffect, useId, useRef, useState } from "react";
import { THEMES, themeOf, type Theme } from "./themes";
import type { CustomTheme } from "../generated/CustomTheme";

/** Use the catalogue colours directly so previews do not inherit the active theme. */
function Palette({ theme }: { theme: Theme }) {
  return <span className="theme-palette" aria-hidden="true">
    {(["bg", "active", "border", "accent", "text"] as const).map(role =>
      <span key={role} style={{ backgroundColor: theme.roles[role] }} />,
    )}
  </span>;
}

export default function ThemePicker({ value, custom, onChoose }: {
  value: string;
  custom?: CustomTheme | null;
  onChoose: (id: string) => void;
}) {
  const themes = custom ? [...THEMES, themeOf("custom", custom)] : THEMES;
  const chosen = themes.find(theme => theme.id === value) ?? themeOf(undefined);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const held = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const id = useId();
  const listId = `${id}-list`;

  useEffect(() => {
    if (!open) return;
    const away = (event: PointerEvent) => {
      if (!held.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", away, true);
    return () => document.removeEventListener("pointerdown", away, true);
  }, [open]);

  useEffect(() => {
    if (open) document.getElementById(`${id}-${active}`)?.scrollIntoView?.({ block: "nearest" });
  }, [open, active, id]);

  const show = () => {
    setActive(themes.indexOf(chosen));
    setOpen(true);
  };
  const choose = (theme: Theme) => {
    setOpen(false);
    trigger.current?.focus();
    if (theme.id !== chosen.id) onChoose(theme.id);
  };

  return <div className="theme-picker" ref={held} onBlur={event => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setOpen(false);
  }}>
    <label id={`${id}-label`} htmlFor={id}>Theme</label>
    <button
      ref={trigger}
      id={id}
      type="button"
      role="combobox"
      className="theme-trigger"
      aria-labelledby={`${id}-label`}
      aria-haspopup="listbox"
      aria-expanded={open}
      aria-controls={open ? listId : undefined}
      aria-activedescendant={open ? `${id}-${active}` : undefined}
      onClick={() => {
        trigger.current?.focus();
        if (open) setOpen(false); else show();
      }}
      onKeyDown={event => {
        switch (event.key) {
          case "ArrowDown":
          case "ArrowUp":
            if (!open) show();
            else setActive(at => Math.max(0, Math.min(themes.length - 1, at + (event.key === "ArrowDown" ? 1 : -1))));
            break;
          case "Home": setActive(0); setOpen(true); break;
          case "End": setActive(themes.length - 1); setOpen(true); break;
          case "Enter":
          case " ":
            if (open) choose(themes[active]!); else show();
            break;
          case "Escape":
            if (!open) return;
            setOpen(false);
            break;
          case "Tab": setOpen(false); return;
          default: return;
        }
        event.preventDefault();
        event.stopPropagation();
      }}
    >
      <span className="theme-name">{chosen.name}</span>
      <Palette theme={chosen} />
      <span className="theme-marker" aria-hidden="true">▾</span>
    </button>
    {open && <ul className="theme-list" id={listId} role="listbox" aria-labelledby={`${id}-label`}>
      {themes.map((theme, index) => <li
        key={theme.id}
        id={`${id}-${index}`}
        role="option"
        aria-selected={theme.id === chosen.id}
        className={`theme-option${index === active ? " active" : ""}`}
        onPointerMove={() => setActive(index)}
        onPointerDown={event => event.preventDefault()}
        onClick={() => choose(theme)}
      >
        <span className="theme-name">{theme.name}</span>
        <Palette theme={theme} />
        <span className="theme-marker" aria-hidden="true">{theme.id === chosen.id ? "✓" : ""}</span>
      </li>)}
    </ul>}
  </div>;
}
