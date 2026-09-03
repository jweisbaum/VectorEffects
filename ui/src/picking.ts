/**
 * Placing an existing object's position property by clicking the map.
 *
 * The inspector can edit a position by typing a longitude and a latitude, which
 * is exact but no use at all for a point you are looking at: an aim point, an
 * anchor or a clone source is chosen by *where it is*, not by its coordinates.
 * Arming a pick hands the next map click to one property of one object.
 *
 * The request travels through the app shell because the two ends are in
 * different panels: the inspector arms it, the map answers it.
 */
export interface PositionPick {
  /** The object whose property is being placed. */
  object: number;
  /** The property id, as the inspector and the backend name it. */
  property: string;
  /** Its label, so the map can say what it is placing. */
  label: string;
  /** Where the property points now, drawn so the move is visible. */
  lon: number;
  lat: number;
}
