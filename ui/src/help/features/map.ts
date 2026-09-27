import { msg } from "../../i18n";
import type { Feature } from "../features";

/**
 * The map's option bar and projection picker.
 *
 * A tool option is tagged `option:<PropId>` by the bar, which renders every
 * option from the schema; the ones worth finding by name are listed here,
 * each revealed by selecting a tool that has it with its default settings
 * (so the option is live, not hidden by a mode).
 */
const features: Feature[] = [
  // Shared by the creation tools; the brush is the one that has them all.
  { id: "option:Feather", label: msg("Feather"), description: msg("Softens the edge of what the tool paints, from a hard edge to a gradual fade."),
    keywords: [msg("soft edge"), msg("falloff"), msg("blur")], topic: "tools", reveal: ["tool:brush"] },
  { id: "option:EdgeMode", label: msg("Edge"), description: msg("Whether a feathered edge blends into the field beneath or replaces it."),
    keywords: [msg("blend"), msg("replace"), msg("edge mode")], topic: "tools", reveal: ["tool:brush"] },
  { id: "option:SizeKm", label: msg("Size"), description: msg("The size of the brush, in ground distance or in pixels on the map."),
    keywords: [msg("brush size"), msg("radius"), msg("footprint")], topic: "tools", reveal: ["tool:brush"] },
  { id: "option:Speed", label: msg("Speed"), description: msg("The speed of the wind or current the tool paints."),
    keywords: [msg("wind speed"), msg("strength"), msg("knots")], topic: "tools", reveal: ["tool:brush"] },
  { id: "option:Direction", label: msg("Direction"), description: msg("The direction of the painted flow, in the project’s direction convention."),
    keywords: [msg("bearing"), msg("heading"), msg("angle")], topic: "tools", reveal: ["tool:brush"] },
  { id: "option:DirectionMode", label: msg("Direction mode"), description: msg("Paint one constant direction, or aim every vector toward or away from a target."),
    keywords: [msg("toward point"), msg("away from point"), msg("aim")], topic: "targets", reveal: ["tool:brush"] },
  { id: "option:BrushShape", label: msg("Brush shape"), description: msg("Paint with a round or a square stamp."),
    keywords: [msg("square"), msg("round"), msg("stamp")], topic: "brush", reveal: ["tool:brush"] },
  { id: "option:unit", label: msg("Size unit (km or px)"), description: msg("Measure a shape on the ground, or in pixels so it keeps its shape on the map."),
    keywords: [msg("pixels"), msg("kilometres"), msg("stamp space")], topic: "tools", reveal: ["tool:brush"] },
  { id: "option:eyedropper", label: msg("Eyedropper"), description: msg("Take the speed and direction from a point on the map."),
    keywords: [msg("sample"), msg("pick colour"), msg("copy vector")], topic: "tools", reveal: ["tool:brush"] },
  { id: "option:pick-on-map", label: msg("Pick on map"), description: msg("Place a position option, such as a target or a source, by clicking the map."),
    keywords: [msg("place"), msg("click position"), msg("coordinates")], topic: "tools", reveal: ["tool:clone_stamp"] },

  // The circle stamp.
  { id: "option:FillMode", label: msg("Fill"), description: msg("A filled disc, a ring, or a disc whose speed ramps from centre to edge."),
    keywords: [msg("ring"), msg("disc"), msg("perimeter")], topic: "circle", reveal: ["tool:circle"] },
  { id: "option:DiameterKm", label: msg("Diameter"), description: msg("The diameter of the circle stamp."),
    keywords: [msg("circle size"), msg("radius")], topic: "circle", reveal: ["tool:circle"] },
  { id: "option:RotationSense", label: msg("Rotation"), description: msg("Whether the flow around the circle turns clockwise or counterclockwise."),
    keywords: [msg("clockwise"), msg("cyclone"), msg("anticyclone")], topic: "circle", reveal: ["tool:circle"] },
  { id: "option:CircleAngle", label: msg("Angle from tangent"), description: msg("Tilts the circling flow inward or outward, keeping its speed."),
    keywords: [msg("inflow"), msg("outflow"), msg("spiral")], topic: "circle", reveal: ["tool:circle"] },

  // The shape fill.
  { id: "option:ShapeSource", label: msg("Shape"), description: msg("Draw a polygon, or drag out a square, rectangle or circle."),
    keywords: [msg("polygon"), msg("rectangle"), msg("preset")], topic: "fill", reveal: ["tool:shape_fill"] },
  { id: "option:VectorMode", label: msg("Vector mode"), description: msg("Fill the shape with one vector, or with a gradient between two."),
    keywords: [msg("gradient"), msg("constant"), msg("ramp")], topic: "fill", reveal: ["tool:shape_fill"] },

  // The mask and the clone stamp.
  { id: "option:Invert", label: msg("Invert"), description: msg("Mask everything except what is drawn, confining the field to a region."),
    keywords: [msg("inverse"), msg("outside"), msg("confine")], topic: "mask", reveal: ["tool:mask"] },
  { id: "option:SourcePoint", label: msg("Source"), description: msg("Where the clone stamp copies the field from."),
    keywords: [msg("sample point"), msg("origin"), msg("clone from")], topic: "clone", reveal: ["tool:clone_stamp"] },
  { id: "option:OffsetMode", label: msg("Offset"), description: msg("Keep the source at a fixed offset from the stroke, or at one fixed point."),
    keywords: [msg("aligned"), msg("fixed"), msg("source offset")], topic: "clone", reveal: ["tool:clone_stamp"] },

  // The path.
  { id: "option:CurveKind", label: msg("Curve"), description: msg("Draw the path as straight segments or as a smooth Bézier curve."),
    keywords: [msg("bezier"), msg("polyline"), msg("spline")], topic: "path", reveal: ["tool:curve"] },
  { id: "option:WidthKm", label: msg("Width"), description: msg("The width of the corridor the path paints."),
    keywords: [msg("corridor"), msg("jet width"), msg("thickness")], topic: "path", reveal: ["tool:curve"] },
  { id: "option:CurveDirectionMode", label: msg("Direction mode"), description: msg("Let the flow follow the path, or hold one constant direction."),
    keywords: [msg("relative to path"), msg("follow route"), msg("along the path")], topic: "path", reveal: ["tool:curve"] },

  // The edit tools.
  { id: "option:Gain", label: msg("Amount"), description: msg("How much Intensify / reduce strengthens or weakens the field, from −100% to +200%."),
    keywords: [msg("intensify"), msg("reduce"), msg("gain")], topic: "intensity", reveal: ["tool:intensity"] },
  { id: "option:Radial", label: msg("Amount"), description: msg("How strongly Diverge / converge bends the flow outward or inward."),
    keywords: [msg("divergence"), msg("convergence"), msg("radial")], topic: "divergence", reveal: ["tool:divergence"] },
  { id: "option:TurnSense", label: msg("Turn"), description: msg("Whether Rotate flow turns the vectors clockwise or counterclockwise."),
    keywords: [msg("rotate direction"), msg("clockwise"), msg("veer")], topic: "rotation", reveal: ["tool:turn"] },
  { id: "option:TurnAmountDeg", label: msg("Amount"), description: msg("How many degrees Rotate flow turns the vectors."),
    keywords: [msg("rotation angle"), msg("degrees"), msg("veer")], topic: "rotation", reveal: ["tool:turn"] },
  { id: "option:WarpMode", label: msg("Warp"), description: msg("Push the field toward a point, or twist it about the anchor."),
    keywords: [msg("push"), msg("twist"), msg("swirl")], topic: "warp", reveal: ["tool:warp"] },
  { id: "option:InterpolationDistanceKm", label: msg("Interpolation distance"), description: msg("How wide a band around a displaced field is blended into the surrounding flow."),
    keywords: [msg("displace"), msg("blend band"), msg("smoothing")], topic: "warp", reveal: ["tool:liquify"] },

  // The projection.
  { id: "map:projection", label: msg("Map projection"), description: msg("Choose a world, polar or regional projection for the map view."),
    keywords: [msg("globe"), msg("mercator"), msg("polar")], topic: "view" },
  { id: "map:custom-projection", label: msg("Custom projection"), description: msg("Use a PROJ definition, WKT, or bundled EPSG code as the map projection."),
    keywords: [msg("definition"), msg("coordinate system"), msg("CRS")], topic: "view", reveal: ["projection:open"] },
];
export default features;
