import { describe, expect, it } from "vitest";
import { hitShapePoint, movedShapePoint } from "./shapeEditing";
import type { ShapeControls } from "../generated/ShapeControls";

const controls: ShapeControls = { object: 9, step: 5, revision: 10,
  rings: [[[0,0],[20,0],[20,20]],[[5,5],[6,5],[5,6]]],
  keyed: [[false,true,false],[false,false,false]],
};
const project = ([x,y]: [number,number]) => ({ x: x*2, y: y*2 });

describe("shape perimeter interaction", () => {
  it("picks the nearest visible point, including holes, in screen pixels", () => {
    expect(hitShapePoint(controls,project,{x:40,y:1},8)).toEqual({ ring: 0, point: 1 });
    expect(hitShapePoint(controls,project,{x:10,y:10},8)).toEqual({ ring: 1, point: 0 });
    expect(hitShapePoint(controls,project,{x:80,y:80},8)).toBeNull();
  });
  it("previews only the grabbed vertex and preserves the baseline for cancellation", () => {
    const moved = movedShapePoint(controls,{ring:1,point:2},[9,10]);
    expect(moved.rings[1]![2]).toEqual([9,10]);
    expect(controls.rings[1]![2]).toEqual([5,6]);
    expect(moved.rings[0]).toBe(controls.rings[0]);
    expect(moved.revision).toBe(controls.revision);
    expect(moved.keyed).toBe(controls.keyed);
  });
});
