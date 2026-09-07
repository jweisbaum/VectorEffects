//! Reads ERA5 wind and GlobCurrent surface current from their public Zarr
//! archives (spec.md 4.10).
//!
//! Both archives are anonymous, so no credential is involved. Chunks arrive
//! over HTTPS through [`http`] and are decoded by `zarrs` with the blosc
//! codec in [`blosc`], which keeps the build free of a C or C++ toolchain.
//!
//! **This is the one crate that reaches the network**, and only when the user
//! asks it to: an import is a file dialog that happens to read from an
//! archive rather than a disk (spec.md 1.5, invariant 5). Nothing here runs
//! on a timer, on startup, or behind the user's back.
//!
//! Every source hands back fields on the ERA5 0.25 degree grid — 1440 x 721,
//! north to south from the pole and east from the prime meridian — so a field
//! goes into a GRIB2 message as it is, and wind and current share a grid
//! definition.
//!
//! The readers were written for the GribHistory application and are carried
//! here with their tests; the archives, their quirks and the reasoning about
//! provisional hours are the same in both.

pub mod blosc;
pub mod codec;
pub mod era5;
pub mod error;
pub mod globcurrent;
pub mod http;
pub mod regrid;
pub mod source;
pub mod store;
pub mod time;

pub use era5::{DEFAULT_STORE_URL, Era5Store};
pub use error::{Result, ZarrError};
pub use globcurrent::GlobCurrentStore;
pub use source::{Field, FieldSource, NI, NJ, POINTS_PER_STEP, Step, Variable};
pub use time::Utc;

/// Which archive an import reads.
///
/// One layer each (spec.md 4.10): ERA5 gives 10 m wind and GlobCurrent gives
/// the total surface current, and a project can hold both at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    /// ERA5 10 m wind.
    Era5Wind,
    /// GlobCurrent total surface current.
    GlobCurrent,
}

impl Archive {
    /// Every archive, in the order an import reads them.
    pub const ALL: [Self; 2] = [Self::Era5Wind, Self::GlobCurrent];

    /// The identifier the frontend sends and the document stores.
    pub fn id(self) -> &'static str {
        match self {
            Self::Era5Wind => "era5-wind",
            Self::GlobCurrent => "globcurrent",
        }
    }

    /// What the layer is called.
    pub fn label(self) -> &'static str {
        match self {
            Self::Era5Wind => "ERA5 10 m wind",
            Self::GlobCurrent => "GlobCurrent surface current",
        }
    }

    /// Reads an identifier back.
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|archive| archive.id() == id)
    }

    /// Opens the archive at its published location.
    pub fn open(self) -> Result<Box<dyn FieldSource>> {
        Ok(match self {
            Self::Era5Wind => Box::new(Era5Store::open(DEFAULT_STORE_URL)?),
            Self::GlobCurrent => Box::new(GlobCurrentStore::open(
                globcurrent::DEFAULT_MY_URL,
                globcurrent::DEFAULT_NRT_URL,
            )?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identifier is what the document keeps, so it has to survive a
    /// round trip: a layer whose archive cannot be read back is a layer
    /// nothing can refresh.
    #[test]
    fn every_archive_round_trips_through_its_identifier() {
        for archive in Archive::ALL {
            assert_eq!(Archive::parse(archive.id()), Some(archive));
            assert!(!archive.label().is_empty());
        }
        assert_eq!(Archive::parse("gfs"), None);
    }
}
