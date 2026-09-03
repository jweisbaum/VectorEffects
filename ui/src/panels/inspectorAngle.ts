/**
 * The inspector's angle conversion, in its own module so it can be tested
 * without rendering the panel.
 */

import { displayDirection } from "../project/format";

/**
 * A stored angle as it is shown, and back again.
 *
 * A *flow direction* is stored as an azimuth-toward and shown in the project's
 * convention, which for a "from" project is the reciprocal — the same
 * conversion the map readout and the brush's own Dir field make. Without it the
 * inspector reported the reciprocal of what the user had just painted: 270
 * entered on the toolbar came back as 90 here.
 *
 * A geometric angle — an object's rotation, a gradient's axis — is not a flow
 * direction and is shown exactly as stored, which is why the schema
 * distinguishes the two units rather than converting every angle.
 *
 * `displayDirection` is its own inverse (it adds 180 either way), so this
 * converts in both directions.
 */
export function toShownAngle(unit: string, convention: string, degrees: number): number {
  return unit === "direction" ? displayDirection(convention, degrees) : degrees;
}
