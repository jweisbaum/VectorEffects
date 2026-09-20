// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import ProjectionPicker from "./ProjectionPicker";

(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
it("searches the offline catalogue by EPSG and applies the selected map", async()=>{
  const host=document.createElement("div");document.body.append(host);
  const root=createRoot(host),choose=vi.fn(async()=>{});
  try{
    await act(async()=>root.render(<ProjectionPicker value="equirectangular" disabled={false} onChange={choose}/>));
    await act(async()=>host.querySelector("button")!.click());
    const dialog=document.querySelector('[role="dialog"]')!;
    expect(dialog.textContent).toContain("270 views");
    const input=dialog.querySelector("input")!;
    await act(async()=>{
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,"EPSG:3413");
      input.dispatchEvent(new Event("input",{bubbles:true}));
    });
    const entries=dialog.querySelectorAll<HTMLButtonElement>(".projection-list button");
    expect(entries.length).toBe(1);
    expect(entries[0]!.textContent).toContain("Polar Stereographic North");
    await act(async()=>entries[0]!.click());
    expect(choose).toHaveBeenCalledWith("epsg_3413");
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(host.querySelector("button"));
  }finally{await act(async()=>root.unmount());host.remove();}
});
