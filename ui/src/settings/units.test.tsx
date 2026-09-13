// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { displayUnits, UnitsProvider, type UnitPreferences } from "./units";
import { formatValue, plotSeries } from "../timeline/graph";
import ToolOptions from "../map/ToolOptions";
import type { ToolSchema } from "../generated/ToolSchema";
import type { ToolState } from "../map/tools";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("global display units", () => {
  it.each(["kt", "mph", "kmh"] as const)("converts %s without changing stored speeds", (speed) => {
    const units = displayUnits({ distance_unit: "nm", speed_unit: speed });
    expect(units.speedToMps(units.speedFromMps(12.345))).toBeCloseTo(12.345, 10);
    expect(units.distanceFromKm(1.852)).toBeCloseTo(1, 10);
    expect(units.distanceToKm(100)).toBeCloseTo(185.2, 10);
    const expected = speed === "kt" ? 19.4384449244 : speed === "mph" ? 22.3693629205 : 36;
    expect(units.speedFromMps(10)).toBeCloseTo(expected, 8);
    const plotted = plotSeries({ label: "", unit: "speed", values: [10] }, "toward", units);
    expect(plotted.values[0]).toBeCloseTo(expected, 8);
    expect(formatValue("speed", expected, units)).toContain(units.speedUnit);
  });

  it("keeps signed angle readouts negative and converts distances in graphs", () => {
    const units = displayUnits({ distance_unit: "nm", speed_unit: "kmh" });
    expect(plotSeries({ label: "", unit: "kilometres", values: [185.2] }, "from", units).values[0]).toBeCloseTo(100);
    expect(formatValue("signed_degrees", -90, units)).toBe("-90.0°");
  });

  it("converts editable ground sizes and speed, keeps pixels intact, and switches without writing", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    const schema: ToolSchema = {
      tool: "brush", label: "Brush", shortcut: "b", hover: true, preview: "field",
      gesture: { kind: "always", gesture: "stroke" }, sizing: { depends_on: [] }, eyedropper: null,
      options: [
        { property: "SizeKm", label: "Size", unit: "kilometres", default: { kind: "number", value: 185.2 }, min: 1, max: 20000 },
        { property: "Speed", label: "Speed", unit: "speed", default: { kind: "number", value: 10 }, min: 0, max: 120 },
      ].map((spec) => ({ ...spec, variants: [], depends_on: [], creation_only: false, slider: null })) as ToolSchema["options"],
    };
    let state: ToolState = { values: {}, unit: "km" };
    const changes: ToolState[] = [];
    const render = async (settings: UnitPreferences) => act(async () => root.render(
      <UnitsProvider settings={settings}>
        <ToolOptions schema={schema} state={state} onChange={(next) => { state = next; changes.push(next); }}
          convention="toward" camera={{ centerLon: 0, centerLat: 0, pxPerDeg: 4 }}
          picking={null} onPick={() => undefined} sampling={false} onSample={() => undefined} />
      </UnitsProvider>,
    ));
    try {
      await render({ distance_unit: "nm", speed_unit: "kmh" });
      let inputs = container.querySelectorAll<HTMLInputElement>("input[type=number]");
      expect(Number(inputs[0]!.value)).toBe(100);
      expect(Number(inputs[1]!.value)).toBe(36);
      expect(Number(inputs[1]!.max)).toBe(432);
      await render({ distance_unit: "km", speed_unit: "kt" });
      expect(changes).toHaveLength(0);
      inputs = container.querySelectorAll<HTMLInputElement>("input[type=number]");
      expect(Number(inputs[0]!.value)).toBeCloseTo(185.2);
      await render({ distance_unit: "nm", speed_unit: "kmh" });
      const input = container.querySelectorAll<HTMLInputElement>("input[type=number]")[1]!;
      await act(async () => {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, "72");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      expect(state.values.Speed).toEqual({ kind: "number", value: 20 });
      state = { values: { SizeKm: { kind: "number", value: 250 } }, unit: "px" };
      await render({ distance_unit: "nm", speed_unit: "mph" });
      expect(container.querySelector<HTMLInputElement>("input")?.value).toBe("250");
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
