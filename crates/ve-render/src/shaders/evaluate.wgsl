// Field evaluation on the GPU.
//
// A port of `cpu.rs`, which is the authority: this exists only to make the
// preview fast. The two need to agree perceptually, not numerically — speed
// within max(0.25 m/s, 2%), direction within 2 degrees (spec.md 7.9) — so this
// works in f32 and uses whatever approximations the hardware offers.
//
// Not handled here, deliberately:
//   - the clone stamp, which reads its layer beneath itself and so needs
//     recursion; scenes containing one fall back to the CPU.
//   - the warp modifier, for the same reason: it reads its layer at a
//     displaced position (spec.md 6.3). The other three modifiers transform
//     the vector where it already is and are handled below.
// Everything else follows the same order of operations as the CPU path:
// each layer composited on its own, the layers stacked by coverage, and
// the coverage and the kind written out beside the field (M31).

const EARTH_RADIUS_M: f32 = 6371229.0;
const PI: f32 = 3.14159265358979;
const DEG: f32 = 0.01745329251994;
// Metres per degree of latitude, for the projected frame. The same earth as
// EARTH_RADIUS_M, written out because WGSL has no const arithmetic on PI here.
const M_PER_DEGREE: f32 = 111198.9234485458;

struct Object {
    anchor: vec2<f32>,        // lon, lat in degrees
    rotation_deg: f32,
    scale: f32,

    cap_radius_m: f32,
    shape_kind: u32,          // 0 capsule, 1 disc, 2 annulus, 3 rect, 4 polygon,
                              // 5 swept square
    shape_a: f32,             // capsule/disc radius, annulus radius, rect half
                              // width, swept-square half size
    shape_b: f32,             // annulus half width, rect half height

    shape_offset: u32,        // capsule: segment pairs; polygon: ring
    shape_count: u32,
    speed_kind: u32,          // 0 constant, 1 radial, 2 axis
    speed_a: f32,

    speed_b: f32,
    speed_extent: f32,
    dir_kind: u32,            // 0 constant, 1 toward, 2 axis, 3 along path,
                              // 4 tangential, 5 away from a point
    dir_a: f32,

    dir_b: f32,
    feather: f32,
    // What a modifier does to the field beneath it: 0 none, 1 gain, 2 radial,
    // 3 turn. In the two words where `divergence` and `curl` used to sit
    // (spec.md 7.5). A warp is the fourth kind and never arrives here — it
    // reads the composite at another position, which needs recursion, so
    // `supports` in gpu.rs sends a scene containing one to the CPU.
    mod_kind: u32,
    mod_a: f32,

    edge_mode: u32,           // 0 blend, 1 replace
    gradient_axis: f32,
    gradient_extent: f32,
    // Sent rather than re-derived here: computing it independently on each
    // side is exactly how two backends quietly disagree.
    feather_reference: f32,

    path_offset: u32,         // the ordered path, for along-path direction
    path_count: u32,
    space: u32,               // 0 geodesic (ground metres), 1 projected (map metres)
    invert: u32,              // 1 = covers everything but its footprint

    // The object's own movement, added to what it paints (spec.md 9.3, M13).
    // An angular velocity in radians per second, in earth-centred cartesian
    // coordinates: a translation and a turn are each one, and they add. The
    // fourth word is the relative rate of change of scale, per second.
    motion_omega: vec3<f32>,
    motion_scale_rate: f32,

    // The layer the object is in, counting from the bottom; the kind of
    // field its layer holds, 1 for wind; and whether it removes rather than
    // writes — the mask (M31). The kernel composites each layer on its own
    // and stacks the layers by coverage, as `composite` in cpu.rs does.
    layer: u32,
    kind: u32,
    erases: u32,
    pad0: u32,
};

// An imported field's time slice: a regular lat/lon lattice in canonical
// orientation, columns eastward from lon0 and rows southward from lat0.
// Mirrors `ve_core::raster::RasterGrid`; RASTER_WORDS in gpu.rs counts these.
struct Raster {
    ni: u32,
    nj: u32,
    offset: u32,              // first sample in raster_data
    z: u32,                   // objects beneath it

    lon0: f32,
    lat0: f32,
    dlon: f32,
    dlat: f32,

    wraps: u32,               // 1 when the column after the last is column 0
    speed_min: f32,           // the layer's speed band; 0..inf when it has none
    speed_max: f32,
    layer_kind: u32,          // the layer in the low 16 bits, the kind above (M31)
};

@group(0) @binding(0) var<storage, read> objects: array<Object>;
@group(0) @binding(1) var<storage, read> points: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read> samples: array<vec2<f32>>;   // lon, lat
@group(0) @binding(3) var<storage, read_write> output: array<vec4<f32>>; // u, v, coverage, kind
@group(0) @binding(4) var<storage, read> rasters: array<Raster>;
@group(0) @binding(5) var<storage, read> raster_data: array<vec2<f32>>; // u, v

// A raster sample at or below this marks a missing node: the sentinel
// `ve_core::raster::MISSING`, compared rather than NaN-tested because NaN
// checks are what a shader compiler is allowed to optimise away.
const RASTER_MISSING: f32 = -1.0e29;
// Slack, in cells, for a sample that lands just past the last node. Wider
// than the CPU's 1e-6 because index arithmetic here is in f32; a difference
// only that close to the edge of a regional grid is beneath the preview
// tolerance.
const RASTER_EDGE_SLACK: f32 = 1e-3;

fn rem_euclid_360(x: f32) -> f32 {
    return x - floor(x / 360.0) * 360.0;
}

// One node, or a missing marker.
fn raster_node(raster: Raster, i: u32, j: u32) -> vec2<f32> {
    return raster_data[raster.offset + j * raster.ni + i];
}

fn raster_node_present(node: vec2<f32>) -> bool {
    return node.x > RASTER_MISSING && node.y > RASTER_MISSING;
}

// A port of `RasterGrid::sample`, decision for decision. Returns (u, v, 1)
// where the grid has a value and (0, 0, 0) where it has none.
fn sample_raster(raster: Raster, position: vec2<f32>) -> vec3<f32> {
    let none = vec3<f32>(0.0, 0.0, 0.0);
    if (raster.ni == 0u || raster.nj == 0u) { return none; }
    let last_i = f32(raster.ni - 1u);
    let last_j = f32(raster.nj - 1u);

    let fj_raw = (raster.lat0 - position.y) / raster.dlat;
    if (fj_raw < -RASTER_EDGE_SLACK || fj_raw > last_j + RASTER_EDGE_SLACK) { return none; }
    let fi_raw = rem_euclid_360(position.x - raster.lon0) / raster.dlon;
    let wraps = raster.wraps != 0u;
    if (!wraps && fi_raw > last_i + RASTER_EDGE_SLACK) { return none; }

    var fi_max = last_i;
    if (wraps) { fi_max = f32(raster.ni); }
    let fi = clamp(fi_raw, 0.0, fi_max);
    let fj = clamp(fj_raw, 0.0, last_j);

    let i0 = min(u32(floor(fi)), raster.ni - 1u);
    let j0 = min(u32(floor(fj)), raster.nj - 1u);
    let tx = fi - f32(i0);
    let ty = fj - f32(j0);
    var i1 = i0;
    if (i0 + 1u < raster.ni) { i1 = i0 + 1u; } else if (wraps) { i1 = 0u; }
    let j1 = min(j0 + 1u, raster.nj - 1u);

    var total = 0.0;
    var sum = vec2<f32>(0.0, 0.0);
    let c00 = raster_node(raster, i0, j0);
    if (raster_node_present(c00)) { let w = (1.0 - tx) * (1.0 - ty); total += w; sum += c00 * w; }
    let c10 = raster_node(raster, i1, j0);
    if (raster_node_present(c10)) { let w = tx * (1.0 - ty); total += w; sum += c10 * w; }
    let c01 = raster_node(raster, i0, j1);
    if (raster_node_present(c01)) { let w = (1.0 - tx) * ty; total += w; sum += c01 * w; }
    let c11 = raster_node(raster, i1, j1);
    if (raster_node_present(c11)) { let w = tx * ty; total += w; sum += c11 * w; }
    if (total <= 1e-6) { return none; }
    return vec3<f32>(sum / total, 1.0);
}

// Whether a sample is inside the layer's speed band (spec.md 4.8). A sample
// outside it is treated as a missing one, so the field beneath shows through:
// the port of the same test in `sample_upto`.
fn kept(raster: Raster, uv: vec2<f32>) -> bool {
    let speed = length(uv);
    return speed >= raster.speed_min && speed <= raster.speed_max;
}

fn normalize_lon(lon: f32) -> f32 {
    return ((lon + 180.0) - floor((lon + 180.0) / 360.0) * 360.0) - 180.0;
}

fn distance_m(a: vec2<f32>, b: vec2<f32>) -> f32 {
    let phi1 = a.y * DEG;
    let phi2 = b.y * DEG;
    let dphi = phi2 - phi1;
    let dlambda = normalize_lon(b.x - a.x) * DEG;
    let s1 = sin(dphi * 0.5);
    let s2 = sin(dlambda * 0.5);
    let h = s1 * s1 + cos(phi1) * cos(phi2) * s2 * s2;
    return 2.0 * EARTH_RADIUS_M * asin(clamp(sqrt(h), -1.0, 1.0));
}

fn initial_bearing(a: vec2<f32>, b: vec2<f32>) -> f32 {
    let phi1 = a.y * DEG;
    let phi2 = b.y * DEG;
    let dlambda = normalize_lon(b.x - a.x) * DEG;
    let y = sin(dlambda) * cos(phi2);
    let x = cos(phi1) * sin(phi2) - sin(phi1) * cos(phi2) * cos(dlambda);
    return atan2(y, x) / DEG;
}

// The object's local frame: geometry only, never a direction (see aeqd.rs for
// why). Mirrors `Frame::to_local`; the two are the same construction twice, and
// the fidelity suite is what keeps them the same.
fn to_local(object: Object, position: vec2<f32>) -> vec2<f32> {
    if (object.space == 1u) {
        // Map space: degrees scaled to metres, no cosine, so a circle in the
        // frame is a circle on the map at every latitude.
        let east = normalize_lon(position.x - object.anchor.x) * M_PER_DEGREE / object.scale;
        let north = (position.y - object.anchor.y) * M_PER_DEGREE / object.scale;
        let theta = object.rotation_deg * DEG;
        let c = cos(theta);
        let s = sin(theta);
        return vec2<f32>(east * c - north * s, east * s + north * c);
    }
    let d = distance_m(object.anchor, position);
    if (d < 1e-6) {
        return vec2<f32>(0.0, 0.0);
    }
    let bearing = initial_bearing(object.anchor, position);
    let local = (bearing - object.rotation_deg) * DEG;
    let radius = d / object.scale;
    return vec2<f32>(radius * sin(local), radius * cos(local));
}

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let denom = dot(ba, ba);
    var t = 0.0;
    if (denom > 1e-12) {
        t = clamp(dot(pa, ba) / denom, 0.0, 1.0);
    }
    return length(pa - ba * t);
}

// Chebyshev distance from `p` to the segment `a`-`b`. See `segment_distance_inf`
// in sdf.rs, which this is a port of: the minimum of a convex piecewise-linear
// function, taken over the four crossings and the two ends rather than searched
// for, so both backends land on the same number.
fn inf_at(e: vec2<f32>, d: vec2<f32>, t: f32) -> f32 {
    let at = e + d * t;
    return max(abs(at.x), abs(at.y));
}

// One crossing candidate, folded into the running best. A denominator at zero
// puts the crossing at infinity, which the two ends already answer for.
fn inf_crossing(e: vec2<f32>, d: vec2<f32>, numerator: f32, denominator: f32, best: f32) -> f32 {
    if (abs(denominator) <= 1e-12) { return best; }
    return min(best, inf_at(e, d, clamp(numerator / denominator, 0.0, 1.0)));
}

fn segment_distance_inf(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let e = a - p;
    let d = b - a;
    var best = min(inf_at(e, d, 0.0), inf_at(e, d, 1.0));
    best = inf_crossing(e, d, -e.x, d.x, best);              // x term changes sign
    best = inf_crossing(e, d, -e.y, d.y, best);              // y term changes sign
    best = inf_crossing(e, d, e.y - e.x, d.x - d.y, best);   // terms meet, same sign
    best = inf_crossing(e, d, -(e.x + e.y), d.x + d.y, best);// terms meet, opposite
    return best;
}

fn shape_distance(object: Object, p: vec2<f32>) -> f32 {
    switch object.shape_kind {
        case 1u: {
            return length(p) - object.shape_a;
        }
        case 2u: {
            return abs(length(p) - object.shape_a) - object.shape_b;
        }
        case 3u: {
            let d = abs(p) - vec2<f32>(object.shape_a, object.shape_b);
            return length(max(d, vec2<f32>(0.0, 0.0))) + min(max(d.x, d.y), 0.0);
        }
        case 4u: {
            // Polygon: nearest edge, signed by a crossing count.
            var best = 1e30;
            var inside = false;
            let n = object.shape_count;
            for (var i = 0u; i < n; i = i + 1u) {
                let a = points[object.shape_offset + i];
                let b = points[object.shape_offset + ((i + 1u) % n)];
                best = min(best, segment_distance(p, a, b));
                if ((a.y > p.y) != (b.y > p.y)) {
                    let t = (p.y - a.y) / (b.y - a.y);
                    if (p.x < a.x + t * (b.x - a.x)) {
                        inside = !inside;
                    }
                }
            }
            if (inside) { return -best; }
            return best;
        }
        case 5u: {
            // Swept square: the same segment pairs as a capsule, measured in
            // the Chebyshev metric whose unit ball is the stamp.
            let n = object.shape_count;
            if (n == 0u) { return 1e30; }
            var best = 1e30;
            for (var i = 0u; i + 1u < n; i = i + 2u) {
                best = min(
                    best,
                    segment_distance_inf(p, points[object.shape_offset + i],
                                         points[object.shape_offset + i + 1u])
                );
            }
            return best - object.shape_a;
        }
        default: {
            // Capsule: swept disc along one or more polylines, uploaded as
            // independent segment pairs. Pairs rather than polylines because a
            // merged stroke holds several chains, and walking one contiguous
            // run would sweep the brush across the gaps between them.
            let n = object.shape_count;
            if (n == 0u) { return 1e30; }
            var best = 1e30;
            for (var i = 0u; i + 1u < n; i = i + 2u) {
                best = min(
                    best,
                    segment_distance(p, points[object.shape_offset + i],
                                     points[object.shape_offset + i + 1u])
                );
            }
            return best - object.shape_a;
        }
    }
}

fn smooth_step(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 <= edge0) {
        if (x >= edge1) { return 1.0; }
        return 0.0;
    }
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn axis_fraction(object: Object, local: vec2<f32>) -> f32 {
    if (object.gradient_extent <= 0.0) { return 0.5; }
    let axis = (object.gradient_axis - object.rotation_deg) * DEG;
    let along = local.x * sin(axis) + local.y * cos(axis);
    return clamp(0.5 + along / (2.0 * object.gradient_extent), 0.0, 1.0);
}

fn speed_at(object: Object, local: vec2<f32>) -> f32 {
    switch object.speed_kind {
        case 1u: {
            var t = 0.0;
            if (object.speed_extent > 0.0) {
                t = clamp(length(local) / object.speed_extent, 0.0, 1.0);
            }
            return object.speed_a + (object.speed_b - object.speed_a) * t;
        }
        case 2u: {
            let t = axis_fraction(object, local);
            return object.speed_a + (object.speed_b - object.speed_a) * t;
        }
        default: { return object.speed_a; }
    }
}

// Shortest-arc blend between two bearings.
fn lerp_bearing(start: f32, end: f32, t: f32) -> f32 {
    var delta = end - start;
    delta = delta - floor(delta / 360.0) * 360.0;
    if (delta > 180.0) { delta = delta - 360.0; }
    return start + delta * t;
}

fn p_of(object: Object, i: u32) -> vec2<f32> {
    return points[object.path_offset + i];
}

fn destination(origin: vec2<f32>, bearing_deg: f32, distance: f32) -> vec2<f32> {
    let delta = distance / EARTH_RADIUS_M;
    let theta = bearing_deg * DEG;
    let phi1 = origin.y * DEG;
    let lambda1 = origin.x * DEG;
    let sin_phi2 = sin(phi1) * cos(delta) + cos(phi1) * sin(delta) * cos(theta);
    let phi2 = asin(clamp(sin_phi2, -1.0, 1.0));
    let lambda2 = lambda1 + atan2(sin(theta) * sin(delta) * cos(phi1),
                                  cos(delta) - sin(phi1) * sin_phi2);
    return vec2<f32>(normalize_lon(lambda2 / DEG), phi2 / DEG);
}

// The inverse of `to_local`, and it has to answer for both spaces the way that
// one does. Only `path_bearing` calls it, which is why a map-space frame going
// through the ground formula stayed hidden until the curve's direction mode was
// compared: it put the segment endpoints tens of degrees from where the CPU had
// them, and the tangent between them is what the flow follows.
fn to_global(object: Object, local: vec2<f32>) -> vec2<f32> {
    if (object.space == 1u) {
        // Undo `to_local`'s rotation, then read the offset back as degrees.
        let theta = object.rotation_deg * DEG;
        let c = cos(theta);
        let s = sin(theta);
        let east = local.x * c + local.y * s;
        let north = -local.x * s + local.y * c;
        let lon = object.anchor.x + east * object.scale / M_PER_DEGREE;
        // Clamped rather than wrapped, as in `Frame::to_global`: a shape
        // reaching past a pole flattens against it, which is what the map shows.
        let lat = clamp(object.anchor.y + north * object.scale / M_PER_DEGREE, -90.0, 90.0);
        return vec2<f32>(normalize_lon(lon), lat);
    }
    let radius = length(local);
    if (radius < 1e-6) { return object.anchor; }
    let bearing = atan2(local.x, local.y) / DEG + object.rotation_deg;
    return destination(object.anchor, bearing, radius * object.scale);
}

fn path_bearing(object: Object, local: vec2<f32>) -> f32 {
    let n = object.path_count;
    if (n < 2u) { return object.rotation_deg; }

    var best = 1e30;
    var best_index = 0u;
    for (var i = 0u; i + 1u < n; i = i + 1u) {
        // The sample against the segment, in that order. Reversed, this asked
        // for the distance from vertex i to the segment running from the sample
        // to vertex i+1 — a different number, minimised by a different segment,
        // so the flow followed the tangent of whichever part of the curve
        // happened to win.
        let d = segment_distance(local, p_of(object, i), p_of(object, i + 1u));
        if (d < best) { best = d; best_index = i; }
    }
    // The bearing is taken between the segment's endpoints on the globe, not
    // from a local frame angle, so it is a true azimuth at any distance.
    let a = to_global(object, p_of(object, best_index));
    let b = to_global(object, p_of(object, best_index + 1u));
    return initial_bearing(a, b);
}

fn direction_at(object: Object, position: vec2<f32>, local: vec2<f32>) -> f32 {
    switch object.dir_kind {
        case 1u: {
            return initial_bearing(position, vec2<f32>(object.dir_a, object.dir_b));
        }
        case 2u: {
            return lerp_bearing(object.dir_a, object.dir_b, axis_fraction(object, local));
        }
        case 3u: {
            return path_bearing(object, local) + object.dir_a;
        }
        case 4u: {
            let radial = initial_bearing(object.anchor, position);
            return radial + object.dir_a;
        }
        // Away from the target: the reciprocal of the bearing to it, which is
        // the outward tangent to the same great circle.
        case 5u: {
            return initial_bearing(position, vec2<f32>(object.dir_a, object.dir_b)) + 180.0;
        }
        default: { return object.dir_a; }
    }
}

// The velocity an object's own movement adds at a position, in m/s eastward
// and northward. The port of `Motion::velocity_at` in scene.rs, decision for
// decision: `omega x p` is a rate on the unit sphere, the earth's radius makes
// it metres, and the scale term is radial from the anchor.
fn motion_uv(object: Object, position: vec2<f32>) -> vec2<f32> {
    var uv = vec2<f32>(0.0, 0.0);
    let w = object.motion_omega;
    if (dot(w, w) > 0.0) {
        let lat = position.y * DEG;
        let lon = position.x * DEG;
        let cos_lat = cos(lat);
        let p = vec3<f32>(cos_lat * cos(lon), cos_lat * sin(lon), sin(lat));
        let v = cross(w, p) * EARTH_RADIUS_M;
        let east = vec3<f32>(-sin(lon), cos(lon), 0.0);
        let north = vec3<f32>(-sin(lat) * cos(lon), -sin(lat) * sin(lon), cos_lat);
        uv = vec2<f32>(dot(v, east), dot(v, north));
    }
    if (object.motion_scale_rate != 0.0) {
        let radial = object.motion_scale_rate * distance_m(object.anchor, position);
        let bearing = initial_bearing(object.anchor, position) * DEG;
        uv = uv + vec2<f32>(radial * sin(bearing), radial * cos(bearing));
    }
    return uv;
}

fn uv_from(speed: f32, bearing_deg: f32) -> vec2<f32> {
    let theta = bearing_deg * DEG;
    return vec2<f32>(speed * sin(theta), speed * cos(theta));
}

// What a modifier makes of the vector beneath it (spec.md 6.3).
//
// The port of `modified_vector` in cpu.rs, decision for decision: the gain
// scales, the radial component is a fraction of the local speed along the
// frame's own outward bearing, and a turn rotates (u, v) by the same expansion
// of sin(az + d) and cos(az + d) the CPU uses. A calm cell stays calm in all
// three, which is why the turn is written out rather than routed through a
// speed and an azimuth.
// The nearest point of a swept shape's centreline to `p`, in the local
// frame: the same walk over segment pairs the capsule distance makes, keeping
// the point rather than the distance. `sdf::nearest_on_chains` is the
// authority; this is its port.
fn nearest_on_segments(object: Object, p: vec2<f32>) -> vec2<f32> {
    let n = object.shape_count;
    var best = 1e30;
    var nearest = p;
    for (var i = 0u; i + 1u < n; i = i + 2u) {
        let a = points[object.shape_offset + i];
        let b = points[object.shape_offset + i + 1u];
        let ab = b - a;
        let len2 = dot(ab, ab);
        var t = 0.0;
        if (len2 > 0.0) {
            t = clamp(dot(p - a, ab) / len2, 0.0, 1.0);
        }
        let q = a + ab * t;
        let d = distance(p, q);
        if (d < best) {
            best = d;
            nearest = q;
        }
    }
    return nearest;
}

// Metres from a stroke's centreline over which a divergence fades in
// (spec.md 6.3, M29). Mirrors `RADIAL_TAPER_M` in cpu.rs.
const RADIAL_TAPER_M: f32 = 100.0;

fn modified_vector(object: Object, position: vec2<f32>, beneath: vec2<f32>) -> vec2<f32> {
    if (object.mod_kind == 1u) {
        return beneath * (1.0 + object.mod_a);
    }
    if (object.mod_kind == 2u) {
        let speed = length(beneath);
        // Outward from a swept stroke's own centreline, tapered to nothing
        // on it; from the anchor for a stamp (M29). Decision for decision the
        // CPU's `Modifier::Radial` arm.
        var origin = object.anchor;
        var weight = 1.0;
        if (object.shape_kind == 0u || object.shape_kind == 5u) {
            let local = to_local(object, position);
            let q = nearest_on_segments(object, local);
            weight = clamp(distance(local, q) / RADIAL_TAPER_M, 0.0, 1.0);
            origin = to_global(object, q);
        }
        let radial = initial_bearing(origin, position);
        let sum = beneath + uv_from(speed * object.mod_a * weight, radial);
        // Back to the speed it had (M54): a divergence bends the flow, it does
        // not drive it. The CPU's arm, decision for decision — including the
        // sum of nothing, which keeps nothing rather than acquiring a
        // direction it does not have.
        let magnitude = length(sum);
        if (magnitude <= 1e-9) {
            return sum;
        }
        return sum * (speed / magnitude);
    }
    if (object.mod_kind == 3u) {
        let theta = object.mod_a * DEG;
        let s = sin(theta);
        let c = cos(theta);
        return vec2<f32>(beneath.x * c + beneath.y * s, beneath.y * c - beneath.x * s);
    }
    return beneath;
}

// One layer's accumulation while the kernel walks it, and the stack of the
// layers finished so far (spec.md 7.6, M31). Mirrors `stack` in cpu.rs.
struct Composite {
    out_uv: vec2<f32>,
    out_coverage: f32,
    out_kind: f32,
    strong: bool,
    acc: vec2<f32>,
    coverage: f32,
    layer: u32,
    kind: u32,
    open: bool,
};

// Stacks the layer being accumulated over what is beneath it. The
// accumulation is premultiplied by its coverage, so the layer goes over the
// stack by the usual rule; the kind is the topmost layer's that covers at
// least half the cell, or, while none does, the topmost that touches it.
fn close_layer(c: ptr<function, Composite>) {
    if ((*c).open && (*c).coverage > 0.0) {
        let cov = (*c).coverage;
        (*c).out_uv = (*c).out_uv * (1.0 - cov) + (*c).acc;
        (*c).out_coverage = (*c).out_coverage * (1.0 - cov) + cov;
        if (cov >= 0.5) {
            (*c).out_kind = f32((*c).kind);
            (*c).strong = true;
        } else if (!(*c).strong) {
            (*c).out_kind = f32((*c).kind);
        }
    }
    (*c).open = false;
}

// Begins accumulating `layer` if it is not the one open, closing the open one.
fn enter_layer(c: ptr<function, Composite>, layer: u32, kind: u32) {
    if ((*c).open && (*c).layer == layer) { return; }
    close_layer(c);
    (*c).acc = vec2<f32>(0.0, 0.0);
    (*c).coverage = 0.0;
    (*c).layer = layer;
    (*c).kind = kind;
    (*c).open = true;
}

// How much of the cell the layer has written after an object, the port of
// `covered` in cpu.rs: a mask subtracts, a blend accumulates, a replace sets.
fn covered(object: Object, before: f32, weight: f32) -> f32 {
    if (object.erases != 0u) { return before * (1.0 - weight); }
    if (object.edge_mode == 1u) { return weight; }
    return before + (1.0 - before) * weight;
}

fn apply_raster(c: ptr<function, Composite>, raster: Raster, position: vec2<f32>) {
    enter_layer(c, raster.layer_kind & 0xffffu, raster.layer_kind >> 16u);
    let sampled = sample_raster(raster, position);
    if (sampled.z > 0.5 && kept(raster, sampled.xy)) {
        (*c).acc = sampled.xy;
        (*c).coverage = 1.0;
    }
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= arrayLength(&samples)) { return; }

    let position = samples[index];
    var c: Composite;
    c.out_uv = vec2<f32>(0.0, 0.0);
    c.out_coverage = 0.0;
    c.out_kind = 1.0;
    c.strong = false;
    c.acc = vec2<f32>(0.0, 0.0);
    c.coverage = 0.0;
    c.layer = 0u;
    c.kind = 1u;
    c.open = false;

    // Imported fields are interleaved with the objects by z, as in
    // `sample_layer`: a raster at z is applied just before object z. Where the
    // grid has a value it overwrites outright. A layer boundary — an object
    // or a raster of another layer — stacks the layer so far and starts the
    // next (M31).
    let raster_count = arrayLength(&rasters);
    var next_raster = 0u;

    let count = arrayLength(&objects);
    for (var o = 0u; o < count; o = o + 1u) {
        while (next_raster < raster_count && rasters[next_raster].z <= o) {
            apply_raster(&c, rasters[next_raster], position);
            next_raster = next_raster + 1u;
        }
        let object = objects[o];
        enter_layer(&c, object.layer, object.kind);

        // Coverage, the port of `coverage` in cpu.rs: the cap cull, the signed
        // distance, the feather ramp, and the inversion that turns all three
        // inside out. An inverted object covers everything outside its
        // footprint, so a cell past the cap is inside it at full weight.
        let inverted = object.invert != 0u;
        let ground = distance_m(object.anchor, position);
        var weight = 1.0;
        if (ground > object.cap_radius_m) {
            if (!inverted) { continue; }
        } else {
            let local_cover = to_local(object, position);
            let cover_distance = shape_distance(object, local_cover);
            if (cover_distance > 0.0 && !inverted) { continue; }
            let band = object.feather * object.feather_reference;
            var inside = 1.0;
            if (band > 0.0) {
                inside = smooth_step(0.0, band, -cover_distance);
            }
            if (inverted) { weight = 1.0 - inside; } else { weight = inside; }
        }

        let local = to_local(object, position);

        // A modifier rewrites what is already in the buffer — everything below
        // it in its layer — and fades from the old to the new by the same
        // weight (spec.md 6.3, 7.6).
        if (object.mod_kind != 0u) {
            let modified = modified_vector(object, position, c.acc);
            c.acc = c.acc + (modified - c.acc) * weight;
            continue;
        }

        let speed = max(speed_at(object, local), 0.0);
        let bearing = direction_at(object, position, local);
        // Added before the edge mode, so the feather fades the sum rather
        // than the two separately (spec.md 9.3).
        let vector = uv_from(speed, bearing) + motion_uv(object, position);

        c.coverage = covered(object, c.coverage, weight);
        if (object.edge_mode == 1u) {
            c.acc = vector * weight;
        } else {
            c.acc = c.acc + (vector - c.acc) * weight;
        }
    }
    while (next_raster < raster_count && rasters[next_raster].z <= count) {
        apply_raster(&c, rasters[next_raster], position);
        next_raster = next_raster + 1u;
    }
    close_layer(&c);

    output[index] = vec4<f32>(c.out_uv, c.out_coverage, c.out_kind);
}
