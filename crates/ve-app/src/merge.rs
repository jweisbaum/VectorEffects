//! Absorbing a gesture into an object it is indistinguishable from.
//!
//! Going over an area repeatedly should leave one thing to select, move and
//! animate rather than a pile of identical objects, so a stroke that lands on
//! an existing one it cannot be told apart from becomes another *chain* of it
//! (spec.md 6.1). This is a property of the geometry rather than of the brush:
//! every tool that draws a stroke is offered the same test, and the tools that
//! must not take it say so where they refuse it.
//!
//! Merging is only allowed where it cannot change what the layer looks like,
//! which is what every condition below is protecting.

use ve_core::LonLat;
use ve_core::document::{Geometry, Layer, LocalPoint, Object};
use ve_core::schema::PropId;
use ve_render::aeqd::Local;
use ve_render::scene::flatten_object;
use ve_render::sdf::chains_distance;

/// How far a merged stroke may reach from its anchor, in metres.
///
/// Geometry lives in an azimuthal-equidistant frame, which is well conditioned
/// near its anchor and degenerates at the antipode. A quarter of the earth's
/// circumference is a wide margin from that, and a stroke wanting to reach
/// further is better off as its own object with its own anchor.
const MERGE_MAX_RADIUS_M: f64 = 10_007_543.0;

/// How far apart, as a fraction, the sizes of two strokes painted in pixels
/// may be and still merge (M32).
///
/// A size in px is resolved to km at the zoom the stroke was painted at
/// (spec.md 3.5), and the wheel zooms continuously, so two strokes of one
/// pixel brush a scroll apart never agreed to the metre — and two modifier
/// strokes that do not merge compound where they overlap, doubling the
/// effect. Within this much the stroke joins at the target's width, which
/// under a feathered edge is the same brush to the eye; further apart it is
/// a different brush and stays its own object. A size typed in km is exact
/// and compared exactly.
const PX_SIZE_TOLERANCE: f64 = 0.1;

/// Whether a stroke's stamp is a shape on the map rather than the ground.
fn is_projected(object: &Object) -> bool {
    object
        .props
        .value_at(object.tool, PropId::StampSpace, 0)
        .and_then(ve_core::PropValue::as_enum)
        == Some(1)
}

/// The base of a size property, in km.
fn size_of(object: &Object, id: PropId) -> Option<f64> {
    object
        .props
        .value_at(object.tool, id, 0)
        .and_then(ve_core::PropValue::as_f32)
        .map(f64::from)
}

/// Finds the object `stroke` should be merged into, with the merged geometry.
///
/// Merging is only safe when it changes nothing about how the layer composites.
/// That needs three things, checked from the top of the stack down:
///
///  * the target renders identically to the new stroke apart from where it sits
///    (every property equal, keyframes included);
///  * their footprints actually overlap, or the two would read as one object
///    while looking like two;
///  * nothing between them in z-order overlaps the new stroke, or absorbing it
///    downwards would move it beneath something it was painted on top of.
pub fn merge_target(
    layer: &Layer,
    stroke: &Object,
    positions: &[LonLat],
) -> Option<(ve_core::id::Id, Geometry)> {
    let flat_stroke = flatten_object(stroke, 0)?;
    // Both brush shapes sweep the same chains; the reach that decides whether
    // two strokes touch is the stamp's half-width either way. For a square that
    // is measured across the flats rather than the diagonal, so the overlap
    // test below is slightly strict — and strict is the safe direction, since
    // refusing a merge only costs tidiness while a wrong one changes the layer.
    let reach_m = match flat_stroke.shape {
        ve_render::sdf::Shape::Capsule { radius_m, .. } => radius_m,
        ve_render::sdf::Shape::SweptSquare { half_size_m, .. } => half_size_m,
        _ => return None,
    };

    for candidate in layer.objects.iter().rev() {
        if let Some(merged) = merged_geometry(candidate, stroke, positions, reach_m) {
            return Some((candidate.id, merged));
        }
        // Not a merge target: if it overlaps the new stroke it stands between
        // the stroke and anything below, and the search stops.
        if blocks(candidate, &flat_stroke) {
            return None;
        }
    }
    None
}

/// The geometry `candidate` would have with `stroke` merged into it, if the two
/// may be merged at all.
fn merged_geometry(
    candidate: &Object,
    stroke: &Object,
    positions: &[LonLat],
    reach_m: f64,
) -> Option<Geometry> {
    if candidate.shape_animation.is_some() || stroke.shape_animation.is_some() {
        return None;
    }
    if candidate.tool != stroke.tool || candidate.active_range != stroke.active_range {
        return None;
    }
    let Geometry::Stroke { chains } = &candidate.geometry else {
        return None;
    };

    // Position differs by definition; everything else, keyframes included, must
    // match or the merged object could not render both strokes as they were
    // painted.
    if candidate.props.len() != stroke.props.len() {
        return None;
    }
    for (id, anim) in candidate.props.iter() {
        if *id == PropId::Position {
            // An animated position moves the whole object, so the frame the new
            // stroke would be expressed in is not the frame it was painted in.
            if anim.is_animated() {
                return None;
            }
            continue;
        }
        // Two pixel brushes a scroll apart are the same brush within a
        // tolerance; the stroke takes the target's width (M32).
        if *id == PropId::SizeKm
            && is_projected(candidate)
            && is_projected(stroke)
            && !anim.is_animated()
            && stroke
                .props
                .get(*id)
                .is_some_and(|other| !other.is_animated())
            && let (Some(a), Some(b)) = (size_of(candidate, *id), size_of(stroke, *id))
        {
            if (a - b).abs() > a.max(b) * PX_SIZE_TOLERANCE {
                return None;
            }
            continue;
        }
        if stroke.props.get(*id) != Some(anim) {
            return None;
        }
    }

    // Re-express the new stroke in the candidate's frame. Identical properties
    // mean identical scale and rotation, so only the anchor differs.
    let flat = flatten_object(candidate, 0)?;
    let chain: Vec<Local> = positions
        .iter()
        .map(|position| flat.frame.to_local(*position))
        .collect();
    if chain
        .iter()
        .any(|p| p[0].hypot(p[1]) * flat.frame.scale > MERGE_MAX_RADIUS_M)
    {
        return None;
    }

    let existing: Vec<Vec<Local>> = chains
        .iter()
        .map(|chain| chain.iter().map(|p| [p.x, p.y]).collect())
        .collect();
    if chains_distance(&existing, std::slice::from_ref(&chain)) > reach_m * 2.0 {
        return None;
    }

    let mut merged = chains.clone();
    merged.push(chain.iter().map(|p| LocalPoint::new(p[0], p[1])).collect());
    Some(Geometry::Stroke { chains: merged })
}

/// Whether `candidate` stands between the new stroke and anything below it.
///
/// Checked over the candidate's whole lifetime, not just the step being edited:
/// an object that only appears later, or that moves, still covers the stroke
/// when it does, and the stack order has to hold at every step.
fn blocks(candidate: &Object, stroke: &ve_render::scene::FlatObject) -> bool {
    let range = candidate.active_range;
    // A still object has one footprint, so one step answers for every step.
    let last = if candidate.props.iter().any(|(_, anim)| anim.is_animated()) {
        range.end
    } else {
        range.start
    };
    (range.start..=last).any(|step| {
        flatten_object(candidate, step).is_some_and(|flat| footprints_may_overlap(&flat, stroke))
    })
}

/// Whether two footprints can touch, judged by bounding circles.
///
/// Deliberately conservative: an approximation that says "maybe" too often
/// costs a merge, while one that says "no" too often would reorder the stack.
fn footprints_may_overlap(
    a: &ve_render::scene::FlatObject,
    b: &ve_render::scene::FlatObject,
) -> bool {
    let reach = |o: &ve_render::scene::FlatObject| o.shape.bounding_radius_m() * o.frame.scale;
    a.frame.anchor.distance_m(b.frame.anchor) <= reach(a) + reach(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ve_core::schema::ToolKind;
    use ve_core::value::{Interpolation, PropValue};

    fn brush(anchor: LonLat, chain: Vec<[f64; 2]>) -> Object {
        let mut object = Object::new(ToolKind::Brush, "s", 12);
        object.geometry = Geometry::Stroke {
            chains: vec![
                chain
                    .into_iter()
                    .map(|p| LocalPoint::new(p[0], p[1]))
                    .collect(),
            ],
        };
        if let Some(prop) = object.props.get_mut(PropId::Position) {
            prop.set_base(PropValue::LonLat(anchor));
        }
        object
    }

    /// The frame a stroke is expressed in is the frame it was painted in. An
    /// animated position moves that frame, so the merged stroke would travel
    /// somewhere it was never painted.
    #[test]
    fn a_moving_object_does_not_absorb_a_stroke() {
        let anchor = LonLat::new(0.0, 0.0).unwrap();
        let target = brush(anchor, vec![[0.0, 0.0], [200_000.0, 0.0]]);
        let painted = brush(
            LonLat::new(1.0, 0.0).unwrap(),
            vec![[0.0, 0.0], [200_000.0, 0.0]],
        );
        let positions = [
            LonLat::new(1.0, 0.0).unwrap(),
            LonLat::new(2.0, 0.0).unwrap(),
        ];

        let mut layer = Layer::new("L");
        layer.objects.push(target.clone());
        assert!(
            merge_target(&layer, &painted, &positions).is_some(),
            "the two overlap and match, so they would otherwise merge"
        );

        let mut moving = target;
        moving.props.get_mut(PropId::Position).unwrap().set_key(
            6,
            PropValue::LonLat(LonLat::new(40.0, 20.0).unwrap()),
            Interpolation::Linear,
        );
        let mut layer = Layer::new("L");
        layer.objects.push(moving);
        assert!(merge_target(&layer, &painted, &positions).is_none());
    }

    fn sized(mut object: Object, size_km: f32, projected: bool) -> Object {
        object
            .props
            .get_mut(PropId::SizeKm)
            .unwrap()
            .set_base(PropValue::F32(size_km));
        object
            .props
            .get_mut(PropId::StampSpace)
            .unwrap()
            .set_base(PropValue::Enum(u8::from(projected)));
        object
    }

    /// Spec 6.1 (M32): a pixel brush is resolved to km at the zoom it was
    /// painted at, so two strokes of one brush a scroll apart differ slightly
    /// in km and merge within the tolerance; a real change of brush does not,
    /// and a size typed in km is compared exactly.
    #[test]
    fn pixel_brushes_a_scroll_apart_merge_and_kilometre_ones_do_not() {
        let anchor = LonLat::new(0.0, 0.0).unwrap();
        let chain = vec![[0.0, 0.0], [200_000.0, 0.0]];
        let positions = [
            LonLat::new(1.0, 0.0).unwrap(),
            LonLat::new(2.0, 0.0).unwrap(),
        ];
        let painted_at = LonLat::new(1.0, 0.0).unwrap();
        let attempt = |target_km: f32, stroke_km: f32, projected: bool| {
            let mut layer = Layer::new("L");
            layer
                .objects
                .push(sized(brush(anchor, chain.clone()), target_km, projected));
            let stroke = sized(brush(painted_at, chain.clone()), stroke_km, projected);
            merge_target(&layer, &stroke, &positions).is_some()
        };
        assert!(attempt(800.0, 800.0, true), "the same brush merges");
        assert!(
            attempt(800.0, 850.0, true),
            "a scroll apart in px is the same brush"
        );
        assert!(
            !attempt(800.0, 1000.0, true),
            "a quarter wider is another brush"
        );
        assert!(!attempt(800.0, 850.0, false), "a size typed in km is exact");
    }
}
