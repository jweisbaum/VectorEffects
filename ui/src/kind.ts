/**
 * The two kinds of field a layer can be part of (spec.md 4.3, M29): 10 m
 * wind and surface current. The names are the backend's; the labels are what
 * the panel and the title bar say; the letter is the tile address's, where a
 * tile of each kind is a different tile.
 */

export type FieldKindName = "wind" | "current";

export const KINDS: readonly FieldKindName[] = ["wind", "current"];

export const KIND_LABELS: Record<FieldKindName, string> = {
  wind: "10 m wind",
  current: "Surface currents",
};

/** The kind's segment in a tile address: `w` or `c`, as `protocol::parse` reads it. */
export function kindLetter(kind: FieldKindName): "w" | "c" {
  return kind === "wind" ? "w" : "c";
}

/** A kind from anything the backend might call it, defaulting to wind. */
export function kindOf(value: string | null | undefined): FieldKindName {
  return value === "current" ? "current" : "wind";
}
