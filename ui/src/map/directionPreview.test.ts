import { describe, expect, it } from "vitest";
import { previewField, type ToolState } from "./tools";

describe("direction previews", () => {
  it.each([0, 1])("keeps inward/outward independent of circle rotation %i", (sense) => {
    for (const [angle, expected] of [[-90, 180], [0, sense === 0 ? 90 : 270], [90, 0]] as const) {
      const state: ToolState = { unit: "km", values: {
        RotationSense: { kind: "choice", index: sense }, CircleAngle: { kind: "number", value: angle },
      } };
      expect(previewField("circle", state, null).azimuthAt(0, 5)).toBeCloseTo(expected);
    }
  });

  it("points inward using the bearing at a high-latitude cell", () => {
    const state: ToolState = { unit: "km", values: { CircleAngle: { kind: "number", value: -90 } } };
    const footprint = { kind: "disc" as const, centre: [0, 65] as const, radiusKm: 2000, space: "geodesic" as const };
    // Initial great-circle bearing from (15, 70) to (0, 65).
    const phi1 = 70 * Math.PI / 180, phi2 = 65 * Math.PI / 180, delta = -15 * Math.PI / 180;
    const expected = (Math.atan2(Math.sin(delta) * Math.cos(phi2), Math.cos(phi1) * Math.sin(phi2) - Math.sin(phi1) * Math.cos(phi2) * Math.cos(delta)) * 180 / Math.PI + 360) % 360;
    expect(previewField("circle", state, footprint).azimuthAt(15, 70)).toBeCloseTo(expected);
  });

  it.each(["brush", "shape_fill"] as const)("previews rhumb and great-circle target offsets for %s", (tool) => {
    const state: ToolState = { unit: "km", values: {
      DirectionMode: { kind: "choice", index: 1 },
      Target: { kind: "position", lon: 90, lat: 60 },
      TargetAngle: { kind: "number", value: -30 },
      TargetPath: { kind: "choice", index: 1 },
    } };
    expect(previewField(tool, state, null).azimuthAt(0, 60)).toBeCloseTo(60);
    state.values.DirectionMode = { kind: "choice", index: 2 };
    expect(previewField(tool, state, null).azimuthAt(0, 60)).toBeCloseTo(240);
    state.values.DirectionMode = { kind: "choice", index: 1 };
    state.values.TargetPath = { kind: "choice", index: 0 };
    expect(previewField(tool, state, null).azimuthAt(0, 60)).toBeCloseTo(19.10660535);
  });
});
