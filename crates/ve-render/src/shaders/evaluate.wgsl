// Field evaluation on the GPU.
//
// A port of `cpu.rs`, which is the authority: this exists only to make the
// preview fast. The two need to agree perceptually, not numerically — speed
// within max(0.25 m/s, 2%), direction within 2 degrees (spec.md 7.9) — so this
// works in f32 and uses whatever approximations the hardware offers.
//
// Not handled here, deliberately:
//   - the clone stamp, which reads the composite beneath itself and so needs
//     recursion; scenes containing one fall back to the CPU.
// Everything else follows the same order of operations as the CPU path.

const EARTH_RADIUS_M: f32 = 6371229.0;
const PI: f32 = 3.14159265358979;
const DEG: f32 = 0.01745329251994;
const RADIAL_EPSILON_M: f32 = 1.0;
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
    divergence: f32,
    curl: f32,

    edge_mode: u32,           // 0 blend, 1 replace
    gradient_axis: f32,
    gradient_extent: f32,
    // Sent rather than re-derived here: computing it independently on each
    // side is exactly how two backends quietly disagree.
    feather_reference: f32,

    path_offset: u32,         // the ordered path, for along-path direction
    path_count: u32,
    space: u32,               // 0 geodesic (ground metres), 1 projected (map metres)
    pad0: f32,
};

@group(0) @binding(0) var<storage, read> objects: array<Object>;
@group(0) @binding(1) var<storage, read> points: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read> samples: array<vec2<f32>>;   // lon, lat
@group(0) @binding(3) var<storage, read_write> output: array<vec2<f32>>; // u, v

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

fn uv_from(speed: f32, bearing_deg: f32) -> vec2<f32> {
    let theta = bearing_deg * DEG;
    return vec2<f32>(speed * sin(theta), speed * cos(theta));
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= arrayLength(&samples)) { return; }

    let position = samples[index];
    var accumulated = vec2<f32>(0.0, 0.0);

    let count = arrayLength(&objects);
    for (var o = 0u; o < count; o = o + 1u) {
        let object = objects[o];

        let ground = distance_m(object.anchor, position);
        if (ground > object.cap_radius_m) { continue; }

        let local = to_local(object, position);
        let signed_distance = shape_distance(object, local);
        if (signed_distance > 0.0) { continue; }

        let band = object.feather * object.feather_reference;
        var weight = 1.0;
        if (band > 0.0) {
            weight = smooth_step(0.0, band, -signed_distance);
        }

        let speed = max(speed_at(object, local), 0.0);
        let bearing = direction_at(object, position, local);
        var vector = uv_from(speed, bearing);

        if ((object.divergence != 0.0 || object.curl != 0.0) && ground > RADIAL_EPSILON_M) {
            let radial = initial_bearing(object.anchor, position);
            vector = vector
                + uv_from(object.divergence * speed, radial)
                + uv_from(object.curl * speed, radial + 90.0);
        }

        if (object.edge_mode == 1u) {
            accumulated = vector * weight;
        } else {
            accumulated = accumulated + (vector - accumulated) * weight;
        }
    }

    output[index] = accumulated;
}
