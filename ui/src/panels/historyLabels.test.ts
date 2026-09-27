import { afterEach, describe, expect, it } from "vitest";

import { setLanguage } from "../i18n";
import { translateHistoryLabel } from "./historyLabels";

describe("translateHistoryLabel", () => {
  afterEach(() => setLanguage("en"));

  it("leaves every label as the backend wrote it in English", () => {
    expect(translateHistoryLabel("Add layer")).toBe("Add layer");
    expect(translateHistoryLabel("Delete 3 objects")).toBe("Delete 3 objects");
  });

  it("translates an exact label", () => {
    setLanguage("es");
    expect(translateHistoryLabel("Add layer")).toBe("Añadir capa");
    expect(translateHistoryLabel("Reorder layers")).toBe("Reordenar capas");
  });

  it("translates a patterned label and keeps its count", () => {
    setLanguage("es");
    expect(translateHistoryLabel("Paste 4 frames")).toBe("Pegar 4 fotogramas");
    expect(translateHistoryLabel("Rotate 2 objects")).toBe("Rotar 2 objetos");
    // A count, not an object called "3 objects".
    expect(translateHistoryLabel("Delete 3 objects")).toBe("Eliminar 3 objetos");
  });

  it("keeps a name inside a label as the document has it", () => {
    setLanguage("es");
    expect(translateHistoryLabel("Add Brush 3")).toBe("Añadir Brush 3");
    expect(translateHistoryLabel("Delete Front, 12 Jan")).toBe("Eliminar Front, 12 Jan");
    // An exact label is not mistaken for an object name.
    expect(translateHistoryLabel("Delete layer")).toBe("Eliminar capa");
  });

  it("passes a label it does not know through unchanged", () => {
    setLanguage("es");
    expect(translateHistoryLabel("Frobnicate the widget")).toBe("Frobnicate the widget");
  });
});
