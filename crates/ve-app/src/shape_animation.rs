//! Shape editing uses the evaluated footprint and placement used by rendering.

use serde::Serialize;
use ts_rs::TS;
use ve_core::shape_animation::ShapeAnimation;
use ve_core::{Command, LocalPoint, Object, Project};
use ve_render::scene::{FlatObject, flatten_object_at, place};
use ve_render::sdf::Shape;

use crate::animation::{InterpolationView, KeyframeView, TrackView};
use crate::commands::AppState;
use crate::document::{PropertyValue, object_id, pointed_at};
use crate::error::{AppError, Result};
use crate::projects::{ProjectSummary, with_session};

/// Perimeter controls in geographic coordinates, only requested in edit mode.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "ShapeControls.ts")]
pub struct ShapeControls {
    /// The object being edited.
    pub object: u64,
    /// The sampled project step.
    pub step: u32,
    /// Revision used to reject a stale drag after undo or another edit.
    pub revision: u64,
    /// Closed contours of [longitude, latitude] controls.
    pub rings: Vec<Vec<[f64; 2]>>,
    /// Whether each point has its own key at this frame.
    pub keyed: Vec<Vec<bool>>,
}

fn bad(value: impl Into<String>) -> AppError {
    AppError::BadOption {
        field: "shape",
        value: value.into(),
    }
}

fn target(project: &Project, object: u64) -> Result<&Object> {
    let layer = project
        .layers
        .iter()
        .find(|l| l.objects.iter().any(|o| o.id.raw() == object))
        .ok_or(AppError::Core(ve_core::CoreError::MissingObject(object)))?;
    pointed_at(layer)?;
    if layer.locked {
        return Err(bad(format!("{} is locked", layer.name)));
    }
    let target = project
        .object(object_id(object))
        .ok_or(AppError::Core(ve_core::CoreError::MissingObject(object)))?;
    if !target.tool.can_animate_shape() {
        return Err(bad("macros and patches cannot have shape keyframes"));
    }
    Ok(target)
}

fn flat_at(project: &Project, object: &Object, step: u32) -> Result<FlatObject> {
    if step > project.last_step() {
        return Err(bad("frame is outside the project"));
    }
    let kind = project
        .layers
        .iter()
        .find(|l| l.objects.iter().any(|o| o.id == object.id))
        .map(|l| l.parameter())
        .unwrap_or(project.settings.field_kind);
    let links = ve_core::follow::resolve(project, step);
    let placed = place(project, object, kind, step, &links);
    // Timeline geometry remains editable outside the coarse lifetime, just
    // like position keys. This clone does not change the object's visibility.
    let mut editable = object.clone();
    editable.active_range = ve_core::StepRange::full(project.settings.step_count);
    editable.props.insert(
        ve_core::PropId::Enabled,
        ve_core::Animatable::constant(ve_core::PropValue::Bool(true)),
    );
    flatten_object_at(&editable, step, placed.derived)
        .ok_or_else(|| bad("this object has no perimeter"))
}

fn prepared(object: &Object, flat: &FlatObject) -> Result<ShapeAnimation> {
    if let Some(animation) = &object.shape_animation {
        return Ok(animation.clone());
    }
    let rings = ve_render::perimeter::rings(&flat.shape);
    if rings.is_empty() {
        return Err(bad("this object has no perimeter"));
    }
    Ok(ShapeAnimation::new(
        rings
            .iter()
            .map(|r| r.iter().map(|p| LocalPoint::new(p[0], p[1])).collect())
            .collect(),
        flat.shape.animation_reference_m(),
    ))
}

/// Reads controls without changing the document or adding history.
#[tauri::command]
pub fn shape_controls(
    state: tauri::State<'_, AppState>,
    object: u64,
    step: u32,
) -> Result<ShapeControls> {
    controls_of(&state, object, step)
}

/// Non-IPC implementation of [`shape_controls`].
pub fn controls_of(state: &AppState, object: u64, step: u32) -> Result<ShapeControls> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let obj = target(&open.project, object)?;
        let flat = flat_at(&open.project, obj, step)?;
        let animation = prepared(obj, &flat)?;
        let scale = contour_scale(&animation, &flat);
        let rings = animation
            .at(step)
            .iter()
            .map(|r| {
                r.iter()
                    .map(|p| {
                        let q = flat
                            .frame
                            .to_global(flat.shape.rescale_perimeter_point([p.x, p.y], scale));
                        [q.lon, q.lat]
                    })
                    .collect()
            })
            .collect();
        Ok(ShapeControls {
            object,
            step,
            revision: open.revision,
            rings,
            keyed: animation
                .rings
                .iter()
                .map(|r| r.iter().map(|p| p.keys.contains_key(&step)).collect())
                .collect(),
        })
    })
}

fn contour_scale(animation: &ShapeAnimation, flat: &FlatObject) -> f64 {
    let source = match &flat.shape {
        Shape::Contours { source, .. } => source.as_ref(),
        s => s,
    };
    source.animation_reference_m() / animation.reference_size_m.max(1e-9)
}

/// Moves one perimeter point, creating only that point's key at this step.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn move_shape_point(
    state: tauri::State<'_, AppState>,
    object: u64,
    step: u32,
    ring: usize,
    point: usize,
    lon: f64,
    lat: f64,
    revision: u64,
) -> Result<ProjectSummary> {
    move_point(&state, object, step, ring, point, [lon, lat], revision)
}

/// Non-IPC implementation of [`move_shape_point`].
pub fn move_point(
    state: &AppState,
    object: u64,
    step: u32,
    ring: usize,
    point: usize,
    position: [f64; 2],
    revision: u64,
) -> Result<ProjectSummary> {
    let geo = ve_core::LonLat::new(position[0], position[1])?;
    rewrite(
        state,
        object,
        step,
        None,
        |animation, flat, first, current| {
            if revision != current {
                return Err(bad("the document changed during this drag; try again"));
            }
            let scale = contour_scale(animation, flat);
            if scale <= 1e-9 {
                return Err(bad(
                    "increase the object's size before editing its perimeter",
                ));
            }
            let local = flat
                .shape
                .rescale_perimeter_point(flat.frame.to_local(geo), 1.0 / scale);
            let vertex = animation
                .rings
                .get_mut(ring)
                .and_then(|r| r.get_mut(point))
                .ok_or_else(|| bad("the point no longer exists"))?;
            if vertex.keys.is_empty() && step > first {
                vertex.set(first, vertex.base);
            }
            vertex.set(step, LocalPoint::new(local[0], local[1]));
            Ok(())
        },
    )
}

fn rewrite(
    state: &AppState,
    object: u64,
    step: u32,
    gesture: Option<String>,
    change: impl FnOnce(&mut ShapeAnimation, &FlatObject, u32, u64) -> Result<()>,
) -> Result<ProjectSummary> {
    with_session(state, |session| {
        let open = session.require_open()?;
        let obj = target(&open.project, object)?;
        let flat = flat_at(&open.project, obj, step)?;
        let before = obj.shape_animation.clone();
        let mut animation = prepared(obj, &flat)?;
        change(
            &mut animation,
            &flat,
            obj.active_range.start.min(step),
            open.revision,
        )?;
        let after = Some(animation);
        if before != after {
            let command = Command::SetShapeAnimation {
                object: object_id(object),
                before,
                after,
            };
            if let Some(key) = gesture {
                open.history
                    .push_coalesced(&mut open.project, command, key)?;
            } else {
                open.history.push(&mut open.project, command)?;
            }
            open.touch();
        }
        Ok(ProjectSummary::of(open))
    })
}

/// Pins every point at the current evaluated position.
pub fn key_at(state: &AppState, object: u64, step: u32) -> Result<ProjectSummary> {
    rewrite(state, object, step, None, |a, _, _, _| {
        for p in a.rings.iter_mut().flatten() {
            p.set(step, p.at(step));
        }
        Ok(())
    })
}

/// Removes all point keys on this shape's aggregate marker.
pub fn unkey_at(state: &AppState, object: u64, step: u32) -> Result<ProjectSummary> {
    rewrite(state, object, step, None, |a, _, _, _| {
        for p in a.rings.iter_mut().flatten() {
            p.keys.remove(&step);
        }
        Ok(())
    })
}

/// Moves the point keys represented by an aggregate marker.
pub fn move_key(
    state: &AppState,
    object: u64,
    from: u32,
    to: u32,
    gesture: Option<String>,
) -> Result<ProjectSummary> {
    rewrite(state, object, to, gesture, |a, _, _, _| {
        if !a.keys().contains_key(&from) {
            return Err(bad("there is no shape key at that frame"));
        }
        for p in a.rings.iter_mut().flatten() {
            if let Some(k) = p.keys.remove(&from) {
                p.keys.insert(to, k);
            }
        }
        Ok(())
    })
}

/// Eases all points keyed on this marker.
pub fn ease_from(
    state: &AppState,
    object: u64,
    step: u32,
    interp: InterpolationView,
) -> Result<ProjectSummary> {
    rewrite(state, object, step, None, |a, _, _, _| {
        if !a.keys().contains_key(&step) {
            return Err(bad("there is no shape key at that frame"));
        }
        for p in a.rings.iter_mut().flatten() {
            if let Some(k) = p.keys.get_mut(&step) {
                k.interp = interp.into_model();
            }
        }
        Ok(())
    })
}

/// Shape uses the timeline's existing diamond editing, deletion and easing.
pub fn track(object: &Object, step: u32) -> TrackView {
    let keys = object
        .shape_animation
        .as_ref()
        .map(|a| a.keys())
        .unwrap_or_default();
    let keyed_here = keys.contains_key(&step);
    let interpolated_here = !keyed_here
        && object.shape_animation.as_ref().is_some_and(|a| {
            a.rings.iter().flatten().any(|p| {
                p.keys
                    .range(..step)
                    .next_back()
                    .is_some_and(|(_, k)| k.interp != ve_core::Interpolation::Step)
                    && p.keys.range(step..).next().is_some()
            })
        });
    TrackView {
        property: "shape".into(),
        label: "Shape".into(),
        base: PropertyValue::Bool { value: true },
        keys: keys
            .into_iter()
            .map(|(step, interp)| KeyframeView {
                step,
                value: PropertyValue::Bool { value: true },
                interp: InterpolationView::of(interp),
            })
            .collect(),
        interpolations: vec![
            InterpolationView::Linear,
            InterpolationView::Step,
            InterpolationView::EaseIn,
            InterpolationView::EaseOut,
            InterpolationView::EaseInOut,
        ],
        keyed_here,
        interpolated_here,
        motion_available: false,
        motion: false,
        can_follow: false,
        follows: None,
        follows_name: None,
        inherited: Vec::new(),
    }
}
