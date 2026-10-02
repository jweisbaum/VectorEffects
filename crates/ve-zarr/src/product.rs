//! The near-real-time products an import can fetch (spec.md 4.10).
//!
//! Each is one Copernicus Marine dataset read through [`crate::arco`], found
//! through the catalogue ([`crate::stac`]) because its address is not a
//! constant. What a product *is* — which arrays, which field, how long each
//! of its times stands for — is here; where it lives is asked for.

use crate::arco::{ArcoSpec, ArcoStore};
use crate::error::Result;
use crate::source::{FieldSource, Variable};

/// A product fetched for "the last N days".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// Total surface current, hourly: geostrophic plus Ekman plus tide.
    Multiobs,
    /// Geostrophic surface current from altimetry, daily.
    Duacs,
    /// Blended scatterometer and model wind at 10 m, hourly.
    WindL4,
}

/// Where one product is in the Marine Data Store.
struct Home {
    product: &'static str,
    dataset: &'static str,
    /// The store's address as observed on 2026-10-02, for a day the
    /// catalogue cannot be read. It goes stale when the product is reissued,
    /// which is why it is the fallback and not the answer.
    fallback: &'static str,
}

impl Product {
    /// Every product, in the order the dialog lists them.
    pub const ALL: [Self; 3] = [Self::Multiobs, Self::Duacs, Self::WindL4];

    /// The identifier the frontend sends and the document stores.
    pub fn id(self) -> &'static str {
        match self {
            Self::Multiobs => "multiobs",
            Self::Duacs => "duacs",
            Self::WindL4 => "wind-l4",
        }
    }

    /// What the layer is called.
    pub fn label(self) -> &'static str {
        match self {
            Self::Multiobs => "Copernicus MULTIOBS surface current",
            Self::Duacs => "Copernicus DUACS geostrophic current",
            Self::WindL4 => "Copernicus L4 hourly wind",
        }
    }

    /// Reads an identifier back.
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|product| product.id() == id)
    }

    /// The field it carries.
    pub fn variable(self) -> Variable {
        match self {
            Self::Multiobs | Self::Duacs => Variable::SurfaceCurrent,
            Self::WindL4 => Variable::Wind10m,
        }
    }

    /// How long each of its times stands for, in hours (spec.md 4.10): a
    /// daily analysis is the field for its day.
    pub fn period_hours(self) -> u32 {
        match self {
            Self::Multiobs | Self::WindL4 => 1,
            Self::Duacs => 24,
        }
    }

    /// The line its licence asks to be shown.
    pub fn credit(self) -> &'static str {
        "Generated using E.U. Copernicus Marine Service Information"
    }

    fn home(self) -> Home {
        match self {
            Self::Multiobs => Home {
                product: "MULTIOBS_GLO_PHY_MYNRT_015_003",
                dataset: "cmems_obs-mob_glo_phy-cur_nrt_0.25deg_PT1H-i",
                fallback: crate::globcurrent::DEFAULT_NRT_URL,
            },
            Self::Duacs => Home {
                product: "SEALEVEL_GLO_PHY_L4_NRT_008_046",
                dataset: "cmems_obs-sl_glo_phy-ssh_nrt_allsat-l4-duacs-0.125deg_P1D",
                fallback: "https://s3.waw3-1.cloudferro.com/mdl-arco-time-045/arco/SEALEVEL_GLO_PHY_L4_NRT_008_046/cmems_obs-sl_glo_phy-ssh_nrt_allsat-l4-duacs-0.125deg_P1D_202506/timeChunked.zarr",
            },
            Self::WindL4 => Home {
                product: "WIND_GLO_PHY_L4_NRT_012_004",
                dataset: "cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H",
                fallback: "https://s3.waw3-1.cloudferro.com/mdl-arco-time-050/arco/WIND_GLO_PHY_L4_NRT_012_004/cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H_202207/timeChunked.zarr",
            },
        }
    }

    fn spec(self) -> ArcoSpec {
        match self {
            Self::Multiobs => ArcoSpec {
                name: "MULTIOBS",
                variable: Variable::SurfaceCurrent,
                u_path: "/uo",
                v_path: "/vo",
                // The surface; the store also holds 15 m down.
                level: Some(("/elevation", 0.0)),
            },
            Self::Duacs => ArcoSpec {
                name: "DUACS",
                variable: Variable::SurfaceCurrent,
                u_path: "/ugos",
                v_path: "/vgos",
                level: None,
            },
            Self::WindL4 => ArcoSpec {
                name: "L4 wind",
                variable: Variable::Wind10m,
                u_path: "/eastward_wind",
                v_path: "/northward_wind",
                level: None,
            },
        }
    }

    /// Opens the product where the catalogue says it is.
    ///
    /// If the catalogue cannot be read — or lists the dataset somewhere that
    /// will not open — the last address known to work is tried before giving
    /// up, and the failure reported is the catalogue's: it is the one that
    /// says what is actually wrong.
    pub fn open(self) -> Result<Box<dyn FieldSource>> {
        let home = self.home();
        let found = crate::stac::discover(home.product, home.dataset)
            .and_then(|url| ArcoStore::open(&url, self.spec()));
        match found {
            Ok(store) => Ok(Box::new(store)),
            Err(first) => match ArcoStore::open(home.fallback, self.spec()) {
                Ok(store) => Ok(Box::new(store)),
                Err(_) => Err(first),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identifier is what the document keeps, so it has to survive a
    /// round trip, and no two products may share one.
    #[test]
    fn every_product_round_trips_through_its_identifier() {
        for product in Product::ALL {
            assert_eq!(Product::parse(product.id()), Some(product));
            assert!(!product.label().is_empty());
            assert!(!product.credit().is_empty());
            // A history archive's identifier names a different thing.
            assert_eq!(crate::Archive::parse(product.id()), None);
        }
        assert_eq!(Product::parse("era5-wind"), None);
    }

    /// The periods the timeline's steps divide or are divided by: one of 1,
    /// 6 and 24 hours, which is what lets a product stride a coarser
    /// timeline and hold on a finer one.
    #[test]
    fn every_period_is_one_the_timeline_s_steps_fit() {
        for product in Product::ALL {
            assert!([1, 6, 24].contains(&product.period_hours()), "{product:?}");
        }
        assert_eq!(Product::Duacs.period_hours(), 24);
        assert_eq!(Product::Duacs.variable(), Variable::SurfaceCurrent);
        assert_eq!(Product::WindL4.variable(), Variable::Wind10m);
    }
}
