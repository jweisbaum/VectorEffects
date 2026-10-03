//! The near-real-time products an import can fetch (spec.md 4.10).
//!
//! Each is one Copernicus Marine dataset read through [`crate::arco`], found
//! through the catalogue ([`crate::stac`]) because its address is not a
//! constant. What a product *is* — which arrays, which field, how long each
//! of its times stands for — is here; where it lives is asked for.

use crate::arco::{ArcoSpec, ArcoStore};
use crate::erddap::{ErddapSpec, ErddapStore};
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
    /// Scatterometer wind at 10 m from Metop-B and Metop-C, daily, the
    /// day's four passes merged.
    Ascat,
    /// Cross-Calibrated Multi-Platform wind at 10 m, near-real-time
    /// (version 2.1 NRT), six-hourly, from a NOAA ERDDAP server.
    Ccmp,
    /// NOAA's daily Optimum Interpolation SST, version 2.1, preliminary
    /// days included: 0.25 degree, from a NOAA ERDDAP server.
    Oisst,
    /// NOAA's Geo-Polar Blended SST, day and night, daily: 0.05 degree, read
    /// at every fifth point, from NOAA CoastWatch's ERDDAP server.
    GeoPolar,
    /// The Met Office's OSTIA SST analysis, daily, through Copernicus
    /// Marine: its 0.2 degree copy of the 0.05 degree field.
    Ostia,
}

/// Where one product is in the Marine Data Store.
struct Home {
    product: &'static str,
    dataset: &'static str,
    /// The store's address as observed on 2026-10-02, for a day the
    /// catalogue cannot be read. It goes stale when the product is reissued,
    /// which is why it is the fallback and not the answer.
    fallback: &'static str,
    /// Which of the dataset's stores: `timeChunked`, or a lighter copy.
    asset: &'static str,
}

impl Product {
    /// Every product, in the order the dialog lists them.
    pub const ALL: [Self; 8] = [
        Self::Multiobs,
        Self::Duacs,
        Self::WindL4,
        Self::Ascat,
        Self::Ccmp,
        Self::Oisst,
        Self::GeoPolar,
        Self::Ostia,
    ];

    /// The identifier the frontend sends and the document stores.
    pub fn id(self) -> &'static str {
        match self {
            Self::Multiobs => "multiobs",
            Self::Duacs => "duacs",
            Self::WindL4 => "wind-l4",
            Self::Ascat => "ascat",
            Self::Ccmp => "ccmp",
            Self::Oisst => "oisst",
            Self::GeoPolar => "geopolar",
            Self::Ostia => "ostia",
        }
    }

    /// What the layer is called.
    pub fn label(self) -> &'static str {
        match self {
            Self::Multiobs => "Copernicus MULTIOBS surface current",
            Self::Duacs => "Copernicus DUACS geostrophic current",
            Self::WindL4 => "Copernicus L4 hourly wind",
            Self::Ascat => "ASCAT Metop-B/C wind",
            Self::Ccmp => "CCMP NRT wind",
            Self::Oisst => "NOAA OISST sea-surface temperature",
            Self::GeoPolar => "NOAA Geo-Polar Blended sea-surface temperature",
            Self::Ostia => "OSTIA sea-surface temperature",
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
            Self::WindL4 | Self::Ascat | Self::Ccmp => Variable::Wind10m,
            Self::Oisst | Self::GeoPolar | Self::Ostia => Variable::SeaSurfaceTemperature,
        }
    }

    /// How long each of its times stands for, in hours (spec.md 4.10): a
    /// daily analysis is the field for its day.
    pub fn period_hours(self) -> u32 {
        match self {
            Self::Multiobs | Self::WindL4 => 1,
            Self::Duacs | Self::Ascat | Self::Oisst | Self::GeoPolar | Self::Ostia => 24,
            Self::Ccmp => 6,
        }
    }

    /// The line its licence asks to be shown.
    pub fn credit(self) -> &'static str {
        match self {
            Self::Ccmp => "CCMP Version-2.1 NRT wind data are produced by Remote Sensing Systems",
            Self::Oisst => {
                "NOAA OI SST V2.1 data provided by the NOAA National Centers for Environmental Information"
            }
            Self::GeoPolar => "NOAA Geo-Polar Blended SST, from NOAA CoastWatch",
            Self::Multiobs | Self::Duacs | Self::WindL4 | Self::Ascat | Self::Ostia => {
                "Generated using E.U. Copernicus Marine Service Information"
            }
        }
    }

    /// Where CCMP is: NOAA's Pacific Islands OceanWatch ERDDAP. The West
    /// Coast node lists the same dataset as `pifscCcmpDailyV21NRT` and, on
    /// 2 October 2026, answered its data requests with a redirect here.
    const CCMP: ErddapSpec = ErddapSpec {
        name: "CCMP",
        server: "https://oceanwatch.pifsc.noaa.gov/erddap",
        dataset: "ccmp-daily-v2-1-NRT",
        variable: Variable::Wind10m,
        u: "uwnd",
        v: Some("vwnd"),
        extra_axes: 0,
        stride: 1,
        daily: false,
    };

    /// OISST on NOAA's West Coast ERDDAP: the near-real-time aggregation,
    /// with its depth axis (`zlev`, one level) between time and latitude.
    const OISST: ErddapSpec = ErddapSpec {
        name: "OISST",
        server: "https://coastwatch.pfeg.noaa.gov/erddap",
        dataset: "ncdcOisst21NrtAgg_LonPM180",
        variable: Variable::SeaSurfaceTemperature,
        u: "sst",
        v: None,
        extra_axes: 1,
        stride: 1,
        daily: true,
    };

    /// Geo-Polar Blended, day and night, on NOAA CoastWatch's ERDDAP; the
    /// server takes every fifth point of its 0.05 degree grid.
    const GEO_POLAR: ErddapSpec = ErddapSpec {
        name: "Geo-Polar",
        server: "https://coastwatch.noaa.gov/erddap",
        dataset: "noaacwBLENDEDsstDNDaily",
        variable: Variable::SeaSurfaceTemperature,
        u: "analysed_sst",
        v: None,
        extra_axes: 0,
        stride: 5,
        daily: true,
    };

    /// The ASCAT passes, each a dataset of its own: Metop-B and Metop-C,
    /// ascending and descending, at 0.25 degree.
    const ASCAT_PASSES: [Home; 4] = [
        Home {
            product: "WIND_GLO_PHY_L3_NRT_012_002",
            dataset: "cmems_obs-wind_glo_phy_nrt_l3-metopb-ascat-asc-0.25deg_P1D-i",
            fallback: "https://s3.waw3-1.cloudferro.com/mdl-arco-time-048/arco/WIND_GLO_PHY_L3_NRT_012_002/cmems_obs-wind_glo_phy_nrt_l3-metopb-ascat-asc-0.25deg_P1D-i_202311/timeChunked.zarr",
            asset: "timeChunked",
        },
        Home {
            product: "WIND_GLO_PHY_L3_NRT_012_002",
            dataset: "cmems_obs-wind_glo_phy_nrt_l3-metopb-ascat-des-0.25deg_P1D-i",
            fallback: "https://s3.waw3-1.cloudferro.com/mdl-arco-time-048/arco/WIND_GLO_PHY_L3_NRT_012_002/cmems_obs-wind_glo_phy_nrt_l3-metopb-ascat-des-0.25deg_P1D-i_202311/timeChunked.zarr",
            asset: "timeChunked",
        },
        Home {
            product: "WIND_GLO_PHY_L3_NRT_012_002",
            dataset: "cmems_obs-wind_glo_phy_nrt_l3-metopc-ascat-asc-0.25deg_P1D-i",
            fallback: "https://s3.waw3-1.cloudferro.com/mdl-arco-time-048/arco/WIND_GLO_PHY_L3_NRT_012_002/cmems_obs-wind_glo_phy_nrt_l3-metopc-ascat-asc-0.25deg_P1D-i_202311/timeChunked.zarr",
            asset: "timeChunked",
        },
        Home {
            product: "WIND_GLO_PHY_L3_NRT_012_002",
            dataset: "cmems_obs-wind_glo_phy_nrt_l3-metopc-ascat-des-0.25deg_P1D-i",
            fallback: "https://s3.waw3-1.cloudferro.com/mdl-arco-time-048/arco/WIND_GLO_PHY_L3_NRT_012_002/cmems_obs-wind_glo_phy_nrt_l3-metopc-ascat-des-0.25deg_P1D-i_202311/timeChunked.zarr",
            asset: "timeChunked",
        },
    ];

    /// Where a single-dataset Copernicus product is, and how it is read;
    /// `None` for the products that are not one Copernicus dataset.
    fn marine(self) -> Option<(Home, ArcoSpec)> {
        Some((self.home()?, self.spec()?))
    }

    fn home(self) -> Option<Home> {
        Some(match self {
            Self::Multiobs => Home {
                product: "MULTIOBS_GLO_PHY_MYNRT_015_003",
                dataset: "cmems_obs-mob_glo_phy-cur_nrt_0.25deg_PT1H-i",
                fallback: crate::globcurrent::DEFAULT_NRT_URL,
                asset: "timeChunked",
            },
            Self::Duacs => Home {
                product: "SEALEVEL_GLO_PHY_L4_NRT_008_046",
                dataset: "cmems_obs-sl_glo_phy-ssh_nrt_allsat-l4-duacs-0.125deg_P1D",
                fallback: "https://s3.waw3-1.cloudferro.com/mdl-arco-time-045/arco/SEALEVEL_GLO_PHY_L4_NRT_008_046/cmems_obs-sl_glo_phy-ssh_nrt_allsat-l4-duacs-0.125deg_P1D_202506/timeChunked.zarr",
                asset: "timeChunked",
            },
            Self::WindL4 => Home {
                product: "WIND_GLO_PHY_L4_NRT_012_004",
                dataset: "cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H",
                fallback: "https://s3.waw3-1.cloudferro.com/mdl-arco-time-050/arco/WIND_GLO_PHY_L4_NRT_012_004/cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H_202207/timeChunked.zarr",
                asset: "timeChunked",
            },
            Self::Ostia => Home {
                product: "SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001",
                dataset: "METOFFICE-GLO-SST-L4-NRT-OBS-SST-V2",
                fallback: "https://s3.waw3-1.cloudferro.com/mdl-arco-time-045/arco/SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001/METOFFICE-GLO-SST-L4-NRT-OBS-SST-V2/downsampled4.zarr",
                // A fifth of a degree: the full field is 26 million values a
                // day, and the layer is drawn on a quarter-degree grid.
                asset: "downsampled4",
            },
            // Four datasets, opened through `ASCAT_PASSES`; and not
            // Copernicus at all.
            Self::Ascat | Self::Ccmp | Self::Oisst | Self::GeoPolar => return None,
        })
    }

    fn spec(self) -> Option<ArcoSpec> {
        Some(match self {
            Self::Multiobs => ArcoSpec {
                name: "MULTIOBS",
                variable: Variable::SurfaceCurrent,
                u_path: "/uo",
                v_path: "/vo",
                // The surface; the store also holds 15 m down.
                level: Some(("/elevation", 0.0)),
                time_path: None,
                daily: false,
            },
            Self::Duacs => ArcoSpec {
                name: "DUACS",
                variable: Variable::SurfaceCurrent,
                u_path: "/ugos",
                v_path: "/vgos",
                level: None,
                time_path: None,
                daily: false,
            },
            Self::WindL4 => ArcoSpec {
                name: "L4 wind",
                variable: Variable::Wind10m,
                u_path: "/eastward_wind",
                v_path: "/northward_wind",
                level: None,
                time_path: None,
                daily: false,
            },
            // One value per point: the "v" array is the same one, opened
            // twice and never read (`ArcoStore` reads only `u` of a scalar).
            Self::Ostia => ArcoSpec {
                name: "OSTIA",
                variable: Variable::SeaSurfaceTemperature,
                u_path: "/analysed_sst",
                v_path: "/analysed_sst",
                level: None,
                time_path: None,
                daily: true,
            },
            Self::Ascat | Self::Ccmp | Self::Oisst | Self::GeoPolar => return None,
        })
    }

    /// Opens one dataset where the catalogue says it is.
    ///
    /// If the catalogue cannot be read — or lists the dataset somewhere that
    /// will not open — the last address known to work is tried before giving
    /// up, and the failure reported is the catalogue's: it is the one that
    /// says what is actually wrong.
    fn open_home(home: &Home, spec: ArcoSpec) -> Result<ArcoStore> {
        let found = crate::stac::discover_asset(home.product, home.dataset, home.asset)
            .and_then(|url| ArcoStore::open(&url, spec));
        match found {
            Ok(store) => Ok(store),
            Err(first) => ArcoStore::open(home.fallback, spec).map_err(|_| first),
        }
    }

    /// Opens the product where the catalogue says it is.
    ///
    /// If the catalogue cannot be read — or lists the dataset somewhere that
    /// will not open — the last address known to work is tried before giving
    /// up, and the failure reported is the catalogue's: it is the one that
    /// says what is actually wrong.
    pub fn open(self) -> Result<Box<dyn FieldSource>> {
        if self == Self::Ascat {
            // The four passes found and opened together, each with its own
            // fallback.
            let members = std::thread::scope(|scope| {
                let handles: Vec<_> = Self::ASCAT_PASSES
                    .iter()
                    .map(|home| scope.spawn(move || Self::open_home(home, crate::ascat::SPEC)))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                    })
                    .collect::<Result<Vec<_>>>()
            })?;
            return Ok(Box::new(crate::ascat::AscatStore::new(members)?));
        }
        let erddap = match self {
            Self::Ccmp => Some(Self::CCMP),
            Self::Oisst => Some(Self::OISST),
            Self::GeoPolar => Some(Self::GEO_POLAR),
            _ => None,
        };
        if let Some(spec) = erddap {
            return Ok(Box::new(ErddapStore::open(
                spec,
                Box::new(crate::erddap::Http::new()?),
            )?));
        }
        let (home, spec) = self
            .marine()
            .ok_or_else(|| crate::error::ZarrError::Open(format!("{} has no store", self.id())))?;
        Ok(Box::new(Self::open_home(&home, spec)?))
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
