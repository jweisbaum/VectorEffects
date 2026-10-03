"""Writes the HDF5 fixtures ve-hdf5 is tested against.

Run with h5py (any version on HDF5 1.10): `python3 make.py` in this folder.
The files are written by the reference HDF5 library, never by the reader
under test, and every value in them is a formula the Rust tests compute
independently — so the tests check the reader against the library, not
against itself.

`netcdf4.h5` has the layout netCDF-C writes and NOAA's and the Met Office's
NetCDF-4 files use: superblock 2, version 2 object headers, chunked data
behind version 1 B-trees, deflate and shuffle, and more than eight
attributes per object, which puts them in dense storage (a fractal heap and
a version 2 B-tree).

`superblock0.h5` is CMC's shape: superblock 0 around new-style groups.

`dense_links.h5` has more links than fit compactly, so the root group's
links are in dense storage too.
"""

import h5py
import numpy as np


def wind(t, j, i):
    return np.float32(t * 100 + j * 10 + i + 0.5)


def write_common(f):
    # 3 x 5 x 7 floats in 2 x 3 x 4 chunks: edge chunks run past the array.
    w = np.fromfunction(wind, (3, 5, 7), dtype=np.float32)
    w[1, 2, 3] = -9999.0
    d = f.create_dataset("wind", data=w, chunks=(2, 3, 4), compression="gzip",
                         shuffle=True, fillvalue=np.float32(-9999.0))
    d.attrs["units"] = np.bytes_(b"m s-1")
    d.attrs["_FillValue"] = np.float32(-9999.0)

    # Packed temperatures, as a GHRSST file packs them.
    sst = (np.arange(24, dtype=np.int16).reshape(4, 6) * 50) - 300
    sst[3, 5] = -32768
    s = f.create_dataset("sst", data=sst, chunks=(2, 4), compression="gzip",
                         fillvalue=np.int16(-32768))
    s.attrs["scale_factor"] = np.float32(0.01)
    s.attrs["add_offset"] = np.float32(273.15)
    s.attrs["_FillValue"] = np.int16(-32768)
    s.attrs["units"] = np.bytes_(b"kelvin")
    s.attrs.create("long_name", "sea surface foundation temperature",
                   dtype=h5py.string_dtype("utf-8"))
    for k in range(10):  # past eight: dense storage
        s.attrs[f"extra_{k}"] = np.int32(k * 3)

    # Unfiltered, contiguous, as netCDF writes a small time axis.
    t = f.create_dataset("time", data=np.array([420768, 420774], dtype=np.int32))
    t.attrs["units"] = np.bytes_(b"hours since 1978-01-01")

    # Big-endian doubles.
    f.create_dataset("big", data=np.array([1.5, -2.25, 1e10], dtype=">f8"))

    # A hundred chunks: more than one version 1 B-tree node can hold.
    f.create_dataset("many", data=(np.arange(200) % 120).astype(np.int8),
                     chunks=(2,), compression="gzip")

    # Enough attributes to give the fractal heap an indirect block and the
    # name index an internal node, as a real GHRSST file's do.
    a = f.create_dataset("attributes", data=np.zeros(1, dtype=np.int8))
    for k in range(200):
        a.attrs[f"a{k:03}"] = np.arange(8, dtype=np.float64) + k

    for k in range(12):
        f.attrs[f"global_{k}"] = np.float64(k / 4)
    f.attrs["title"] = np.bytes_(b"ve-hdf5 fixture")


with h5py.File("netcdf4.h5", "w", libver=("v108", "v108")) as f:
    write_common(f)

with h5py.File("superblock0.h5", "w", libver=("earliest", "v108")) as f:
    write_common(f)

with h5py.File("dense_links.h5", "w", libver=("v108", "v108")) as f:
    for k in range(20):
        f.create_dataset(f"v{k:02}", data=np.full(3, k, dtype=np.int32))
