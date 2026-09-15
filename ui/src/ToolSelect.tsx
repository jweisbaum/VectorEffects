import { useEffect, useId, useLayoutEffect, useRef, useState, type SelectHTMLAttributes } from "react";
import { createPortal } from "react-dom";

/**
 * A select with an in-page option list. WebKit's native popup can retain mouse
 * capture after selection and swallow the next canvas press, even after blur.
 * Keep the real select for labels, forms and change events; never open its
 * native popup. Closing our list does not consume presses outside the control.
 */
export default function ToolSelect(props: SelectHTMLAttributes<HTMLSelectElement>) {
  const select = useRef<HTMLSelectElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const id = useId();
  const [open, setOpen] = useState(false);
  const [highlight, setHighlight] = useState(0);
  const [position, setPosition] = useState({ left: 0, top: 0, width: 0, maxHeight: 240 });
  const search = useRef({ value: "", time: 0 });
  const readOptions = () => Array.from(select.current?.options ?? []);
  const options = readOptions();
  const enabled = (index: number, choices = readOptions()) => choices[index] && !choices[index].disabled;

  const show = () => {
    if (props.disabled) return;
    select.current?.focus({ preventScroll: true });
    setHighlight(Math.max(0, select.current?.selectedIndex ?? 0));
    setOpen(true);
  };
  const choose = (index: number) => {
    const element = select.current;
    const options = readOptions();
    if (!element || !enabled(index)) return;
    element.value = options[index]!.value;
    element.dispatchEvent(new Event("change", { bubbles: true }));
    setOpen(false);
    element.blur();
  };

  useLayoutEffect(() => {
    if (!open || !select.current) return;
    const rect = select.current.getBoundingClientRect();
    const below = window.innerHeight - rect.bottom - 8;
    const maxHeight = Math.min(240, Math.max(below, rect.top - 8));
    const width = Math.min(Math.max(rect.width, 160), window.innerWidth - 16);
    setPosition({
      left: Math.max(8, Math.min(rect.left, window.innerWidth - width - 8)),
      top: below >= Math.min(240, options.length * 30) ? rect.bottom + 3 : Math.max(8, rect.top - maxHeight - 3),
      width, maxHeight,
    });
  }, [open, props.children]);

  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !select.current?.contains(event.target) && !menu.current?.contains(event.target)) {
        setOpen(false);
      }
    };
    const close = () => setOpen(false);
    document.addEventListener("pointerdown", outside, true);
    window.addEventListener("resize", close);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("resize", close);
    };
  }, [open]);

  useEffect(() => {
    if (open) menu.current?.children[highlight]?.scrollIntoView?.({ block: "nearest" });
  }, [open, highlight]);

  return <span className="tool-select">
    <select {...props} ref={select}
      aria-expanded={open} aria-controls={open ? id : undefined}
      aria-activedescendant={open ? `${id}-${highlight}` : undefined}
      onPointerDown={event => {
        if (event.button !== 0 || props.disabled) return;
        event.preventDefault();
        if (open) setOpen(false); else show();
      }}
      onMouseDown={event => event.preventDefault()}
      onClick={event => { event.preventDefault(); if (event.detail === 0 && !open) show(); }}
      onPointerUp={event => event.stopPropagation()}
      onKeyUp={event => event.stopPropagation()}
      onBlur={event => { setOpen(false); props.onBlur?.(event); }}
      onKeyDown={event => {
        const options = readOptions();
        event.stopPropagation();
        if (event.key === "Tab") { setOpen(false); return; }
        if (event.key === "Escape") { event.preventDefault(); setOpen(false); return; }
        if (event.key === "Enter" || event.key === " " || event.key === "F4") {
          event.preventDefault();
          if (open) choose(highlight); else show();
          return;
        }
        const direction = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
        if (direction || event.key === "Home" || event.key === "End") {
          event.preventDefault();
          if (!open) show();
          let index = event.key === "Home" ? 0 : event.key === "End" ? options.length - 1
            : (open ? highlight : select.current?.selectedIndex ?? 0) + direction;
          while (index >= 0 && index < options.length && !enabled(index)) index += direction || (event.key === "Home" ? 1 : -1);
          if (enabled(index)) setHighlight(index);
          return;
        }
        if (event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey) {
          event.preventDefault();
          const now = performance.now();
          search.current = { value: (now - search.current.time < 600 ? search.current.value : "") + event.key.toLowerCase(), time: now };
          if (!open) show();
          const index = options.findIndex((option, index) => enabled(index) && option.text.toLowerCase().startsWith(search.current.value));
          if (index >= 0) setHighlight(index);
        }
      }} />
    {open && createPortal(<div ref={menu} id={id} role="listbox" className="tool-select-menu"
      aria-label={props["aria-label"] ?? select.current?.labels?.[0]?.textContent ?? "Options"}
      style={position} onPointerDown={event => event.preventDefault()}>
      {options.map((option, index) => <button key={`${index}-${option.value}`} id={`${id}-${index}`}
        type="button" role="option" tabIndex={-1} aria-selected={option.selected}
        disabled={option.disabled} className={index === highlight ? "highlighted" : ""}
        onPointerMove={() => setHighlight(index)} onClick={() => choose(index)}>{option.text}</button>)}
    </div>, document.body)}
  </span>;
}
