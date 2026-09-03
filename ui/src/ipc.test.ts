import { describe, expect, it } from "vitest";

import { IpcError, isErrorPayload } from "./ipc";

describe("isErrorPayload", () => {
  it("accepts the shape Rust actually sends", () => {
    expect(isErrorPayload({ kind: "grib", message: "bad grid" })).toBe(true);
  });

  it("rejects a bare string, which is how a panic arrives", () => {
    expect(isErrorPayload("something exploded")).toBe(false);
    expect(isErrorPayload(null)).toBe(false);
    expect(isErrorPayload({ kind: "grib" })).toBe(false);
  });
});

describe("IpcError", () => {
  it("carries the machine-readable kind alongside the message", () => {
    const err = new IpcError({ kind: "polar", message: "leg infeasible" });
    expect(err.kind).toBe("polar");
    expect(err.message).toBe("leg infeasible");
    expect(err).toBeInstanceOf(Error);
  });
});
