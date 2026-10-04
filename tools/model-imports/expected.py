"""Reference values for crates/ve-app/tests/model_imports.rs.

For each packaged model file in a directory, writes `<file>.expected.json`:
the 10 m wind at a few points and every forecast hour, computed without the
application's code.

- A lat/lon file: ecCodes' value at the grid node (the points are nodes).
- An ICON icosahedral file: the three cells nearest the point, by a brute-force
  search over DWD's own CLAT/CLON files (not the bundled asset), weighted by
  inverse chord -- the resampling spec 4.8 specifies. The points are on a
  0.5-degree lattice, so they are nodes of every project grid.
- An ensemble: its first member at each time, which is what an import keeps.

    python3 tools/model-imports/expected.py MODELS_DIR CLATCLON_DIR

Needs numpy and the eccodes bindings. A development tool, not a dependency.
"""
import json, os, sys
import numpy as np
import eccodes as ec

MESHES = {  # grid UUID -> (CLAT, CLON) file names in CLATCLON_DIR
    "a27b8de618c411e4820ab5b098c6a5c0": ("icon_global_icosahedral_time-invariant_2023062000_CLAT.grib2",
                                         "icon_global_icosahedral_time-invariant_2023062000_CLON.grib2"),
    "ae487d14fe2e11e4af85e50a2a56a360": ("icon-eps_global_icosahedral_time-invariant_2023120400_clat.grib2",
                                         "icon-eps_global_icosahedral_time-invariant_2023120400_clon.grib2"),
    "ae487d28fe2e11e4af85e50a2a56a360": ("icon-eu-eps_europe_icosahedral_time-invariant_2023120400_clat.grib2",
                                         "icon-eu-eps_europe_icosahedral_time-invariant_2023120400_clon.grib2"),
    "c6b12daa91ad64045b26c1b6452a2a20": ("icon-d2_germany_icosahedral_time-invariant_2023120400_000_0_clat.grib2",
                                         "icon-d2_germany_icosahedral_time-invariant_2023120400_000_0_clon.grib2"),
}

# Points inside each domain, and points well outside a regional one.
GLOBAL = [(0.0, 0.0), (-30.0, 45.0), (120.0, -30.0), (179.5, 60.0), (-179.5, 60.0),
          (0.0, 89.5), (-70.0, -50.0), (10.0, 51.0)]
EUROPE = [(-10.0, 45.0), (10.0, 50.0), (20.0, 40.0), (0.0, 60.0), (30.0, 55.0)]
GERMANY = [(10.0, 51.0), (13.5, 52.5), (7.0, 50.0), (11.5, 48.0), (6.0, 54.0)]
FRANCE = [(2.5, 48.5), (-1.5, 47.0), (5.0, 44.0), (7.5, 48.5), (0.0, 43.5)]
FAR = [(0.0, 0.0), (-30.0, 45.0), (100.0, 60.0), (120.0, -50.0)]


def unit(lat, lon):
    a, o = np.radians(lat), np.radians(lon)
    return np.stack([np.cos(a) * np.cos(o), np.cos(a) * np.sin(o), np.sin(a)], -1)


def messages(path):
    with open(path, "rb") as f:
        while True:
            g = ec.codes_grib_new_from_file(f)
            if g is None:
                return
            yield g


def points_for(name):
    n = name.lower()
    if "d2" in n:
        return GERMANY, FAR
    if "eu" in n:
        return EUROPE, FAR
    if "arome" in n:
        return FRANCE, FAR
    return GLOBAL, []


def main(models, meshes):
    cache = {}
    for name in sorted(os.listdir(models)):
        if not name.endswith(".grib2"):
            continue
        path = os.path.join(models, name)
        inside, far = points_for(name)
        # First message per (valid time, component): what an import keeps.
        fields = {}
        for g in messages(path):
            short = ec.codes_get(g, "shortName")
            if short not in ("10u", "10v"):
                ec.codes_release(g)
                continue
            valid = ec.codes_get(g, "validityDate") * 10000 + ec.codes_get(g, "validityTime")
            key = (valid, short)
            if key in fields:
                ec.codes_release(g)
                continue
            if ec.codes_get(g, "gridType") == "unstructured_grid":
                uuid = ec.codes_get_string(g, "uuidOfHGrid")
                if uuid not in cache:
                    la, lo = MESHES[uuid]
                    lat = ec.codes_get_values(next(messages(os.path.join(meshes, la))))
                    lon = ec.codes_get_values(next(messages(os.path.join(meshes, lo))))
                    cache[uuid] = unit(lat, lon).astype(np.float32)
                cells = cache[uuid]
                values = ec.codes_get_values(g)
                out = []
                for lon_, lat_ in inside:
                    q = unit(lat_, lon_).astype(np.float32)
                    d = np.sqrt(((cells - q) ** 2).sum(1).astype(np.float64))
                    idx = np.lexsort((np.arange(len(d)), d))[:3]
                    w = 1.0 / d[idx]
                    out.append(float(np.dot(w, values[idx]) / w.sum()))
            else:
                data = ec.codes_grib_get_data(g)
                lats = np.array([p["lat"] for p in data])
                lons = np.array([(p["lon"] + 180.0) % 360.0 - 180.0 for p in data])
                values = np.array([p["value"] for p in data])
                out = []
                for lon_, lat_ in inside:
                    hit = np.nonzero((np.abs(lats - lat_) < 1e-4) & (np.abs(lons - lon_) < 1e-4))[0]
                    assert len(hit) == 1, f"{name}: ({lon_}, {lat_}) is not a node"
                    out.append(float(values[hit[0]]))
            fields[key] = out
            ec.codes_release(g)
        times = sorted({t for t, _ in fields})
        expected = {
            "points": inside,
            "far": far,
            "times": [
                {"valid": f"{str(t)[:4]}-{str(t)[4:6]}-{str(t)[6:8]}T{str(t)[8:10]}:{str(t)[10:12]}:00Z",
                 "u": fields[(t, "10u")], "v": fields[(t, "10v")]}
                for t in times
            ],
        }
        with open(path + ".expected.json", "w") as f:
            json.dump(expected, f, indent=1)
        print(f"{name}: {len(times)} times, {len(inside)} points")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
