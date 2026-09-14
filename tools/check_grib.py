#!/usr/bin/env python3
"""Checks an exported GRIB2 against what the sample painted.

Run against `cargo run -p ve-app --example export_sample`, which paints an
eastward stroke on the equator and a northward stamp at 120W 40N.

This is the check that catches a transposed u/v or an inverted direction
convention: both produce a file that parses perfectly and is entirely wrong.
"""
import sys

import numpy as np

try:
    import eccodes
except ImportError:
    sys.exit("python3-eccodes is required")


def read(path):
    fields = {}
    with open(path, "rb") as handle:
        while True:
            gid = eccodes.codes_grib_new_from_file(handle)
            if gid is None:
                break
            key = (eccodes.codes_get(gid, "shortName"), eccodes.codes_get(gid, "step"))
            ni = eccodes.codes_get(gid, "Ni")
            nj = eccodes.codes_get(gid, "Nj")
            values = np.asarray(eccodes.codes_get_values(gid), dtype=float).reshape(nj, ni)
            if eccodes.codes_get(gid, "bitmapPresent"):
                # Request integers explicitly: some distribution builds report
                # bitmap's native type as bytes and generic get_array returns a
                # single byte string instead of the expanded mask.
                bitmap = np.asarray(eccodes.codes_get_long_array(gid, "bitmap")).reshape(nj, ni)
                values[bitmap == 0] = np.nan
            fields[key] = values
            eccodes.codes_release(gid)
    return fields


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "sample.grib2"
    fields = read(path)
    if len(fields) != 6:
        sys.exit(f"expected 6 messages, found {len(fields)}")
    if "--empty" in sys.argv[2:]:
        if not all(np.isnan(values).all() for values in fields.values()):
            sys.exit("an empty export contains defined values")
        print("all six empty messages decode as entirely undefined")
        return

    u = fields[("10u", 0)]
    v = fields[("10v", 0)]

    def at(grid, lon, lat):
        return grid[int(round(90 - lat)), int(round(lon % 360))]

    checks = [
        ("eastward stroke is in u", at(u, 0, 0), 25.0),
        ("eastward stroke is not in v", at(v, 0, 0), 0.0),
        ("northward stamp is in v", at(v, -120, 40), 18.0),
        ("northward stamp is not in u", at(u, -120, 40), 0.0),
        ("unpainted ocean is undefined (u)", at(u, 100, -50), np.nan),
        ("unpainted ocean is undefined (v)", at(v, 100, -50), np.nan),
    ]

    failures = []
    for name, got, want in checks:
        ok = np.isnan(got) if np.isnan(want) else abs(got - want) < 0.01
        print(f"  {'ok  ' if ok else 'FAIL'} {name:32} got {got:8.3f} want {want:8.3f}")
        if not ok:
            failures.append(name)

    # Meteorological direction, as a routing engine would derive it.
    speed = float(np.hypot(at(u, 0, 0), at(v, 0, 0)))
    direction = float((270 - np.degrees(np.arctan2(at(v, 0, 0), at(u, 0, 0)))) % 360)
    ok = abs(direction - 270.0) < 0.5
    print(f"  {'ok  ' if ok else 'FAIL'} {'wind toward east comes from 270':32} "
          f"got {direction:8.1f} want  270.0")
    if not ok:
        failures.append("direction convention")
    print(f"\n  speed at 0E 0N: {speed:.2f} m/s ({speed * 1.94384:.1f} kt)")

    if failures:
        sys.exit(f"\n{len(failures)} check(s) failed: {', '.join(failures)}")
    print("\nall checks passed")


if __name__ == "__main__":
    main()
