"""Writes a small file in the shape of NOAA's Blended Seawinds NRT day file.

Run with h5py: `python3 make.py` in this folder. The real file
(`NBSv02_wind_6hourly_YYYYMMDD_nrt.nc`) is NetCDF-4 with u_wind and v_wind
over (time, zlev, lat, lon), four six-hourly times, latitudes -89.75 to
89.75 and longitudes 0 to 359.75, fill -9999, missing points NaN, chunked
and deflated whole. This one keeps every one of those traits on a 1 degree
grid, so the store's test reads the same layout without 18 MB of fixture.

u at time t is t + 1 m/s, v is -2 m/s, everywhere except: NaN north of
60 N (no observation), and -9999 at the cell (lat 0.5, lon 10.5).
"""

import h5py
import numpy as np

lat = np.arange(-89.5, 90.0, 1.0, dtype=np.float32)
lon = np.arange(0.5, 360.0, 1.0, dtype=np.float32)
# 2026-10-01 00Z, hours since 1978-01-01, as the real file stamps it.
base = 427320
u = np.empty((4, 1, lat.size, lon.size), dtype=np.float32)
for t in range(4):
    u[t] = t + 1.0
v = np.full_like(u, -2.0)
for a in (u, v):
    a[:, :, lat > 60.0, :] = np.nan
    a[:, :, 90, 10] = -9999.0

with h5py.File("NBSv02_wind_6hourly_20261001_nrt.nc", "w", libver=("v108", "v108")) as f:
    f.create_dataset("lat", data=lat).attrs["units"] = np.bytes_(b"degrees_north")
    f.create_dataset("lon", data=lon).attrs["units"] = np.bytes_(b"degrees_east")
    t = f.create_dataset("time", data=np.arange(base, base + 24, 6, dtype=np.int32))
    t.attrs["units"] = np.bytes_(b"hours since 1978-01-01 00:00:00")
    f.create_dataset("zlev", data=np.array([10.0], dtype=np.float32))
    for name, data in (("u_wind", u), ("v_wind", v)):
        d = f.create_dataset(name, data=data, chunks=data.shape, compression="gzip",
                             fillvalue=np.float32(-9999.0))
        d.attrs["_FillValue"] = np.float32(-9999.0)
        d.attrs["units"] = np.bytes_(b"m s-1")
        d.attrs["scale_factor"] = np.float32(1.0)
        d.attrs["add_offset"] = np.float32(0.0)
