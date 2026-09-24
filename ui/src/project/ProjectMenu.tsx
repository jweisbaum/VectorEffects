import { useEffect, useId, useRef, useState } from "react";

type Action = () => void;

/** Project commands share their existing save/replace guards in App. */
export default function ProjectMenu({ onNew, onOpen, onSave, onSaveAs, onClose }: {
  onNew: Action; onOpen: Action; onSave: Action; onSaveAs: Action; onClose: Action;
}) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const container = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const firstFocus = useRef(0);
  const actions = [
    { label: "New…", run: onNew },
    { label: "Open…", run: onOpen },
    { label: "Save", run: onSave },
    { label: "Save As…", run: onSaveAs },
    { label: "Close", run: onClose },
  ];

  useEffect(() => {
    if (!open) return;
    menu.current?.querySelectorAll<HTMLButtonElement>("button")[firstFocus.current]?.focus();
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

  const close = () => { setOpen(false); trigger.current?.focus(); };

  return <div className="project-menu" ref={container}>
    <button ref={trigger} aria-haspopup="menu" aria-expanded={open} aria-controls={open ? id : undefined}
      onClick={() => { firstFocus.current = 0; setOpen(!open); }}
      onKeyDown={event => {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
          event.preventDefault(); event.stopPropagation();
          firstFocus.current = event.key === "ArrowUp" ? actions.length - 1 : 0;
          setOpen(true);
        }
      }}>
      Project <span aria-hidden="true">▾</span>
    </button>
    {open && <div ref={menu} id={id} className="project-menu-items" role="menu" aria-label="Project"
      onKeyDown={event => {
        // Existing project shortcuts still reach App's window handler.
        if (event.metaKey || event.ctrlKey) { setOpen(false); return; }
        event.stopPropagation();
        const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button")];
        const current = items.indexOf(document.activeElement as HTMLButtonElement);
        let next: number | null = null;
        if (event.key === "ArrowDown") next = (current + 1) % items.length;
        if (event.key === "ArrowUp") next = (current + items.length - 1) % items.length;
        if (event.key === "Home") next = 0;
        if (event.key === "End") next = items.length - 1;
        if (next !== null) { event.preventDefault(); items[next]?.focus(); }
        else if (event.key === "Escape") { event.preventDefault(); close(); }
        // Start normal tab navigation from the trigger when the popup closes.
        else if (event.key === "Tab") close();
      }}>
      {actions.map(action => <button key={action.label} role="menuitem" tabIndex={-1}
        onClick={() => { close(); action.run(); }}>{action.label}</button>)}
    </div>}
  </div>;
}
