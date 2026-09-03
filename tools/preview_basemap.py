#!/usr/bin/env python3
"""Renders assets/basemap.bin to a PNG for visual inspection.

A verification tool, not part of the build. Rasterising the converted binary is
the only cheap way to confirm the triangulation, the hole handling, and the
antimeridian unwrapping are all correct -- a bad ring is obvious in a picture
and nearly invisible in a hex dump.

    python3 tools/preview_basemap.py [--lod 110|50] [--out FILE] [--width N]
"""
import argparse
import struct
import sys
import zlib
from pathlib import Path

SEA = (14, 32, 56)
LAND = (58, 82, 66)
COAST = (150, 200, 220)


def read_basemap(path):
    data = Path(path).read_bytes()
    if data[:4] != b"VEBM":
        sys.exit(f"{path}: bad magic {data[:4]!r}")
    pos = 4
    version, lod_count = struct.unpack_from("<II", data, pos)
    pos += 8
    lods = []
    for _ in range(lod_count):
        marker, ntv, nti, nlv, nls = struct.unpack_from("<IIIII", data, pos)
        pos += 20
        tv = struct.unpack_from(f"<{ntv * 2}f", data, pos)
        pos += ntv * 8
        ti = struct.unpack_from(f"<{nti}I", data, pos)
        pos += nti * 4
        lv = struct.unpack_from(f"<{nlv * 2}f", data, pos)
        pos += nlv * 8
        ls = struct.unpack_from(f"<{nls * 2}I", data, pos)
        pos += nls * 8
        lods.append({
            "marker": marker,
            "tri_vertices": tv, "tri_indices": ti,
            "line_vertices": lv, "line_strips": ls,
        })
    return version, lods


def write_png(path, w, h, pixels):
    raw = b"".join(b"\x00" + bytes(pixels[y * w * 3:(y + 1) * w * 3]) for y in range(h))

    def chunk(tag, payload):
        body = tag + payload
        return struct.pack(">I", len(payload)) + body + struct.pack(">I", zlib.crc32(body))

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 6))
    png += chunk(b"IEND", b"")
    Path(path).write_bytes(png)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lod", type=int, default=110)
    ap.add_argument("--out", default="basemap-preview.png")
    ap.add_argument("--width", type=int, default=1440)
    ap.add_argument("--bin", default="assets/basemap.bin")
    args = ap.parse_args()

    version, lods = read_basemap(args.bin)
    lod = next((l for l in lods if l["marker"] == args.lod), None)
    if lod is None:
        sys.exit(f"no LOD {args.lod}; have {[l['marker'] for l in lods]}")

    w = args.width
    h = w // 2
    px = bytearray(SEA * (w * h))

    def to_pixel(lon, lat):
        return ((lon + 180.0) / 360.0 * w, (90.0 - lat) / 180.0 * h)

    tv, ti = lod["tri_vertices"], lod["tri_indices"]
    for t in range(0, len(ti), 3):
        pts = []
        for k in range(3):
            i = ti[t + k]
            pts.append(to_pixel(tv[i * 2], tv[i * 2 + 1]))
        (x0, y0), (x1, y1), (x2, y2) = pts
        minx, maxx = max(0, int(min(x0, x1, x2))), min(w - 1, int(max(x0, x1, x2)) + 1)
        miny, maxy = max(0, int(min(y0, y1, y2))), min(h - 1, int(max(y0, y1, y2)) + 1)
        denom = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2)
        if abs(denom) < 1e-12:
            continue
        for y in range(miny, maxy + 1):
            for x in range(minx, maxx + 1):
                cx, cy = x + 0.5, y + 0.5
                a = ((y1 - y2) * (cx - x2) + (x2 - x1) * (cy - y2)) / denom
                b = ((y2 - y0) * (cx - x2) + (x0 - x2) * (cy - y2)) / denom
                if a >= 0 and b >= 0 and a + b <= 1:
                    o = (y * w + x) * 3
                    px[o:o + 3] = bytes(LAND)

    lv, ls = lod["line_vertices"], lod["line_strips"]
    for s in range(0, len(ls), 2):
        off, n = ls[s], ls[s + 1]
        for k in range(n):
            i0 = off + k
            i1 = off + (k + 1) % n
            xa, ya = to_pixel(lv[i0 * 2], lv[i0 * 2 + 1])
            xb, yb = to_pixel(lv[i1 * 2], lv[i1 * 2 + 1])
            steps = int(max(abs(xb - xa), abs(yb - ya))) + 1
            if steps > 4000:
                continue
            for j in range(steps + 1):
                t2 = j / steps
                x, y = int(xa + (xb - xa) * t2), int(ya + (yb - ya) * t2)
                if 0 <= x < w and 0 <= y < h:
                    o = (y * w + x) * 3
                    px[o:o + 3] = bytes(COAST)

    write_png(args.out, w, h, px)
    print(f"format v{version}, LOD {args.lod}: "
          f"{len(ti)//3} triangles, {len(ls)//2} rings -> {args.out} ({w}x{h})")


if __name__ == "__main__":
    main()
