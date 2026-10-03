"""Writes a small file in the shape of CMC's daily SST analysis.

Run with h5py: `python3 make.py` in this folder. The real file
(`YYYYMMDD120000-CMC-L4_GHRSST-SSTfnd-CMC0.1deg-GLOB-v02.0-fv03.0.nc`) is
NetCDF-4 behind superblock 0: `analysed_sst` over (time, lat, lon) as int16
kelvin packed by 0.01 from 273.15, fill -32768, with `lat` -90 to 90
including both poles and `lon` -180 to 179.9. This keeps that layout on a
1 degree grid.

20 C equatorward of 60 degrees, -1.8 C poleward, and land (the fill value)
for longitudes 0 to 9.
"""

import h5py
import numpy as np

lat = np.arange(-90.0, 90.5, 1.0, dtype=np.float32)
lon = np.arange(-180.0, 180.0, 1.0, dtype=np.float32)
celsius = np.where(np.abs(lat)[:, None] > 60.0, -1.8, 20.0) * np.ones((1, lon.size))
raw = np.round(celsius * 100.0).astype(np.int16)
raw[:, (lon >= 0) & (lon < 10)] = -32768

with h5py.File("20261001120000-CMC-L4_GHRSST-SSTfnd-CMC0.1deg-GLOB-v02.0-fv03.0.nc", "w",
               libver=("earliest", "v108")) as f:
    f.create_dataset("lat", data=lat).attrs["units"] = np.bytes_(b"degrees_north")
    f.create_dataset("lon", data=lon).attrs["units"] = np.bytes_(b"degrees_east")
    t = f.create_dataset("time", data=np.array([1790812800], dtype=np.int32))
    t.attrs["units"] = np.bytes_(b"seconds since 1981-01-01 00:00:00")
    d = f.create_dataset("analysed_sst", data=raw[None, :, :], chunks=(1, 91, 180),
                         compression="gzip", shuffle=True, fillvalue=np.int16(-32768))
    d.attrs["units"] = np.bytes_(b"kelvin")
    d.attrs["scale_factor"] = np.float32(0.01)
    d.attrs["add_offset"] = np.float32(273.15)
    d.attrs["_FillValue"] = np.int16(-32768)
