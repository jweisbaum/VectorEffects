# routing_test metadata

The metadata documents and the four coordinate chunks of `routing_test`, the
ERA5 routing store the Zarr export is modelled on, copied verbatim. It was
written by zarr-python 3.4 (`build_routing_test.py`), not by this crate, which
is the point: `tests/routing_layout.rs` holds the export's own documents equal
to these. The `data` chunks are 1.6 GB and are not here.

Re-copy them only if the routing store's layout changes.
