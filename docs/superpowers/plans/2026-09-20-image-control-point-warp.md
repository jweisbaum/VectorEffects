# Image control-point warp implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Georeference an image layer by clicking pairs of points — one in the picture, one on the map — so the picture bends to put every pair exactly where it was placed, in every projection.

**Architecture:** The pairs are stored on the document; the warp is derived from them and never serialised. The fit is a homography base with a thin-plate-spline residual, chosen by pair count. Rendering evaluates the warp per mesh vertex in Rust when the pairs change — never per camera — and the vertex shader only projects what it is handed, so every projection works without a special case.

**Tech Stack:** Rust (`ve-core`, `ve-app`), WebGL2 / GLSL ES 3.00, React + TypeScript. No new crate dependencies — the dense solver is written here, because a linear-algebra crate for one 53×53 system is not worth the dependency and several bring a `-sys` (invariant 5).

**Spec:** `docs/superpowers/specs/2026-09-20-image-control-point-warp-design.md`

## Global Constraints

Copied from `CLAUDE.md` and `spec.md`; every task's requirements include these.

- **No `unwrap()` or `expect()` in `ve-core`/`ve-render`/`ve-grib` outside tests.** `thiserror` in library crates, `anyhow` at the app boundary, `AppError` across IPC.
- **Every `f64` that reaches a project file needs a `canonical::*_field` serde helper.** A passing round-trip does not prove otherwise: the workspace build has `serde_json/float_roundtrip` on by coincidence of the dependency graph.
- **Longitude is `[-180, 180)` everywhere**; only `ve-grib` converts to `[0, 360)`.
- **Image placement is in degrees, not metres** — the map's own coordinates, equirectangular, because it is stored and cannot depend on the view (spec §4.9, §3.5).
- **An image layer is display only.** It reaches no scene, no `FlatScene` hash, no render-cache key and no export. Nothing in this plan may change that.
- **Determinism:** no `HashMap` iteration in any evaluation path. `Vec` or `IndexMap`.
- **Frontend types are generated from Rust**, never hand-written. Run `npm run bindings` after changing any IPC-facing type.
- **Nothing that changes per pointer move may be `MapView` state** — use an external store, as `createReadoutStore` does.
- **The 2D overlay is drawn inside `draw()`**; overlay-only changes go through `requestOverlay`, never a synchronous `drawOverlay`.
- **Definition of done, every task:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `VE_FORCE_CPU=1 cargo test --workspace`, `npm run ui:typecheck`, `npm run ui:test`, `npm run check:offline`.

**Checkpoint:** Tasks 1–5 are the whole backend and are independently valuable — after Task 5 the warp is real, tested, undoable and reachable over IPC, with no UI. Tasks 6–9 put it on screen.

---

### Task 1: The control point on the document

**Files:**
- Modify: `crates/ve-core/src/document.rs` (the `Image` variant at ~line 627, and a new `ControlPoint` beside `Placement` at ~line 744)
- Test: `crates/ve-core/src/io.rs` (`mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces: `ve_core::document::ControlPoint { u: f64, v: f64, lon: f64, lat: f64 }` (all public), and `LayerSource::Image { path, placement, opacity, control_points: Vec<ControlPoint> }`.

- [ ] **Step 1: Write the failing round-trip test**

In `crates/ve-core/src/io.rs`'s `mod tests`:

```rust
/// Control points are document state and must survive a save and a load
/// exactly. The values are chosen to be hostile: each has more digits than
/// a `f64` prints by default, which is what catches a missing canonical
/// helper (see `canonical.rs`).
#[test]
fn control_points_survive_a_round_trip() {
    use crate::document::ControlPoint;
    let points = vec![
        ControlPoint { u: 1234.567891, v: 98.7654321, lon: -70.123456789, lat: 41.987654321 },
        ControlPoint { u: 0.000001, v: 65535.999999, lon: 179.999999999, lat: -89.999999999 },
    ];
    let source = crate::document::LayerSource::Image {
        path: std::path::PathBuf::from("/tmp/chart.png"),
        placement: crate::document::Placement::spanning(-71.0, 42.0, -70.0, 41.0, 800, 600),
        opacity: 0.75,
        control_points: points.clone(),
    };
    let json = serde_json::to_string(&source).expect("serialise");
    let back: crate::document::LayerSource = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(back, source, "a control point did not round-trip");
}

/// An older project has no control points and must open exactly as before.
#[test]
fn a_project_without_control_points_opens_with_none() {
    let json = r#"{"Image":{"path":"/tmp/chart.png",
        "placement":{"a":1.0,"b":0.0,"c":-71.0,"d":0.0,"e":-1.0,"f":42.0},
        "opacity":1.0}}"#;
    let back: crate::document::LayerSource = serde_json::from_str(json).expect("deserialise");
    let crate::document::LayerSource::Image { control_points, .. } = back else {
        panic!("not an image");
    };
    assert!(control_points.is_empty());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p ve-core control_points_survive_a_round_trip`
Expected: FAIL — `ControlPoint` is not defined, and `LayerSource::Image` has no `control_points` field.

- [ ] **Step 3: Add the type and the field**

In `crates/ve-core/src/document.rs`, beside `Placement`:

```rust
/// One pair the user placed: a point in the picture, and where on the earth
/// it belongs (spec.md §4.9).
///
/// `u` and `v` are the image's own pixels, **not** degrees, which is what
/// makes a pair independent of whatever placement was in force when it was
/// made — so pairs accumulate and re-fit without drift. `lon` and `lat` are
/// where that pixel should land.
///
/// The warp is computed from these and never stored: it is derived state,
/// like a render (invariants 1 and 2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ControlPoint {
    /// Across the image, in pixels from the left edge.
    #[serde(with = "crate::canonical::ratio_field")]
    pub u: f64,
    /// Down the image, in pixels from the top edge.
    #[serde(with = "crate::canonical::ratio_field")]
    pub v: f64,
    /// Where it belongs: longitude in degrees.
    #[serde(with = "crate::canonical::degrees_field")]
    pub lon: f64,
    /// Where it belongs: latitude in degrees.
    #[serde(with = "crate::canonical::degrees_field")]
    pub lat: f64,
}
```

In the `Image` variant, after `opacity`:

```rust
        /// The pairs that warp it, in the order they were placed.
        ///
        /// Empty means the `placement` above is used unchanged, which is
        /// what every project made before this existed holds.
        #[serde(default)]
        control_points: Vec<ControlPoint>,
```

Then fix every construction site the compiler names — `ve-app`'s `import.rs`/`image.rs` and any test fixture — by adding `control_points: Vec::new()`.

- [ ] **Step 4: Run to verify both tests pass**

Run: `cargo test -p ve-core control_points && cargo test -p ve-core a_project_without_control_points`
Expected: PASS.

- [ ] **Step 5: Extend the existing hostile-float test**

Add the two control points from Step 1 to the image layer in `io::tests::hostile_floats_survive_a_round_trip`, so the new field is covered by the test the recipe names.

Run: `cargo test -p ve-core hostile_floats`
Expected: PASS.

- [ ] **Step 6: Confirm no migration is needed**

`SCHEMA_VERSION` is **not** bumped and `io::MIGRATIONS` is **not** touched — a new field with `#[serde(default)]` needs neither (recipe: "Adding a field to the document"). `a_project_without_control_points_opens_with_none` is the proof.

Run: `VE_FORCE_CPU=1 cargo test -p ve-core`
Expected: PASS, and `SCHEMA_VERSION` unchanged in the diff.

- [ ] **Step 7: Commit**

```bash
git add crates/ve-core/src/document.rs crates/ve-core/src/io.rs crates/ve-app/src
git commit -m "Store image control points on the document"
```

---

### Task 2: The rigid fits — translation, similarity, affine, homography

**Files:**
- Create: `crates/ve-core/src/warp.rs`
- Modify: `crates/ve-core/src/lib.rs` (add `pub mod warp;`)
- Test: in `warp.rs`'s `mod tests`

**Interfaces:**
- Consumes: `ve_core::document::{ControlPoint, Placement}` from Task 1.
- Produces:
  - `ve_core::warp::Warp` (opaque), with
  - `Warp::fit(points: &[ControlPoint], width: u32, height: u32, base: Placement) -> Warp`
  - `Warp::place(&self, u: f64, v: f64) -> (f64, f64)` — image pixel to lon/lat
  - `Warp::is_identity_affine(&self) -> bool` — true when the warp is expressible as the six-number `Placement`, which Task 6 uses to choose the render path
  - `Warp::as_placement(&self) -> Option<Placement>`
  - `ve_core::warp::MAX_CONTROL_POINTS: usize = 50`
  - `solve(a: &mut [f64], b: &mut [f64], n: usize) -> Option<()>` — private, Gaussian elimination with partial pivoting, row-major `a`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{ControlPoint, Placement};

    fn base() -> Placement {
        // 800x600 pixels over one degree square, north up.
        Placement::spanning(-71.0, 42.0, -70.0, 41.0, 800, 600)
    }

    fn at(u: f64, v: f64, lon: f64, lat: f64) -> ControlPoint {
        ControlPoint { u, v, lon, lat }
    }

    /// The defining property, and the one that means "perfectly": whatever
    /// the pairs, the fitted warp puts each pixel exactly on its target.
    fn assert_exact(points: &[ControlPoint], warp: &Warp) {
        for (n, p) in points.iter().enumerate() {
            let (lon, lat) = warp.place(p.u, p.v);
            assert!(
                (lon - p.lon).abs() < 1e-9 && (lat - p.lat).abs() < 1e-9,
                "pair {n} landed at {lon}, {lat} and not at {}, {}",
                p.lon, p.lat
            );
        }
    }

    /// No pairs is today's behaviour: the stored placement, untouched.
    #[test]
    fn no_pairs_is_the_placement_unchanged() {
        let warp = Warp::fit(&[], 800, 600, base());
        assert_eq!(warp.place(0.0, 0.0), base().place(0.0, 0.0));
        assert_eq!(warp.place(800.0, 600.0), base().place(800.0, 600.0));
        assert!(warp.is_identity_affine());
    }

    /// One pair moves the picture and changes nothing else. Checked against
    /// a second, independent point: it must move by exactly the same offset.
    #[test]
    fn one_pair_is_a_translation() {
        let (lon0, lat0) = base().place(400.0, 300.0);
        let points = [at(400.0, 300.0, lon0 + 0.25, lat0 - 0.5)];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        let (was_lon, was_lat) = base().place(0.0, 0.0);
        let (now_lon, now_lat) = warp.place(0.0, 0.0);
        assert!((now_lon - was_lon - 0.25).abs() < 1e-12);
        assert!((now_lat - was_lat + 0.5).abs() < 1e-12);
    }

    /// Two pairs rotate and scale but never shear: a square stays square.
    /// Checked by the property that defines a similarity — every distance
    /// is scaled by the same factor.
    #[test]
    fn two_pairs_are_a_similarity_and_keep_the_aspect() {
        let points = [
            at(0.0, 0.0, 0.0, 0.0),
            at(800.0, 0.0, 0.0, 1.0), // the top edge now runs north
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        let a = warp.place(0.0, 0.0);
        let b = warp.place(800.0, 0.0);
        let c = warp.place(0.0, 800.0);
        let ab = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let ac = ((c.0 - a.0).powi(2) + (c.1 - a.1).powi(2)).sqrt();
        assert!((ab - ac).abs() < 1e-9, "a square became {ab} by {ac}");
    }

    /// Three points determine an affine exactly. The expected answer here is
    /// arithmetic worked by hand, not a second copy of the fit: the image's
    /// top-left, top-right and bottom-left are sent to three chosen places,
    /// so the centre must land at their mean.
    #[test]
    fn three_pairs_are_the_affine_through_them() {
        let points = [
            at(0.0, 0.0, -10.0, 5.0),
            at(800.0, 0.0, -8.0, 5.0),
            at(0.0, 600.0, -10.0, 3.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        let (lon, lat) = warp.place(400.0, 300.0);
        assert!((lon - -9.0).abs() < 1e-9, "{lon}");
        assert!((lat - 4.0).abs() < 1e-9, "{lat}");
        assert!(warp.is_identity_affine(), "three pairs are still an affine");
    }

    /// Four pairs are a homography, which an affine cannot be: the image's
    /// four corners go to a trapezium. The property that proves it is
    /// projective and not affine — the centre of the picture does NOT land
    /// at the mean of the four targets, but the diagonals still cross there.
    #[test]
    fn four_pairs_are_a_homography_that_no_affine_could_be() {
        let points = [
            at(0.0, 0.0, -2.0, 1.0),
            at(800.0, 0.0, 2.0, 1.0),
            at(800.0, 600.0, 1.0, -1.0),
            at(0.0, 600.0, -1.0, -1.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        // The picture's centre lands where the trapezium's diagonals cross,
        // which for this shape is on the vertical axis and above the mean.
        let (lon, lat) = warp.place(400.0, 300.0);
        assert!(lon.abs() < 1e-9, "the centre should stay on the axis: {lon}");
        let mean_lat = (1.0 + 1.0 - 1.0 - 1.0) / 4.0;
        assert!(lat > mean_lat + 1e-6, "an affine would give {mean_lat}, got {lat}");
        assert!(!warp.is_identity_affine(), "a homography is not an affine");
    }

    /// Pairs on a line cannot determine an area. The fit must say so rather
    /// than return a degenerate warp that collapses the picture.
    #[test]
    fn pairs_on_a_line_fall_back_rather_than_collapsing_the_image() {
        let points = [
            at(0.0, 0.0, 0.0, 0.0),
            at(100.0, 0.0, 1.0, 0.0),
            at(200.0, 0.0, 2.0, 0.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        // Not asserted to be exact — it cannot be — but it must still be a
        // usable warp with area, not a collapse to a line.
        let a = warp.place(0.0, 0.0);
        let b = warp.place(0.0, 600.0);
        assert!((a.1 - b.1).abs() > 1e-6, "the image collapsed to a line");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p ve-core warp::`
Expected: FAIL — `warp` module does not exist.

- [ ] **Step 3: Implement the module**

Create `crates/ve-core/src/warp.rs` with:

- `pub const MAX_CONTROL_POINTS: usize = 50;`
- The dense solver, written here rather than pulled in as a dependency:

```rust
/// Solves `a x = b` in place by Gaussian elimination with partial pivoting.
///
/// `a` is row-major and `n` by `n`; the solution is written back into `b`.
/// `None` means the system is singular — for this module's callers that is
/// control points on a line, or two pairs on the same pixel, and every
/// caller answers it by falling back to a simpler fit rather than failing.
///
/// Written here because the largest system this ever sees is 53 by 53 (the
/// fifty-pair cap plus the spline's three affine terms), which is far too
/// small to justify a linear-algebra dependency — and several of them bring
/// a `-sys` crate, which invariant 5 and the three-platform build forbid.
fn solve(a: &mut [f64], b: &mut [f64], n: usize) -> Option<()> {
    debug_assert_eq!(a.len(), n * n);
    debug_assert_eq!(b.len(), n);
    for column in 0..n {
        // Partial pivoting: the largest remaining magnitude in this column.
        // Without it a zero on the diagonal stops an otherwise fine system,
        // and a small one loses most of its digits.
        let mut pivot = column;
        for row in (column + 1)..n {
            if a[row * n + column].abs() > a[pivot * n + column].abs() {
                pivot = row;
            }
        }
        if a[pivot * n + column].abs() < 1e-12 {
            return None;
        }
        if pivot != column {
            for k in 0..n {
                a.swap(column * n + k, pivot * n + k);
            }
            b.swap(column, pivot);
        }
        let diagonal = a[column * n + column];
        for row in (column + 1)..n {
            let factor = a[row * n + column] / diagonal;
            if factor == 0.0 {
                continue;
            }
            for k in column..n {
                a[row * n + k] -= factor * a[column * n + k];
            }
            b[row] -= factor * b[column];
        }
    }
    // Back-substitution.
    for row in (0..n).rev() {
        let mut sum = b[row];
        for k in (row + 1)..n {
            sum -= a[row * n + k] * b[k];
        }
        b[row] = sum / a[row * n + row];
    }
    Some(())
}
```

  and its own test, because everything above depends on it being right:

```rust
    /// The solver against a system whose answer is known by inspection,
    /// and one that is singular and must say so rather than return noise.
    #[test]
    fn the_solver_solves_and_reports_a_singular_system() {
        // 2x + y = 5, x + 3y = 10  =>  x = 1, y = 3.
        let mut a = [2.0, 1.0, 1.0, 3.0];
        let mut b = [5.0, 10.0];
        solve(&mut a, &mut b, 2).expect("solvable");
        assert!((b[0] - 1.0).abs() < 1e-12, "{b:?}");
        assert!((b[1] - 3.0).abs() < 1e-12, "{b:?}");

        // Two copies of the same equation determine nothing.
        let mut a = [1.0, 2.0, 2.0, 4.0];
        let mut b = [3.0, 6.0];
        assert!(solve(&mut a, &mut b, 2).is_none());

        // A zero in the first pivot position is fine once rows are swapped.
        let mut a = [0.0, 1.0, 1.0, 0.0];
        let mut b = [2.0, 3.0];
        solve(&mut a, &mut b, 2).expect("pivoting handles a zero diagonal");
        assert!((b[0] - 3.0).abs() < 1e-12 && (b[1] - 2.0).abs() < 1e-12, "{b:?}");
    }
```
- `pub struct Warp { kind: Kind }` with `enum Kind { Affine(Placement), Projective([f64; 9]), Spline { projective: [f64; 9], points: Vec<ControlPoint>, weights: Vec<[f64; 2]>, hull_radius: f64 } }` — the `Spline` arm is filled in Task 3 and may be `unreachable!()`-free by simply not being constructed yet.
- `Warp::fit` dispatching on `points.len().min(MAX_CONTROL_POINTS)`:
  - `0` → `Kind::Affine(base)`.
  - `1` → `Kind::Affine` = `base` with `c`/`f` shifted by the residual at that pixel.
  - `2` → similarity solved in closed form from the two pixel-to-degree vectors.
  - `3` → affine solved exactly (a 6×6 through `solve`, or the 2×2 closed form).
  - `4..` → homography: build the 8×8 from the first four (or least squares over all for `n > 4` before Task 3 lands), `solve`, then `Kind::Projective`.
- `Warp::place` evaluating the arm; for `Projective`, `w = g*u + h*v + 1`, and if `w.abs() < 1e-12` return the base's answer rather than dividing by zero.
- `is_identity_affine` / `as_placement` returning true/`Some` only for `Kind::Affine`.

**The degenerate rule:** wherever `solve` returns `None`, fall back to the next simpler fit — homography to affine, affine to similarity, similarity to translation, translation to `base`. That is what makes `pairs_on_a_line_fall_back_rather_than_collapsing_the_image` pass, and it must be a documented fallback chain, not a panic.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p ve-core warp::`
Expected: PASS, all six.

- [ ] **Step 5: Commit**

```bash
git add crates/ve-core/src/warp.rs crates/ve-core/src/lib.rs
git commit -m "Fit an image warp from one to four control points"
```

---

### Task 3: The thin-plate-spline residual

**Files:**
- Modify: `crates/ve-core/src/warp.rs`

**Interfaces:**
- Consumes: `Warp`, `Kind::Spline`, `solve` from Task 2.
- Produces: no new public names — `Warp::fit` now returns `Kind::Spline` for five or more pairs, and `Warp::place` evaluates it.

- [ ] **Step 1: Write the failing tests**

```rust
    /// Five or more pairs bend, and every one of them still lands exactly.
    /// This is what "perfectly warped" means and is the test that would
    /// catch a wrong spline.
    #[test]
    fn every_pair_lands_exactly_however_many_there_are() {
        let mut points = Vec::new();
        for i in 0..7 {
            for j in 0..7 {
                let u = i as f64 * 100.0;
                let v = j as f64 * 80.0;
                // A target that no projective map could reach: a sine ripple
                // across the sheet, which is exactly the paper distortion a
                // rubber sheet exists to correct.
                let lon = -71.0 + u / 800.0 + 0.02 * (v / 100.0).sin();
                let lat = 42.0 - v / 600.0 + 0.02 * (u / 100.0).cos();
                points.push(at(u, v, lon, lat));
            }
        }
        assert!(points.len() <= MAX_CONTROL_POINTS);
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        assert!(!warp.is_identity_affine());
    }

    /// Exactly four pairs must stay a pure homography: the spline's residual
    /// is identically zero there, and a picture taken at an angle should not
    /// acquire bending it did not ask for.
    #[test]
    fn four_pairs_bend_not_at_all() {
        let points = [
            at(0.0, 0.0, -2.0, 1.0),
            at(800.0, 0.0, 2.0, 1.0),
            at(800.0, 600.0, 1.0, -1.0),
            at(0.0, 600.0, -1.0, -1.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        // A projective map sends straight lines to straight lines. Three
        // collinear pixels must stay collinear; a spline term would bow them.
        let a = warp.place(0.0, 0.0);
        let m = warp.place(400.0, 300.0);
        let b = warp.place(800.0, 600.0);
        let cross = (m.0 - a.0) * (b.1 - a.1) - (m.1 - a.1) * (b.0 - a.0);
        assert!(cross.abs() < 1e-9, "the diagonal bowed by {cross}");
    }

    /// Outside the hull of the points the spline must not run away: the
    /// radial term grows like r^2 log r, so a far corner has to fall back
    /// towards the projective base rather than diverge.
    #[test]
    fn a_point_far_outside_the_hull_stays_near_the_projective_base() {
        let points: Vec<_> = (0..6)
            .map(|i| {
                let u = 300.0 + (i % 3) as f64 * 100.0;
                let v = 200.0 + (i / 3) as f64 * 100.0;
                at(u, v, -70.5 + u / 4000.0, 41.5 - v / 4000.0)
            })
            .collect();
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        // Ten image-widths away, far outside the hull.
        let (lon, lat) = warp.place(8000.0, 6000.0);
        assert!(lon.is_finite() && lat.is_finite(), "{lon}, {lat}");
        assert!(lon.abs() <= 360.0 && lat.abs() <= 180.0, "ran away to {lon}, {lat}");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p ve-core warp::tests::every_pair_lands_exactly`
Expected: FAIL — with five or more pairs the homography is least-squares, so the pairs do not land exactly.

- [ ] **Step 3: Implement the spline**

In `Warp::fit`, for `n >= 5`:

1. Fit the homography from all `n` pairs by least squares (Task 2's path).
2. Compute the residual at each pair: `r_i = target_i - projective(u_i, v_i)`, in degrees.
3. Build the TPS system over the `n` pairs in **image pixel** space:
   - `K[i][j] = phi(dist(p_i, p_j))` with `phi(r) = r*r*ln(r)` for `r > 0` and `0` at `r == 0`.
   - The full `(n+3) x (n+3)` system is `[[K, P], [P^T, 0]]` with `P` the rows `[1, u_i, v_i]`.
   - Solve twice, once for the longitude residual and once for the latitude, through `solve`.
4. Store `weights` (the `n` radial weights plus the three affine terms, per axis) and `hull_radius` — the greatest distance from the pairs' centroid to any pair.
5. `Warp::place` evaluates `projective(u, v)` then adds the spline, **damped outside the hull**: with `d` the distance from the centroid, multiply the radial sum by `1.0` for `d <= hull_radius` and by `(hull_radius / d).powi(2)` beyond it, so the correction decays instead of diverging. Document that this is the mitigation the spec requires and that it is why `a_point_far_outside_the_hull_stays_near_the_projective_base` passes.

**Scale the pixel coordinates** into roughly 0..1 before building `K` (divide by the image's larger side). `r*r*ln(r)` on raw pixel distances of thousands makes the matrix badly conditioned, and the solver will report singular on a perfectly good set of points.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p ve-core warp::`
Expected: PASS, all nine.

- [ ] **Step 5: Add the antimeridian and pole cases**

Geodesy changes need both (testing rules). Add:

```rust
    /// Pairs that straddle the antimeridian. Longitudes are not normalised
    /// inside a warp — an image spanning the seam runs past 180 so its right
    /// edge stays to the right of its left one, exactly as `Placement::place`
    /// documents — so the fit must not fold it.
    #[test]
    fn pairs_across_the_antimeridian_are_not_folded() {
        let points = [
            at(0.0, 0.0, 178.0, 10.0),
            at(800.0, 0.0, 182.0, 10.0),
            at(0.0, 600.0, 178.0, 8.0),
            at(800.0, 600.0, 182.0, 8.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
        let (lon, _) = warp.place(400.0, 300.0);
        assert!((lon - 180.0).abs() < 1e-6, "the seam folded the image: {lon}");
    }

    /// An image against the pole. The warp is in degree space, so a latitude
    /// past 90 is a real answer for a pixel off the top of the sheet and must
    /// not be clamped inside the fit — but a pair at the pole must still land.
    #[test]
    fn a_pair_at_the_pole_lands_on_the_pole() {
        let points = [
            at(400.0, 0.0, 0.0, 90.0),
            at(0.0, 600.0, -20.0, 80.0),
            at(800.0, 600.0, 20.0, 80.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        assert_exact(&points, &warp);
    }
```

Run: `cargo test -p ve-core warp::`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ve-core/src/warp.rs
git commit -m "Bend an image to every control point with a thin-plate spline"
```

---

### Task 4: Evaluating the warp over a mesh

**Files:**
- Modify: `crates/ve-core/src/warp.rs`
- Create: `crates/ve-core/tests/warp_cost.rs`

**Interfaces:**
- Consumes: `Warp` from Tasks 2 and 3.
- Produces: `Warp::mesh(&self, width: u32, height: u32, cells: u32) -> Vec<[f32; 2]>` — `(cells + 1)^2` lon/lat pairs in row-major order, the row `v = 0` first, each row running `u = 0` to `u = width`. `f32` because that is what reaches a vertex buffer.

- [ ] **Step 1: Write the failing test**

```rust
    /// The mesh is the warp sampled on a grid, in the order the vertex
    /// buffer wants: row by row from the image's top-left. Checked against
    /// `place` at the same pixels, which is the definition.
    #[test]
    fn the_mesh_is_the_warp_sampled_row_by_row() {
        let points = [
            at(0.0, 0.0, -2.0, 1.0),
            at(800.0, 0.0, 2.0, 1.0),
            at(800.0, 600.0, 1.0, -1.0),
            at(0.0, 600.0, -1.0, -1.0),
        ];
        let warp = Warp::fit(&points, 800, 600, base());
        let cells = 4u32;
        let mesh = warp.mesh(800, 600, cells);
        assert_eq!(mesh.len() as u32, (cells + 1) * (cells + 1));
        for row in 0..=cells {
            for col in 0..=cells {
                let u = f64::from(col) / f64::from(cells) * 800.0;
                let v = f64::from(row) / f64::from(cells) * 600.0;
                let (lon, lat) = warp.place(u, v);
                let got = mesh[(row * (cells + 1) + col) as usize];
                assert!((f64::from(got[0]) - lon).abs() < 1e-4, "{got:?} vs {lon}");
                assert!((f64::from(got[1]) - lat).abs() < 1e-4, "{got:?} vs {lat}");
            }
        }
        // The first vertex is the image's top-left corner.
        let corner = warp.place(0.0, 0.0);
        assert!((f64::from(mesh[0][0]) - corner.0).abs() < 1e-4);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p ve-core the_mesh_is_the_warp_sampled`
Expected: FAIL — no method `mesh`.

- [ ] **Step 3: Implement `mesh`**

A double loop over `0..=cells`, calling `place`, pushing `[lon as f32, lat as f32]`. No `HashMap`, no parallelism — it is a few thousand evaluations.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p ve-core the_mesh_is_the_warp_sampled`
Expected: PASS.

- [ ] **Step 5: Add the cost harness**

Create `crates/ve-core/tests/warp_cost.rs`, following `tile_cost`/`export_cost`: ignored, release-only, printing numbers. It builds the 50-pair worst case and times `Warp::fit` and `Warp::mesh(_, _, 64)`.

```bash
cargo test -p ve-core --release --test warp_cost -- --ignored --nocapture
```

Record the two numbers in the commit message. The spec's claim is ~205k radial evaluations per rebuild; the point of the harness is that the claim is **measured, not assumed** (performance rules).

- [ ] **Step 6: Commit**

```bash
git add crates/ve-core/src/warp.rs crates/ve-core/tests/warp_cost.rs
git commit -m "Evaluate an image warp over a mesh

fit 50 pairs: <N> ms; mesh 64x64: <N> ms"
```

---

### Task 5: The command, the view, and removing the corners

**Files:**
- Modify: `crates/ve-app/src/image.rs` (`ImageLayerView` ~line 60; `set_image_corners`/`corners_set` at 710–747, **deleted**; new command beside `set_image_opacity`)
- Modify: `crates/ve-core/src/document.rs` (delete `Placement::from_corners`, ~line 796)
- Modify: `crates/ve-app/src/lib.rs` (the `invoke_handler` list)
- Modify: `crates/ve-app/src/mcp/invoke.rs` (the command table, ~line 131)
- Test: `crates/ve-app/tests/image_layers.rs`

**Interfaces:**
- Consumes: `Warp`, `MAX_CONTROL_POINTS` (Tasks 2–4); `ControlPoint` (Task 1).
- Produces:
  - Command `set_image_control_points(layer: u64, points: Vec<[f64; 4]>)` — each `[u, v, lon, lat]` — returning `ProjectSummary`.
  - `ImageLayerView` gains `control_points: Vec<[f64; 4]>` and `warped: bool`, and **loses** `corners`.

- [ ] **Step 1: Write the failing test**

In `crates/ve-app/tests/image_layers.rs`:

```rust
/// Control points reach the document through the command, come back in the
/// view, and undo restores exactly what was there before.
#[test]
fn control_points_are_set_seen_and_undone() {
    let root = TempRoot::new("control-points");
    let state = app(&root);
    let layer = an_image_layer(&state, &root);

    let before = ve_app::document::tree(&state, 0).expect("tree");
    let was = before.layers.last().expect("layer").image.as_ref().expect("image");
    assert!(was.control_points.is_empty());
    assert!(!was.warped);

    ve_app::image::control_points_set(
        &state,
        layer,
        vec![
            [0.0, 0.0, -2.0, 1.0],
            [800.0, 0.0, 2.0, 1.0],
            [800.0, 600.0, 1.0, -1.0],
            [0.0, 600.0, -1.0, -1.0],
        ],
    )
    .expect("set");

    let after = ve_app::document::tree(&state, 0).expect("tree");
    let now = after.layers.last().expect("layer").image.as_ref().expect("image");
    assert_eq!(now.control_points.len(), 4);
    assert!(now.warped, "four pairs are a homography, which is a warp");

    ve_app::history::undo(&state).expect("undo");
    let back = ve_app::document::tree(&state, 0).expect("tree");
    let again = back.layers.last().expect("layer").image.as_ref().expect("image");
    assert!(again.control_points.is_empty(), "undo did not restore the points");
}

/// Fifty is the cap, and a fifty-first is refused rather than silently
/// dropped — the fit solves an n x n system and the mesh is re-evaluated
/// against every pair.
#[test]
fn more_than_fifty_control_points_are_refused() {
    let root = TempRoot::new("too-many");
    let state = app(&root);
    let layer = an_image_layer(&state, &root);
    let too_many: Vec<[f64; 4]> = (0..51)
        .map(|i| [i as f64, i as f64, i as f64 * 0.1, i as f64 * 0.1])
        .collect();
    assert!(ve_app::image::control_points_set(&state, layer, too_many).is_err());
}
```

Write `an_image_layer` as a helper beside the existing fixtures in that file if one is not already there — it writes a small PNG (the file already builds fixtures byte by byte) and imports it, returning the layer id.

- [ ] **Step 2: Run to verify it fails**

Run: `VE_FORCE_CPU=1 cargo test -p ve-app --test image_layers control_points`
Expected: FAIL — no `control_points_set`.

- [ ] **Step 3: Add the command**

In `crates/ve-app/src/image.rs`, beside `set_image_opacity`, following the same two-function shape (`#[tauri::command]` wrapper plus a `pub fn` implementation so tests and MCP call the same code):

```rust
/// Sets the pairs that warp an image (spec.md §4.9).
///
/// Each is `[u, v, lon, lat]`: a point in the picture, in the image's own
/// pixels, and where on the earth it belongs. The warp is computed from
/// these and never stored.
#[tauri::command]
pub fn set_image_control_points(
    state: tauri::State<'_, AppState>,
    layer: u64,
    points: Vec<[f64; 4]>,
) -> Result<crate::projects::ProjectSummary> {
    control_points_set(&state, layer, points)
}
```

with `control_points_set` going through the existing `write(state, layer, None, ...)` — `None` for the gesture, because applying is one action and not a drag, which is what makes it one undo entry. Inside the closure:

- Refuse `points.len() > ve_core::warp::MAX_CONTROL_POINTS` with `AppError::BadOption { field: "image", value: "... at most 50 ..." }`.
- Refuse any non-finite number the same way.
- Replace `control_points` wholesale with the converted `Vec<ControlPoint>`.

Undo needs nothing: `write` already pushes `Command::SetLayerSource { before, after }`, which carries the whole `LayerSource`.

- [ ] **Step 4: Change the view**

In `ImageLayerView`: delete `corners`, add

```rust
    /// The pairs that warp it: `[u, v, lon, lat]` each.
    pub control_points: Vec<[f64; 4]>,
    /// Whether those pairs bend the picture, so the map must draw it
    /// through the mesh rather than the globe's per-pixel inverse
    /// (spec.md §4.9). False for an image with no pairs, or with pairs
    /// that happen to fit an affine.
    pub warped: bool,
```

`warped` is `!Warp::fit(...).is_identity_affine()`. `placement` stays and keeps meaning the base.

- [ ] **Step 5: Delete the corner path**

Remove `set_image_corners`, `corners_set`, `Placement::from_corners` and its test, and the `set_image_corners` entry in `lib.rs`'s `invoke_handler`. Add `set_image_control_points` to `invoke_handler` and to `mcp/invoke.rs`'s table:

```rust
command!(set_image_control_points, crate::image::set_image_control_points, { layer: u64, points: Vec<[f64; 4]> }),
```

and remove the `set_image_corners` row. The coverage test in `tests/mcp.rs` fails if a command is in neither the table nor `EXCLUDED`, so this is compulsory rather than tidy.

- [ ] **Step 6: Run the tests and regenerate bindings**

```bash
VE_FORCE_CPU=1 cargo test -p ve-app --test image_layers
VE_FORCE_CPU=1 cargo test -p ve-app --test mcp
npm run bindings
```
Expected: PASS, and `ui/src/generated/ImageLayerView.ts` gains the two fields and loses `corners`. The frontend will not typecheck until Task 6 — that is expected and is why these two tasks commit together at Task 6's end if you prefer a green tree at every commit.

- [ ] **Step 7: Commit**

```bash
git add crates/ve-app crates/ve-core ui/src/generated
git commit -m "Set an image's control points, and drop the corner handles"
```

---

### Task 6: Drawing a warped image in every projection

**Files:**
- Modify: `ui/src/map/shaders.ts` (`IMAGE_VERT` ~line 207, `IMAGE_FRAG` ~line 231)
- Modify: `ui/src/map/renderer.ts` (`ImageDraw` ~line 103, `buildImageMesh` ~line 529, `drawImages` ~line 872, `IMAGE_CELLS` line 85)
- Modify: `ui/src/map/images.ts` (`placeVectors`)
- Test: `ui/src/map/shaders.test.ts`

**Interfaces:**
- Consumes: `ImageLayerView.control_points`, `ImageLayerView.warped` (Task 5).
- Produces: `ImageDraw` gains `warpMesh: Float32Array | null` and `warpCells: number`; `IMAGE_WARP_CELLS = 64` exported from `renderer.ts`.

- [ ] **Step 1: Write the failing test**

In `ui/src/map/shaders.test.ts`:

```ts
/**
 * The warped path hands the vertex shader lon/lat directly, so projection
 * happens strictly after the warp — which is the whole reason this works in
 * every projection. The shader must therefore read `aGeo` when `uWarped` is
 * set and never recompute the place from the affine uniforms.
 */
it("takes a warped image's position from the mesh, not from the affine", () => {
  expect(IMAGE_VERT).toContain("aGeo");
  expect(IMAGE_VERT).toMatch(/uWarped\s*\?\s*aGeo/);
});

/**
 * The globe's per-pixel inverse cannot invert a spline, so a warped image
 * must not take that branch. `uProjection >= 15` has to be guarded by
 * `!uWarped` or a bent chart is sampled through a 2x2 affine inverse and
 * comes out scrambled.
 */
it("keeps a warped image off the globe's per-pixel inverse", () => {
  const branch = IMAGE_FRAG.match(/if\s*\([^)]*uProjection\s*>=\s*15[^)]*\)/);
  expect(branch, "the globe branch moved").not.toBeNull();
  expect(branch![0]).toContain("!uWarped");
});

/** An integer uniform in a shared prelude must state its precision. */
it("declares uWarped in both stages without relying on a default", () => {
  for (const source of [IMAGE_VERT, IMAGE_FRAG]) {
    expect(source).toMatch(/uniform bool uWarped;/);
  }
});
```

- [ ] **Step 2: Run to verify they fail**

Run: `npx vitest run src/map/shaders.test.ts --root ui`
Expected: FAIL — `aGeo` and `uWarped` do not exist.

- [ ] **Step 3: Change the shaders**

`IMAGE_VERT`: add `layout(location=2) in vec2 aGeo;` and `uniform bool uWarped;`, then

```glsl
  vec2 lonLat = uWarped
    ? aGeo
    : vec2(
        uPlaceLon.x * aCell.x + uPlaceLon.y * aCell.y + uPlaceLon.z,
        uPlaceLat.x * aCell.x + uPlaceLat.y * aCell.y + uPlaceLat.z
      );
  gl_Position = (uProjection >= 15 && !uWarped)
    ? vec4(aCell * 2.0 - 1.0, 0.0, 1.0)
    : screenToClip(uProjection == 14 ? meshToScreen(aScreen) : geoToScreen(lonLat));
```

`IMAGE_FRAG`: add `uniform bool uWarped;` and guard the globe branch with `if (uProjection >= 15 && !uWarped) { ... }`. Nothing else in the fragment stage changes — a warped image arrives with `vUV` already correct from the mesh.

**Do not write a backtick anywhere in these shader comments** — the sources are template literals and a backtick terminates the string.

- [ ] **Step 4: Build the warped mesh in the renderer**

- `export const IMAGE_WARP_CELLS = 64;` beside `IMAGE_CELLS = 16`.
- A `warpMeshes: Map<number, { vao, buffer, count, cells }>` on the renderer, keyed by layer.
- When an `ImageDraw` carries a `warpMesh`, upload it to that layer's buffer as attribute location 2 alongside the shared `aCell`/`aScreen`, and draw with the warped VAO; otherwise use `this.imageMesh` as today.
- Upload only when the `Float32Array` identity changes — the mesh is rebuilt when the points change, not per frame, and re-uploading per frame is exactly the cost this design exists to avoid.
- Delete a layer's buffer in `dispose()` and when the layer goes.

- [ ] **Step 5: Run to verify they pass**

Run: `npx vitest run src/map/shaders.test.ts --root ui && npm run ui:typecheck`
Expected: PASS.

- [ ] **Step 6: Assert the locality assumption**

The spec justifies the mesh/per-pixel split by "a rubber-sheeted image is local". Make it a test rather than a comment, in `ui/src/map/images.test.ts` (create it):

```ts
/**
 * The split in spec 4.9 is justified by warped images being local, so the
 * mesh is affordable. A warp whose corners span more than a quarter of the
 * earth is outside what that reasoning covers, and the renderer says so
 * rather than quietly drawing a coarse mesh across a hemisphere.
 */
it("says when a warped image is too large for the mesh path", () => {
  expect(warpSpansTooMuch(hugeMeshSpanningHalfTheEarth)).toBe(true);
  expect(warpSpansTooMuch(harbourSizedMesh)).toBe(false);
});
```

`warpSpansTooMuch` lives in `images.ts`, and a true answer reports through `reportError` in `hint.ts` — it does not refuse to draw.

- [ ] **Step 6a: Assert the projection claim itself**

Spec §7 names this test, and it is the one that states the design's central
claim as something that can fail. The warp is fitted once and the mesh is
lon/lat, so the vertex buffer must be **identical** whatever projection the
camera is in — projection happens strictly after. In `ui/src/map/images.test.ts`:

```ts
/**
 * The reason this works in every projection: the warp is evaluated in
 * lon/lat and the shader only projects what it is handed. So the same pairs
 * must produce a byte-identical mesh under every camera. If this ever fails,
 * something has started baking the view into the warp.
 */
it("builds the same mesh whatever the map is being drawn in", () => {
  const view = warpedImageView();
  const first = warpMeshFor(view);
  for (const projection of ["equirectangular", "mercator", "orthographic", "robinson"]) {
    const again = warpMeshFor({ ...view, projection });
    expect(Array.from(again)).toEqual(Array.from(first));
  }
});
```

Run: `npx vitest run src/map/images.test.ts --root ui`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add ui/src/map
git commit -m "Draw a warped image through its mesh in every projection"
```

---

### Task 7: The alignment mode

**Files:**
- Modify: `ui/src/panels/LayerPanel.tsx` (`ImageControls`, ~line 895)
- Create: `ui/src/map/align.ts` — the mode's state and geometry, outside React
- Create: `ui/src/map/align.test.ts`
- Modify: `ui/src/map/MapView.tsx` (pointer handling, overlay, key handling)
- Modify: `ui/src/map/cursor.ts` and `ui/src/map/cursor.test.ts`
- Modify: `ui/src/ipc.ts` (`setImageControlPoints`)

**Interfaces:**
- Consumes: `api.setImageControlPoints(layer, points)` (Task 5); `ImageLayerView.control_points`.
- Produces: `createAlignStore()` in `align.ts` with `{ subscribe, get, begin(layer), pick(kind, lonLat), undoLast(), cancel(), pairs() }`, and `imagePixelAt(view, lonLat)` converting a map position to an image pixel through the view's current warp.

- [ ] **Step 1: Write the failing test**

In `ui/src/map/align.test.ts`:

```ts
/**
 * The mode alternates, starting on the picture, and a half-finished pair is
 * not a pair. This is the whole state machine and it is worth testing away
 * from the map, where it can be driven directly.
 */
it("alternates picture then map, and discards a half-placed pair", () => {
  const store = createAlignStore();
  store.begin(7);
  expect(store.get().expecting).toBe("picture");
  store.pick("picture", { lon: -70, lat: 41 });
  expect(store.get().expecting).toBe("map");
  expect(store.pairs()).toHaveLength(0);
  store.pick("map", { lon: -69, lat: 42 });
  expect(store.get().expecting).toBe("picture");
  expect(store.pairs()).toHaveLength(1);
  store.pick("picture", { lon: -70, lat: 40 });
  expect(store.pairs()).toHaveLength(1);
});

/** Backspace drops the last whole pair, and then the one before it. */
it("undoes the last pair", () => {
  const store = createAlignStore();
  store.begin(7);
  store.pick("picture", { lon: -70, lat: 41 });
  store.pick("map", { lon: -69, lat: 42 });
  store.pick("picture", { lon: -71, lat: 41 });
  store.pick("map", { lon: -68, lat: 42 });
  expect(store.pairs()).toHaveLength(2);
  store.undoLast();
  expect(store.pairs()).toHaveLength(1);
  expect(store.get().expecting).toBe("picture");
});

/**
 * A picture click is carried back to an image pixel through the warp in
 * force, so pairs accumulate against a stable image space however many have
 * already been placed. Checked against the inverse of a known placement.
 */
it("turns a map position into the image pixel under it", () => {
  const view = { width: 800, height: 600, placement: [1 / 800, 0, -71, 0, -1 / 600, 42] };
  expect(imagePixelAt(view, { lon: -70.5, lat: 41.5 })).toEqual([400, 300]);
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `npx vitest run src/map/align.test.ts --root ui`
Expected: FAIL — `align.ts` does not exist.

- [ ] **Step 3: Write `align.ts`**

An external store, **not** React state: the pending point changes per pointer report and putting it in `MapView` state would re-render two thousand lines of hooks per move — the mistake `createReadoutStore` exists to prevent. Shape it the same way, with `subscribe`/`get`/`set`.

- [ ] **Step 4: Run to verify it passes**

Run: `npx vitest run src/map/align.test.ts --root ui`
Expected: PASS.

- [ ] **Step 4a: Report the residual, which the spec promises**

Spec §2 says the panel reports the residual at the image's four corners, so a
warp misbehaving outside the hull is visible rather than discovered later, and
§5 says the per-pair residual is shown once there are enough pairs to fit.
Neither is optional.

`ImageLayerView` gains `corner_residual_deg: Vec<f64>` — four numbers, the
distance in degrees between each corner under the warp and under the plain
projective base, which is exactly "how much is the spline doing out here".
Computed in `image_view` beside `warped`, from the same `Warp`. `ImageControls`
shows the largest of the four when it exceeds a tenth of a degree, as a muted
line, worded as a caution and not an error — it is information, not a refusal.

- [ ] **Step 5: Add the button**

In `ImageControls`, beside the Opacity slider:

```tsx
<button
  onClick={() => onAlign()}
  title="Align by pointing: click a place in the picture, then the same place on the map. Repeat as often as you like, then press Enter. Escape cancels; Backspace drops the last pair."
>
  Align…
</button>
```

- [ ] **Step 6: Wire the map**

In `MapView`: a click while the mode is on goes to `store.pick`, never to a tool. `Enter` calls `api.setImageControlPoints` with the collected pairs and clears the store; `Escape` cancels; `Backspace` undoes the last. The prompt goes through `setHint` in `hint.ts` — no panel renders help text of its own. The cursor gets a row in `cursorFor` and a case in `cursor.test.ts`, or a new mode falls to the crosshair unnoticed.

The overlay draws each pair numbered with a line from picture point to target, inside `draw()` and requested through `requestOverlay`, never by a synchronous `drawOverlay`.

- [ ] **Step 7: Run everything**

```bash
npm run ui:typecheck && npm run ui:test
```
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add ui/src
git commit -m "Align an image by pointing at pairs"
```

---

### Task 8: Remove what the corners left behind

**Files:**
- Modify: `ui/src/map/MapView.tsx` (`imageDrag` ~line 838, `placing.corners` ~line 3374, the handle drawing)
- Modify: `ui/src/ipc.ts` (delete `setImageCorners`)
- Modify: `ui/src/map/images.ts` (`cornersOf`)

- [ ] **Step 0: Reset place clears the control points**

Spec §6: *Reset place* stays and now clears the control points as well as
restoring the placement, "since the two together are where the image sits".
Without this, resetting an image that has been warped puts the base back and
leaves the warp on top of it, which looks like the reset did nothing.

In `placement_reset` in `crates/ve-app/src/image.rs`, clear `control_points`
in the same closure that restores the placement. Test it in
`crates/ve-app/tests/image_layers.rs`:

```rust
/// Resetting an image is "put it back where its file says", and a warp is
/// part of where it sits. Leaving the pairs behind would restore the base
/// and then immediately bend away from it again.
#[test]
fn resetting_the_placement_clears_the_control_points() {
    let root = TempRoot::new("reset-clears");
    let state = app(&root);
    let layer = an_image_layer(&state, &root);
    ve_app::image::control_points_set(
        &state,
        layer,
        vec![[0.0, 0.0, -2.0, 1.0], [800.0, 0.0, 2.0, 1.0], [0.0, 600.0, -2.0, -1.0]],
    )
    .expect("set");
    ve_app::image::placement_reset(&state, layer).expect("reset");
    let tree = ve_app::document::tree(&state, 0).expect("tree");
    let image = tree.layers.last().expect("layer").image.as_ref().expect("image");
    assert!(image.control_points.is_empty(), "the warp survived a reset");
    assert!(!image.warped);
}
```

- [ ] **Step 1: Delete, then let the compiler find the rest**

Remove `setImageCorners` from `ipc.ts` and `cornersOf` from wherever it lives, then run `npm run ui:typecheck` and fix every site it names. `BaselineOutline::of`-style exhaustive matches exist precisely so the compiler asks rather than the feature silently vanishing.

- [ ] **Step 2: Run the suites**

```bash
npm run ui:typecheck && npm run ui:test
VE_FORCE_CPU=1 cargo test --workspace
```
Expected: PASS. Any test that dragged corners is rewritten to place control points, not deleted.

- [ ] **Step 3: Commit**

```bash
git add ui/src crates
git commit -m "Remove the image corner handles the control points replace"
```

---

### Task 9: Documentation, and seeing it work

**Files:**
- Modify: `spec.md` §4.9
- Modify: `CLAUDE.md` (the "Adding an image layer format" recipe, and the repo layout line for `ve-core`)

- [ ] **Step 1: Update `spec.md` §4.9**

Replace the three-corner description with the control-point one: what is stored, the fit by pair count, the mesh/per-pixel split and why, and the fifty cap. Behaviour changed, so this is required in the same commit as the behaviour (definition of done).

- [ ] **Step 2: Update `CLAUDE.md`**

`ve-core`'s line gains `warp` (the image warp of spec §4.9). Add a recipe note that an image with control points draws through the mesh in every projection and must never take the globe's per-pixel inverse.

- [ ] **Step 3: See it in the application**

Not optional — this is a visual feature and the suites cannot see it.

```bash
lsof -nP -iTCP:5173 -sTCP:LISTEN     # check whose dev server that is first
VE_DEV_PORT=5199 node tools/webdriver/cli.mjs serve
```

Import a scanned chart, place five or six pairs, press Enter, and confirm on the flat map, on the globe and on one general projection that the picture lands where the pairs were put. Remember the canvas sits below the title bar — read its `getBoundingClientRect().top` and add it to every `clientY`, or every click lands somewhere else entirely. Kill the driver **as a process group and by pid**; never `pkill -f ve-app`.

- [ ] **Step 4: Run the full definition of done**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
VE_FORCE_CPU=1 cargo test --workspace
npm run ui:typecheck && npm run ui:test && npm run check:offline
```

- [ ] **Step 5: Commit**

```bash
git add spec.md CLAUDE.md
git commit -m "Document the image control-point warp"
```
