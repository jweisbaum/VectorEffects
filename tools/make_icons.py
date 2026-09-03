#!/usr/bin/env python3
"""Generates the application icon set from signed distance fields.

No image library dependency: the icon is rasterised from SDFs (the same idea the
renderer itself uses) and written as raw PNG. Regenerate with:

    python3 tools/make_icons.py
"""
import math
import struct
import zlib
from pathlib import Path

SIZE = 1024
BG = (0x0E, 0x1A, 0x2B)
ARROWS = [
    # (start_x, start_y, angle_deg, length, thickness, colour)
    (0.17, 0.72, -18.0, 0.52, 0.038, (0x6F, 0xD9, 0xFF)),
    (0.15, 0.50, -10.0, 0.62, 0.038, (0xFF, 0xFF, 0xFF)),
    (0.17, 0.28, -2.0, 0.48, 0.038, (0x4A, 0x8F, 0xC4)),
]


def sd_round_box(px, py, half, radius):
    dx = abs(px) - half + radius
    dy = abs(py) - half + radius
    outside = math.hypot(max(dx, 0.0), max(dy, 0.0))
    return outside + min(max(dx, dy), 0.0) - radius


def sd_segment(px, py, ax, ay, bx, by):
    pax, pay = px - ax, py - ay
    bax, bay = bx - ax, by - ay
    denom = bax * bax + bay * bay
    h = 0.0 if denom == 0 else max(0.0, min(1.0, (pax * bax + pay * bay) / denom))
    return math.hypot(pax - bax * h, pay - bay * h)


def arrow_sdf(px, py, sx, sy, angle_deg, length, thickness):
    """Shaft plus two barbs, all capsules. Returns signed distance."""
    a = math.radians(angle_deg)
    ex, ey = sx + math.cos(a) * length, sy + math.sin(a) * length
    best = sd_segment(px, py, sx, sy, ex, ey) - thickness
    # Barbs are shorter and finer than the shaft; at full shaft weight they
    # merge into a blob at icon sizes.
    barb = length * 0.22
    for spread in (152.0, -152.0):
        b = a + math.radians(spread)
        bx, by = ex + math.cos(b) * barb, ey + math.sin(b) * barb
        best = min(best, sd_segment(px, py, ex, ey, bx, by) - thickness * 0.82)
    return best


def over(dst, src, alpha):
    return tuple(round(s * alpha + d * (1.0 - alpha)) for d, s in zip(dst, src))


def render(size):
    px_unit = 1.0 / size
    aa = 1.2 * px_unit  # antialias width, in normalised units
    rows = []
    for j in range(size):
        row = bytearray()
        y = 1.0 - (j + 0.5) * px_unit
        for i in range(size):
            x = (i + 0.5) * px_unit
            bg_d = sd_round_box(x - 0.5, y - 0.5, 0.5, 0.22)
            bg_cov = max(0.0, min(1.0, 0.5 - bg_d / aa))
            colour = BG
            for sx, sy, ang, ln, th, tint in ARROWS:
                d = arrow_sdf(x, y, sx, sy, ang, ln, th)
                cov = max(0.0, min(1.0, 0.5 - d / aa))
                if cov > 0.0:
                    colour = over(colour, tint, cov)
            row += bytes(colour) + bytes((round(bg_cov * 255),))
        rows.append(bytes(row))
    return rows


def write_png(path, size, rows):
    raw = b"".join(b"\x00" + r for r in rows)

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    Path(path).write_bytes(png)


def write_ico(path, png_bytes, size):
    """Modern ICO wrapping a PNG payload. 0 in the size byte means 256."""
    entry = struct.pack(
        "<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(png_bytes), 6 + 16
    )
    Path(path).write_bytes(struct.pack("<HHH", 0, 1, 1) + entry + png_bytes)


if __name__ == "__main__":
    out = Path(__file__).resolve().parent.parent / "crates/ve-app/icons"
    out.mkdir(parents=True, exist_ok=True)
    print(f"rendering {SIZE}x{SIZE}...")
    write_png(out / "icon.png", SIZE, render(SIZE))
    print("rendering 256x256 for .ico...")
    write_png(out / "_ico_src.png", 256, render(256))
    write_ico(out / "icon.ico", (out / "_ico_src.png").read_bytes(), 256)
    (out / "_ico_src.png").unlink()
    print("done")
