//! What the tools palette and the option bar need to know about each tool.
//!
//! The inspector has been schema-driven since M5: it asks what properties an
//! object has and renders them, so adding a property is one line in `ve-core`
//! and nothing in the frontend. The *option bar* is the same question asked
//! before the object exists, and it is answered the same way here — otherwise
//! every tool would carry a hand-written bar, and spec 6.1's rules about which
//! options are shown would have to be re-implemented six times in TypeScript
//! and would be wrong in at least one of them.
//!
//! Nothing here decides anything. It reports what `ve_core::schema` already
//! says, plus the two facts about a tool that are not properties: which gesture
//! drives it, and whether it has a hover indicator (spec.md 6.2).

use serde::Serialize;
use ts_rs::TS;
use ve_core::schema::{self, PropId, ToolKind, Unit};

use crate::create::Tool;
use crate::document::PropertyValue;
use crate::error::Result;

/// Why an option might not be read, and by what.
///
/// The frontend resolves these against the values the tool bar currently holds,
/// which is the one thing it has that the backend does not. The *rule* stays
/// here, so "which options does this mode make inert" has one answer for the
/// inspector and the option bar both (spec.md 6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "OptionDependency.ts")]
pub struct OptionDependency {
    /// The choice property that decides.
    pub on: String,
    /// The variant indices of `on` for which the option is read.
    pub live_for: Vec<u8>,
}

/// One option a tool offers, described well enough to render.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ToolOptionSpec.ts")]
pub struct ToolOptionSpec {
    /// The property id, spelled as `create_object` expects it.
    pub property: String,
    /// The label shown beside the control.
    pub label: String,
    /// Display unit: `"speed"`, `"kilometres"`, `"degrees"`, `"direction"`,
    /// `"percent"` or `"none"`. A speed is stored in m/s and shown in knots; a
    /// `direction` is a flow direction and shown in the project's convention,
    /// where `degrees` is a geometric bearing and is not converted (spec.md 3.3).
    pub unit: String,
    /// The value the tool starts at.
    pub default: PropertyValue,
    /// Lower bound, for numeric options.
    pub min: Option<f32>,
    /// Upper bound, for numeric options.
    pub max: Option<f32>,
    /// Variant names, for choices.
    pub variants: Vec<String>,
    /// Whether the option is fixed once the object exists.
    ///
    /// Reported rather than hidden: a creation-only option is exactly the kind
    /// the *tool* must offer, since it is the only chance to set it. It is the
    /// inspector that leaves it out (spec.md 6.1).
    pub creation_only: bool,
    /// What makes this option inert, if anything.
    pub depends_on: Vec<OptionDependency>,
    /// Edited by a centred slider rather than a typed number (M29).
    pub slider: Option<crate::document::SliderView>,
}

/// Which gesture drives a tool.
///
/// Nearly always one, but the shape fill's depends on what it is drawing: a
/// freehand polygon is a ring of placed vertices and a preset is a drag. The
/// mapping lives here so the frontend cannot send a gesture `create_object`
/// would refuse.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "GestureSelector.ts")]
pub enum GestureSelector {
    /// The tool is always drawn the same way.
    Always {
        /// The gesture's tag, as `Gesture` spells it.
        gesture: String,
    },
    /// A choice property decides.
    ByChoice {
        /// The property that decides.
        on: String,
        /// One gesture tag per variant of `on`, in variant order.
        gestures: Vec<String>,
    },
}

/// How a gesture with this tool previews itself (spec.md 6.1).
///
/// Most tools carry a field of their own, so the preview draws it: the swept
/// region in the speed colour with direction glyphs over it. The ones defined
/// against what is already beneath them do not. The mask writes calm, the
/// clone stamp reads the composite, and a modifier transforms it (spec.md
/// 6.3) — for all three, what the gesture *paints* is decided by what is
/// already there, and a preview that drew a flat colour would be showing
/// something the tool does not do.
///
/// The two operators are previewed by operating on the map itself rather than
/// by drawing over it, which is the only way to show a removal at all: the
/// overlay is a canvas stacked above the field and can add pixels, never take
/// them away. A modifier is placed by a click rather than dragged, so its
/// preview is only the footprint a click would produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "PreviewKind.ts")]
pub enum PreviewKind {
    /// Draw the field the gesture carries.
    Field,
    /// Take the field away where the gesture covers: the mask.
    Mask,
    /// Show the field from the source, where the gesture covers.
    Clone,
    /// Draw the footprint and nothing inside it.
    ///
    /// The modifiers (spec.md 6.3). What one of them produces is whatever was
    /// beneath it, changed — there is no colour that stands for "the same wind,
    /// half as fast", and the honest preview of a modifier is where it will
    /// land. They are placed by a click rather than dragged, so what the map
    /// shows a moment later is the answer itself.
    Outline,
}

/// The km/px control a tool offers, and when it is live.
///
/// The unit *is* the `stamp_space` control (spec.md 3.5): px asks for a shape
/// on the map and km for one on the ground, so offering the space beside the
/// unit would be two controls for one property, and the space would be the one
/// that did nothing. `stamp_space` is therefore not among a tool's options, and
/// this stands in its place — carrying the same dependency rules, so a control
/// the mode makes inert is still hidden.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "Sizing.ts")]
pub struct Sizing {
    /// What makes the unit inert, if anything.
    ///
    /// The shape fill's freehand polygon is the case: its vertices are placed
    /// geographically one by one, so it has no size and no space to size it in.
    pub depends_on: Vec<OptionDependency>,
}

/// The eyedropper a tool offers, on the wire (spec.md 6.1).
///
/// Two property names and the conditions that make them meaningful. The bar
/// renders a button from this and nothing else, so a tool gains an eyedropper
/// by declaring one in the schema.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "EyedropperSpec.ts")]
pub struct EyedropperSpec {
    /// The speed option it writes, in m/s.
    pub speed: String,
    /// The direction option it writes, as an azimuth-toward.
    pub direction: String,
    /// What makes it inert, in the same terms an option's dependencies use.
    pub depends_on: Vec<OptionDependency>,
}

/// Everything the palette and the option bar need for one tool.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ToolSchema.ts")]
pub struct ToolSchema {
    /// Which tool.
    pub tool: Tool,
    /// Display name.
    pub label: String,
    /// Keyboard shortcut, a single lowercase letter.
    pub shortcut: String,
    /// How it is drawn.
    pub gesture: GestureSelector,
    /// Whether a click has a footprint worth previewing under the cursor.
    ///
    /// False for the shape fill and the curve, deliberately and not by omission
    /// (spec.md 6.2): both are built up point by point, so a single click
    /// produces nothing to show.
    pub hover: bool,
    /// How a gesture with this tool previews itself.
    pub preview: PreviewKind,
    /// The unit control, when the tool's shapes are measured at all.
    ///
    /// `None` for a tool with nothing to measure. `Some` for the shape fill as
    /// well as the typed ones: its presets are dragged out rather than typed,
    /// but a drag is still a measurement and is still made either on the ground
    /// or on the map.
    pub sizing: Option<Sizing>,
    /// Sampling the field for a constant speed and direction, where the tool
    /// paints one.
    pub eyedropper: Option<EyedropperSpec>,
    /// Its options, in the order the bar should show them.
    pub options: Vec<ToolOptionSpec>,
}

fn unit_name(unit: Unit) -> &'static str {
    match unit {
        Unit::None => "none",
        Unit::Speed => "speed",
        Unit::Kilometres => "kilometres",
        Unit::Degrees => "degrees",
        Unit::Direction => "direction",
        Unit::Percent => "percent",
    }
}

/// The gesture, or gestures, a tool is drawn with.
fn gesture_for(tool: ToolKind) -> GestureSelector {
    match tool {
        ToolKind::Brush
        | ToolKind::Mask
        | ToolKind::CloneStamp
        // The modifiers are painted too, so that a swathe can be treated in one
        // gesture and two strokes of one merge (spec.md 6.3).
        | ToolKind::Intensity
        | ToolKind::Divergence
        | ToolKind::Turn
        | ToolKind::Warp
        | ToolKind::Liquify => GestureSelector::Always {
            gesture: "stroke".to_owned(),
        },
        // A patch is pasted rather than drawn, so it has no gesture at all.
        // Named here rather than left to a wildcard: the compiler is what
        // makes a new tool declare how it is drawn.
        // The macro is placed by a click, like the circle; the patch is
        // pasted and has no gesture at all. Named rather than left to a
        // wildcard: the compiler is what makes a new tool declare how it is
        // drawn.
        ToolKind::Circle | ToolKind::Patch | ToolKind::Macro => GestureSelector::Always {
            gesture: "point".to_owned(),
        },
        ToolKind::Curve => GestureSelector::Always {
            gesture: "path".to_owned(),
        },
        // Variant 0 is the freehand polygon, placed vertex by vertex; the three
        // presets are dragged out from their centre.
        ToolKind::ShapeFill => GestureSelector::ByChoice {
            on: format!("{:?}", PropId::ShapeSource),
            gestures: vec![
                "ring".to_owned(),
                "extent".to_owned(),
                "extent".to_owned(),
                "extent".to_owned(),
            ],
        },
    }
}

/// How a gesture with this tool previews itself (spec.md 6.1).
fn preview_for(tool: ToolKind) -> PreviewKind {
    match tool {
        ToolKind::Mask => PreviewKind::Mask,
        ToolKind::CloneStamp => PreviewKind::Clone,
        // A modifier has no field of its own to show (spec.md 6.3).
        ToolKind::Intensity
        | ToolKind::Divergence
        | ToolKind::Turn
        | ToolKind::Warp
        | ToolKind::Liquify => PreviewKind::Outline,
        // Everything else paints a field of its own, which is what its gesture
        // shows. The patch's is a captured one and it has no gesture at all,
        // but it is a field, and `operator_outlines` keys off this: anything
        // that is not `Field` is expected to be in that list (spec.md 6.3).
        ToolKind::Brush
        | ToolKind::Circle
        | ToolKind::ShapeFill
        | ToolKind::Curve
        | ToolKind::Patch
        | ToolKind::Macro => PreviewKind::Field,
    }
}

/// Whether a click with this tool has a footprint to preview (spec.md 6.2).
fn has_hover(tool: ToolKind) -> bool {
    match tool {
        ToolKind::Brush
        | ToolKind::Circle
        | ToolKind::Mask
        | ToolKind::CloneStamp
        // A modifier sweeps a stamp like the brush, so a click has exactly one
        // footprint and the cursor can show it.
        | ToolKind::Intensity
        | ToolKind::Divergence
        | ToolKind::Turn
        | ToolKind::Warp
        | ToolKind::Liquify => true,
        // A curve is built node by node, so a click produces no *object* to
        // preview — but it is swept with a stamp, and the nib says how wide
        // (M24). The hover is the nib, a one-point sweep at the pointer.
        ToolKind::Curve => true,
        // Stated, not omitted: a polygon is built vertex by vertex, so a
        // single click produces no footprint to show, and a patch is never
        // under the cursor before it exists.
        ToolKind::ShapeFill | ToolKind::Patch => false,
        // A macro's footprint is the library entry's shape, which the bar
        // knows and the backend does not until the click lands.
        ToolKind::Macro => false,
    }
}

/// The palette shortcut for a tool.
///
/// The letters spec 8.1 assigns. Two are the conventions of other paint
/// applications rather than initials: `P` is the brush (the *pen*), and `B` is
/// the Bézier curve. Taken from the spec rather than chosen here, so the manual
/// and the application agree.
fn shortcut_for(tool: ToolKind) -> &'static str {
    match tool {
        ToolKind::Brush => "p",
        ToolKind::Circle => "c",
        ToolKind::ShapeFill => "f",
        ToolKind::Mask => "e",
        ToolKind::CloneStamp => "s",
        ToolKind::Curve => "b",
        // The modifiers, by initial where the letter was free: intensify,
        // diverge, rotate, warp.
        ToolKind::Intensity => "i",
        ToolKind::Divergence => "d",
        ToolKind::Turn => "r",
        ToolKind::Warp => "w",
        // The smear (D54).
        ToolKind::Liquify => "l",
        // Not in the palette, so it has no key. Named rather than left to a
        // wildcard, so a tool that *is* added to the palette cannot ship
        // without one.
        ToolKind::Patch => "",
        // The insert tool's key (D54); the object itself is not in the
        // palette, and this is what the insert tool is bound to.
        ToolKind::Macro => "n",
    }
}

/// What makes one of `tool`'s properties inert, in wire form.
fn dependencies_of(tool: ToolKind, prop: PropId) -> Vec<OptionDependency> {
    schema::dependencies(tool)
        .iter()
        .filter(|rule| rule.prop == prop)
        .map(|rule| OptionDependency {
            on: format!("{:?}", rule.on),
            live_for: rule.live_for.to_vec(),
        })
        .collect()
}

/// Describes one tool.
fn describe(tool: ToolKind) -> ToolSchema {
    let options = schema::tool_specs(tool)
        .iter()
        // `stamp_space` is not offered as an option of its own. It asks exactly
        // the question the size's unit already asks — px is a shape on the map,
        // km one on the ground (spec.md 3.5) — and a bar that offered both
        // would have two controls for one property, one of which does nothing.
        // The unit is the control; [`ToolSchema::sized`] says the tool has one.
        .filter(|spec| spec.id != PropId::StampSpace)
        // A warp's push is measured from its own anchor, which does not exist
        // until the gesture does — so there is nothing "push to" could mean on
        // the bar, before there is anything to push. It is set by pulling the
        // warp afterwards (spec.md 6.3) and edited in the inspector, like every
        // other property of an object that exists.
        .filter(|spec| spec.id != PropId::PushTo)
        .chain(
            // The one common property a tool bar owns. The rest of the common
            // set is the object's placement and lifetime, which a gesture
            // decides and the inspector edits; `edge_mode` is a choice about
            // how the gesture paints, so it belongs beside the tool's own
            // options — and a modifier, which has no field to paint, does not
            // have one to offer (spec.md 6.3).
            schema::common_specs(tool)
                .iter()
                .filter(|s| s.id == PropId::EdgeMode),
        )
        .map(|spec| ToolOptionSpec {
            property: format!("{:?}", spec.id),
            label: spec.label.to_owned(),
            unit: unit_name(spec.unit).to_owned(),
            default: PropertyValue::of(spec.default.value()),
            min: spec.range.map(|(min, _)| min),
            max: spec.range.map(|(_, max)| max),
            variants: spec.variants.iter().map(|v| (*v).to_owned()).collect(),
            creation_only: spec.creation_only,
            depends_on: dependencies_of(tool, spec.id),
            slider: spec.slider.map(crate::document::SliderView::of),
        })
        .collect();

    ToolSchema {
        tool: Tool::of(tool),
        label: tool.label().to_owned(),
        shortcut: shortcut_for(tool).to_owned(),
        gesture: gesture_for(tool),
        hover: has_hover(tool),
        preview: preview_for(tool),
        // Declaring a stamp space is what it means for a tool's shapes to be
        // measured, whether the measurement is typed or dragged out.
        sizing: schema::spec_for(tool, PropId::StampSpace).map(|_| Sizing {
            depends_on: dependencies_of(tool, PropId::StampSpace),
        }),
        eyedropper: schema::eyedropper(tool).map(|dropper| EyedropperSpec {
            speed: format!("{:?}", dropper.speed),
            direction: format!("{:?}", dropper.direction),
            // What already makes either property inert makes the eyedropper
            // inert too — it writes both, so it is only meaningful where both
            // are. Stated once here rather than re-derived in the bar.
            depends_on: dependencies_of(tool, dropper.speed)
                .into_iter()
                .chain(dependencies_of(tool, dropper.direction))
                .chain(dropper.only_when.iter().map(|rule| OptionDependency {
                    on: format!("{:?}", rule.on),
                    live_for: rule.live_for.to_vec(),
                }))
                .fold(Vec::new(), |mut kept: Vec<OptionDependency>, rule| {
                    if !kept.contains(&rule) {
                        kept.push(rule);
                    }
                    kept
                }),
        }),
        options,
    }
}

/// Every tool, in palette order.
#[tauri::command]
pub fn tool_palette() -> Result<Vec<ToolSchema>> {
    Ok(palette())
}

/// Implementation of [`tool_palette`], callable without a Tauri handle.
pub fn palette() -> Vec<ToolSchema> {
    ToolKind::ALL
        .iter()
        .copied()
        .filter(|tool| tool.in_palette())
        .map(describe)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every property a tool has must be *settable at creation*, or a
    /// creation-only one becomes unreachable — settable nowhere and editable
    /// nowhere, which spec 6.1 says does not belong on the tool at all.
    ///
    /// `stamp_space` is settable without being listed: the size's unit is its
    /// control, and a tool that has the property must therefore have the unit.
    /// That is the whole of the exception, and it is checked rather than
    /// assumed.
    #[test]
    fn every_tool_property_is_settable_at_creation() {
        for described in palette() {
            let tool = described.tool.kind();
            for spec in schema::tool_specs(tool) {
                if spec.id == PropId::StampSpace {
                    assert!(
                        described.sizing.is_some(),
                        "{tool:?} has a stamp space but no unit to select it with"
                    );
                    continue;
                }
                // A warp's push is measured from an anchor the gesture has not
                // placed yet, so it is the one property that cannot be set
                // before the object exists (spec.md 6.3). Named here so that a
                // second such property has to be a decision rather than an
                // omission.
                if spec.id == PropId::PushTo {
                    assert_eq!(tool, ToolKind::Warp);
                    continue;
                }
                assert!(
                    described
                        .options
                        .iter()
                        .any(|o| o.property == format!("{:?}", spec.id)),
                    "{tool:?} does not offer {:?}",
                    spec.id
                );
            }
        }
    }

    /// The converse: a tool that says it is sized must actually have the
    /// property its unit selects, or the bar would offer a control that writes
    /// nowhere.
    #[test]
    fn a_sized_tool_has_a_stamp_space_to_select() {
        for described in palette() {
            let tool = described.tool.kind();
            assert_eq!(
                described.sizing.is_some(),
                schema::spec_for(tool, PropId::StampSpace).is_some(),
                "{tool:?}: offering a unit and having a stamp space are one claim"
            );
        }
    }

    /// The two tools with no field of their own are exactly the two that are
    /// defined against what is beneath them — the mask writes calm and the
    /// clone stamp reads the composite. A tool that carried a speed *and*
    /// previewed as an operator would be claiming both.
    #[test]
    fn a_tool_previews_as_an_operator_exactly_when_it_has_no_speed_of_its_own() {
        for described in palette() {
            let tool = described.tool.kind();
            let carries_a_field = schema::spec_for(tool, PropId::Speed).is_some()
                || schema::spec_for(tool, PropId::SpeedMin).is_some();
            assert_eq!(
                described.preview == PreviewKind::Field,
                carries_a_field,
                "{tool:?}: previewing a field and having one are the same claim"
            );
        }
    }

    /// Spec 6.1: the eyedropper is offered by the tools that paint a single
    /// vector, and it writes the two properties that vector is made of. Named
    /// rather than counted: each is a claim about that tool.
    #[test]
    fn the_eyedropper_is_offered_where_a_single_vector_is_painted() {
        let offered: Vec<(ToolKind, String, String)> = palette()
            .into_iter()
            .filter_map(|described| {
                described
                    .eyedropper
                    .map(|dropper| (described.tool.kind(), dropper.speed, dropper.direction))
            })
            .collect();
        assert_eq!(
            offered,
            vec![
                (ToolKind::Brush, "Speed".to_owned(), "Direction".to_owned()),
                (
                    ToolKind::ShapeFill,
                    "Speed".to_owned(),
                    "Direction".to_owned()
                ),
                (ToolKind::Curve, "Speed".to_owned(), "Direction".to_owned()),
            ],
            "the circle's flow is tangential and has no bearing to take; the \
             mask, the clone stamp and the modifiers have no field of their own"
        );
    }

    /// ...and it is inert wherever either property it writes is. Otherwise a
    /// click would set a number the tool does not read — the same trap an
    /// option with no dependency rule is.
    #[test]
    fn the_eyedropper_is_inert_wherever_what_it_writes_is() {
        for described in palette() {
            let Some(dropper) = &described.eyedropper else {
                continue;
            };
            let tool = described.tool.kind();
            for prop in [&dropper.speed, &dropper.direction] {
                let id = schema::all_specs(tool)
                    .map(|spec| spec.id)
                    .find(|id| format!("{id:?}") == *prop)
                    .unwrap_or_else(|| panic!("{tool:?}: {prop} is not one of its options"));
                for rule in dependencies_of(tool, id) {
                    assert!(
                        dropper.depends_on.contains(&rule),
                        "{tool:?}: {prop} is inert on {}, but the eyedropper is not",
                        rule.on
                    );
                }
            }
        }
    }

    /// A clone stamp needs somewhere to read from for its preview to mean
    /// anything, and the mask needs nothing at all — which is the difference
    /// between the two operators.
    #[test]
    fn only_the_clone_reads_from_somewhere() {
        for described in palette() {
            let tool = described.tool.kind();
            if described.preview == PreviewKind::Clone {
                assert!(
                    schema::spec_for(tool, PropId::SourcePoint).is_some(),
                    "{tool:?} previews a clone but has no source to read"
                );
            }
        }
    }

    /// Spec 3.5 and 6.1: one question, one control. The unit says whether a
    /// shape is measured on the map or on the ground, so a `stamp_space`
    /// dropdown beside it would be a second control for the same property —
    /// and the loser of the two, since the unit is what the gesture freezes.
    #[test]
    fn no_tool_offers_a_stamp_space_option() {
        for described in palette() {
            assert!(
                !described
                    .options
                    .iter()
                    .any(|o| o.property == format!("{:?}", PropId::StampSpace)),
                "{:?} offers a stamp space beside its unit",
                described.tool
            );
        }
    }

    /// The gesture a tool declares has to be one `create_object` accepts, and
    /// a per-variant mapping has to cover every variant — a short list would
    /// leave a mode with no gesture at all.
    #[test]
    fn every_declared_gesture_is_one_the_write_path_knows() {
        const KNOWN: [&str; 5] = ["stroke", "point", "extent", "ring", "path"];

        for described in palette() {
            let tool = described.tool.kind();
            match &described.gesture {
                GestureSelector::Always { gesture } => {
                    assert!(KNOWN.contains(&gesture.as_str()), "{tool:?}: {gesture}");
                }
                GestureSelector::ByChoice { on, gestures } => {
                    let spec = schema::tool_specs(tool)
                        .iter()
                        .find(|s| format!("{:?}", s.id) == *on)
                        .unwrap_or_else(|| panic!("{tool:?} has no property {on}"));
                    assert_eq!(
                        gestures.len(),
                        spec.variants.len(),
                        "{tool:?}: {} gestures for {} variants of {on}",
                        gestures.len(),
                        spec.variants.len()
                    );
                    for gesture in gestures {
                        assert!(KNOWN.contains(&gesture.as_str()), "{tool:?}: {gesture}");
                    }
                }
            }
        }
    }

    /// The letters spec 8.1 assigns, restated so a change to one has to be a
    /// change to both. `V` is the hand's, `M` the select tool's and `T` the
    /// measure tool's (D54); none may be taken here.
    #[test]
    fn the_palette_shortcuts_are_the_ones_the_spec_assigns() {
        let shortcuts: Vec<(ToolKind, String)> = palette()
            .into_iter()
            .map(|d| (d.tool.kind(), d.shortcut))
            .collect();
        assert_eq!(
            shortcuts,
            vec![
                (ToolKind::Brush, "p".to_owned()),
                (ToolKind::Circle, "c".to_owned()),
                (ToolKind::ShapeFill, "f".to_owned()),
                (ToolKind::CloneStamp, "s".to_owned()),
                (ToolKind::Curve, "b".to_owned()),
                (ToolKind::Mask, "e".to_owned()),
                (ToolKind::Intensity, "i".to_owned()),
                (ToolKind::Divergence, "d".to_owned()),
                (ToolKind::Turn, "r".to_owned()),
                (ToolKind::Warp, "w".to_owned()),
                (ToolKind::Liquify, "l".to_owned()),
            ]
        );
    }

    /// ...and distinct from each other and from the keys the palette does not
    /// own, or one of them silently never fires.
    #[test]
    fn the_palette_shortcuts_are_all_different() {
        // `v` is the hand's; `m` is reserved for the measure tool (spec 8.1).
        let mut seen = vec!["v".to_owned(), "m".to_owned()];
        for described in palette() {
            assert!(
                !seen.contains(&described.shortcut),
                "{:?} reuses the shortcut {}",
                described.tool,
                described.shortcut
            );
            seen.push(described.shortcut);
        }
    }

    /// The dependency rules the bar resolves must name options the bar has, or
    /// it would be resolving them against a value it never holds and would
    /// treat the option as inert forever. The unit's rules are checked with the
    /// rest: they are inherited from the stamp space it stands in for, and a
    /// rule keyed on something the bar cannot see would hide the unit always.
    #[test]
    fn every_dependency_names_an_option_the_bar_holds() {
        for described in palette() {
            let holds = |name: &str| described.options.iter().any(|o| o.property == name);
            for option in &described.options {
                for rule in &option.depends_on {
                    assert!(
                        holds(&rule.on),
                        "{:?}: {} depends on {}, which the bar does not hold",
                        described.tool,
                        option.property,
                        rule.on
                    );
                }
            }
            for rule in described.sizing.iter().flat_map(|s| &s.depends_on) {
                assert!(
                    holds(&rule.on),
                    "{:?}: the unit depends on {}, which the bar does not hold",
                    described.tool,
                    rule.on
                );
            }
        }
    }

    /// The unit stands in for `stamp_space`, so it must be live in exactly the
    /// modes the property is read in. The shape fill is the case that matters:
    /// a freehand polygon has no size, so it must not be offered a unit.
    #[test]
    fn the_unit_is_live_exactly_where_the_stamp_space_is() {
        let fill = palette()
            .into_iter()
            .find(|d| d.tool == Tool::ShapeFill)
            .expect("the shape fill is in the palette");
        let sizing = fill.sizing.expect("the shape fill measures its presets");

        let live_for = |index: u8| {
            sizing
                .depends_on
                .iter()
                .all(|rule| rule.live_for.contains(&index))
        };
        // Shape source 0 is the freehand polygon; 1 to 3 are the presets.
        assert!(
            !live_for(0),
            "a polygon was offered a unit it has no size for"
        );
        for preset in 1..=3 {
            assert!(live_for(preset), "preset {preset} was not offered a unit");
        }
    }
}
