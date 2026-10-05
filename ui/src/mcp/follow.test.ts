import { describe, expect, it, vi } from "vitest";

import type { ProjectSummary } from "../generated/ProjectSummary";
import { applyDocumentChanged, type FollowActions } from "./follow";

const summary = (name: string, revision: number): ProjectSummary =>
  ({ name, revision, step_count: 4 } as unknown as ProjectSummary);

function actions(): FollowActions & Record<string, ReturnType<typeof vi.fn>> {
  return {
    setProject: vi.fn(),
    setStep: vi.fn(),
    setSelection: vi.fn(),
    setShapeEditing: vi.fn(),
    setActiveLayer: vi.fn(),
    clearError: vi.fn(),
    replaced: vi.fn(),
  };
}

describe("applyDocumentChanged", () => {
  it("replaces the project on an edit and touches nothing else", () => {
    const a = actions();
    applyDocumentChanged({ project: summary("P", 2), opened: false }, a);
    expect(a.setProject).toHaveBeenCalledWith(summary("P", 2));
    expect(a.setStep).not.toHaveBeenCalled();
    expect(a.setSelection).not.toHaveBeenCalled();
    expect(a.clearError).not.toHaveBeenCalled();
    expect(a.replaced).not.toHaveBeenCalled();
  });

  it("resets the editor state when a different project opens, as the open path does", () => {
    const a = actions();
    applyDocumentChanged({ project: summary("Q", 1), opened: true }, a);
    expect(a.setSelection).toHaveBeenCalledWith([]);
    expect(a.setShapeEditing).toHaveBeenCalledWith(null);
    expect(a.setActiveLayer).toHaveBeenCalledWith(null);
    expect(a.setStep).toHaveBeenCalledWith(0);
    expect(a.clearError).toHaveBeenCalled();
    // The map is told before the summary lands, so it places a regional
    // project's camera on its region even when the region is unchanged (R8).
    expect(a.replaced).toHaveBeenCalled();
    expect(vi.mocked(a.replaced!).mock.invocationCallOrder[0]!).toBeLessThan(vi.mocked(a.setProject).mock.invocationCallOrder[0]!);
    expect(a.setProject).toHaveBeenCalledWith(summary("Q", 1));
  });

  it("clears everything and shows the start screen on null", () => {
    const a = actions();
    applyDocumentChanged({ project: null, opened: true }, a);
    expect(a.setSelection).toHaveBeenCalledWith([]);
    expect(a.setStep).toHaveBeenCalledWith(0);
    expect(a.clearError).toHaveBeenCalled();
    expect(a.setProject).toHaveBeenCalledWith(null);
  });
});
