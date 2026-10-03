//! Finding a Copernicus Marine dataset's store (spec.md 4.10).
//!
//! The Marine Data Store publishes a STAC catalogue beside its data: one
//! document per product, listing its datasets, and one per dataset, listing
//! where each form of it lives. The address of a store is not a constant —
//! the bucket's number and the dataset's version suffix both change when a
//! product is reissued — so it is read from the catalogue each time rather
//! than written down here.
//!
//! The parsing is separate from the fetching so it can be held to the
//! documents as they were actually served (`tests/fixtures/stac`).

use crate::error::{Result, ZarrError};

/// The catalogue's root on the Marine Data Store.
pub const CATALOGUE: &str = "https://s3.waw3-1.cloudferro.com/mdl-metadata/metadata";

/// The name every dataset's own document has, under its versioned directory.
const DATASET_DOCUMENT: &str = "dataset.stac.json";

/// The one host a store may be read from.
///
/// Invariant 5 names the hosts the application reaches, and the offline
/// check holds the *source* to them — it cannot see an address read out of
/// a catalogue at run time. So the catalogue's answer is held here: a
/// dataset listed anywhere but the Marine Data Store's own object store is
/// refused, and the import falls back to the address last known to work.
pub const STORE_HOST: &str = "s3.waw3-1.cloudferro.com";

fn document(text: &str, what: &str) -> Result<serde_json::Value> {
    serde_json::from_str(text)
        .map_err(|err| ZarrError::Layout(format!("{what} is not the JSON it should be: {err}")))
}

/// The link to a dataset's document, out of its product's.
///
/// A product lists each dataset as `<identifier>_<version>/dataset.stac.json`,
/// the version a run of digits. The identifier has to match whole — DUACS
/// publishes three datasets that differ by a few characters — and where a
/// reissued dataset is listed twice, the later version is the live one.
pub fn dataset_href(product_json: &str, dataset_id: &str) -> Result<String> {
    let product = document(product_json, "the product's catalogue entry")?;
    let found = product["links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|link| link["rel"] == "item")
        .filter_map(|link| link["href"].as_str())
        .filter_map(|href| {
            let directory = href.strip_suffix(DATASET_DOCUMENT)?.strip_suffix('/')?;
            // A dataset that has never been reissued has no version suffix
            // at all: OSTIA's directory is its identifier, and nothing more.
            if directory == dataset_id {
                return Some((String::new(), href.to_owned()));
            }
            let version = directory.strip_prefix(dataset_id)?.strip_prefix('_')?;
            let digits = !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit());
            digits.then(|| (version.to_owned(), href.to_owned()))
        })
        // Versions are dates written as digits, so the longer is the later
        // and equal lengths compare as text.
        .max_by(|a, b| (a.0.len(), &a.0).cmp(&(b.0.len(), &b.0)));
    found.map(|(_, href)| href).ok_or_else(|| {
        ZarrError::Open(format!(
            "{} lists no dataset called {dataset_id}",
            product["id"].as_str().unwrap_or("the product")
        ))
    })
}

/// Where a dataset's time-chunked store is, out of its own document.
///
/// Time-chunked rather than geo-chunked: one chunk row per time, which is
/// the shape a few days of a whole field is read in.
pub fn time_chunked_url(dataset_json: &str) -> Result<String> {
    asset_url(dataset_json, "timeChunked")
}

/// Where one of a dataset's stores is: `timeChunked`, or a lighter copy such
/// as `downsampled4`, which OSTIA publishes at a fifth of a degree beside its
/// twentieth (spec.md 4.10, M93).
pub fn asset_url(dataset_json: &str, asset: &str) -> Result<String> {
    let dataset = document(dataset_json, "the dataset's catalogue entry")?;
    let href = dataset["assets"][asset]["href"].as_str().ok_or_else(|| {
        ZarrError::Open(format!(
            "{} has no {asset} store listed",
            dataset["id"].as_str().unwrap_or("the dataset")
        ))
    })?;
    let host = url::Url::parse(href)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default();
    if host != STORE_HOST {
        return Err(ZarrError::Open(format!(
            "the catalogue lists {} at {host}, which is not a host this application reads",
            dataset["id"].as_str().unwrap_or("the dataset")
        )));
    }
    Ok(href.to_owned())
}

/// Looks a dataset's store up in the live catalogue: two small documents.
pub fn discover(product_id: &str, dataset_id: &str) -> Result<String> {
    discover_asset(product_id, dataset_id, "timeChunked")
}

/// [`discover`], for a named store of the dataset.
pub fn discover_asset(product_id: &str, dataset_id: &str, asset: &str) -> Result<String> {
    let product = crate::http::get_text(&format!("{CATALOGUE}/{product_id}/product.stac.json"))?;
    let href = dataset_href(&product, dataset_id)?;
    let dataset = crate::http::get_text(&format!("{CATALOGUE}/{product_id}/{href}"))?;
    asset_url(&dataset, asset)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIND_PRODUCT: &str = include_str!("../tests/fixtures/stac/wind-product.json");
    const DUACS_PRODUCT: &str = include_str!("../tests/fixtures/stac/duacs-product.json");
    const MULTIOBS_PRODUCT: &str = include_str!("../tests/fixtures/stac/multiobs-product.json");
    const WIND_DATASET: &str = include_str!("../tests/fixtures/stac/wind-dataset.json");
    const DUACS_DATASET: &str = include_str!("../tests/fixtures/stac/duacs-dataset.json");

    #[test]
    fn a_dataset_is_found_among_its_product_s_items() {
        assert_eq!(
            dataset_href(WIND_PRODUCT, "cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H").unwrap(),
            "cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H_202207/dataset.stac.json"
        );
        assert_eq!(
            dataset_href(
                MULTIOBS_PRODUCT,
                "cmems_obs-mob_glo_phy-cur_nrt_0.25deg_PT1H-i"
            )
            .unwrap(),
            "cmems_obs-mob_glo_phy-cur_nrt_0.25deg_PT1H-i_202411/dataset.stac.json"
        );
    }

    /// Three DUACS datasets differ by a resolution and a word; an identifier
    /// names exactly one of them, and a prefix of another's name is not it.
    #[test]
    fn a_dataset_is_matched_by_its_whole_identifier() {
        assert_eq!(
            dataset_href(
                DUACS_PRODUCT,
                "cmems_obs-sl_glo_phy-ssh_nrt_allsat-l4-duacs-0.125deg_P1D"
            )
            .unwrap(),
            "cmems_obs-sl_glo_phy-ssh_nrt_allsat-l4-duacs-0.125deg_P1D_202506/dataset.stac.json"
        );
        // The daily MULTIOBS identifier is a prefix of nothing else, and the
        // hourly one is not found by asking for half of it.
        assert!(
            dataset_href(
                MULTIOBS_PRODUCT,
                "cmems_obs-mob_glo_phy-cur_nrt_0.25deg_PT1H"
            )
            .is_err()
        );
        let missing = dataset_href(WIND_PRODUCT, "no-such-dataset").unwrap_err();
        assert!(
            missing.to_string().contains("WIND_GLO_PHY_L4_NRT_012_004"),
            "{missing}"
        );
    }

    /// A product reissued keeps the old dataset listed for a while; the later
    /// version is the one being updated.
    #[test]
    fn the_later_of_two_versions_is_taken() {
        let reissued = WIND_PRODUCT.replace(
            r#""links": ["#,
            r#""links": [{"rel": "item", "href": "cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H_202609/dataset.stac.json", "type": "application/json"},"#,
        );
        assert_ne!(
            reissued, WIND_PRODUCT,
            "the fixture has a links array to add to"
        );
        assert_eq!(
            dataset_href(&reissued, "cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H").unwrap(),
            "cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H_202609/dataset.stac.json"
        );
    }

    /// OSTIA has never been reissued, so its directory is its identifier;
    /// and its light copy is asked for by name.
    #[test]
    fn an_unversioned_dataset_and_a_named_store_are_found() {
        let product = r#"{"id": "SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001", "links": [
            {"rel": "item", "href": "METOFFICE-GLO-SST-L4-NRT-OBS-SST-V2/dataset.stac.json"}]}"#;
        assert_eq!(
            dataset_href(product, "METOFFICE-GLO-SST-L4-NRT-OBS-SST-V2").unwrap(),
            "METOFFICE-GLO-SST-L4-NRT-OBS-SST-V2/dataset.stac.json"
        );
        assert!(dataset_href(product, "METOFFICE-GLO-SST").is_err());
        let dataset = r#"{"id": "x", "assets": {
            "timeChunked": {"href": "https://s3.waw3-1.cloudferro.com/mdl-arco-time-045/a/timeChunked.zarr"},
            "downsampled4": {"href": "https://s3.waw3-1.cloudferro.com/mdl-arco-time-045/a/downsampled4.zarr"}}}"#;
        assert!(
            asset_url(dataset, "downsampled4")
                .unwrap()
                .ends_with("downsampled4.zarr")
        );
        assert!(asset_url(dataset, "downsampled8").is_err());
    }

    #[test]
    fn the_time_chunked_store_is_read_from_the_dataset() {
        assert_eq!(
            time_chunked_url(WIND_DATASET).unwrap(),
            "https://s3.waw3-1.cloudferro.com/mdl-arco-time-050/arco/WIND_GLO_PHY_L4_NRT_012_004/cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H_202207/timeChunked.zarr"
        );
        assert!(
            time_chunked_url(DUACS_DATASET)
                .unwrap()
                .ends_with("0.125deg_P1D_202506/timeChunked.zarr")
        );
        assert!(time_chunked_url(r#"{"assets": {"native": {"href": "x"}}}"#).is_err());
        assert!(time_chunked_url("not json").is_err());
    }

    /// A dataset the catalogue has moved somewhere this application does not
    /// name is refused, not followed (invariant 5).
    #[test]
    fn a_store_on_another_host_is_refused() {
        let moved = WIND_DATASET.replace(
            "https://s3.waw3-1.cloudferro.com/mdl-arco-time-050/",
            "https://store.example.invalid/mdl-arco-time-050/",
        );
        assert_ne!(moved, WIND_DATASET, "the fixture names the store's host");
        let refused = time_chunked_url(&moved).unwrap_err().to_string();
        assert!(refused.contains("store.example.invalid"), "{refused}");
        // The same path on the same host is still fine.
        assert!(time_chunked_url(WIND_DATASET).is_ok());
    }
}
