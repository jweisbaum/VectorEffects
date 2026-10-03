//! CMC's sea-surface temperature analysis, from NASA PO.DAAC (spec.md 4.10,
//! M97).
//!
//! The Canadian Meteorological Centre's 0.1° daily analysis has no
//! anonymous source: PO.DAAC publishes it behind a NASA Earthdata login. So
//! this is the one product that carries a credential, and the rules for it
//! are the whole point of the module:
//!
//! - **The day list is anonymous.** NASA's catalogue (CMR) is asked which
//!   days exist and where each one's file is; it needs no token and gets
//!   none.
//! - **The token goes to PO.DAAC and nowhere else.** The download is asked
//!   of PO.DAAC's archive with the token as a bearer credential, and the
//!   archive answers with a redirect to a signed address on its CloudFront
//!   distribution. That redirect is followed by hand, without the token:
//!   the signature is the credential there, and a redirect that went
//!   anywhere but CloudFront — NASA's login page, when the token is refused
//!   — is reported rather than followed.
//! - **The token is never in a message.** Errors name the file and the
//!   status, never the request's headers.

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, LOCATION};

use crate::arco::cell_grid_of;
use crate::erddap::{Fetch, regrid_within};
use crate::error::{Result, ZarrError};
use crate::http::{Retry, retrying, transient, with_causes};
use crate::source::{Field, FieldSource, Step, Variable, step_at_hour, steps_between};
use crate::time::Utc;

const NAME: &str = "CMC";
/// The dataset's short name in NASA's catalogue.
const SHORT_NAME: &str = "CMC0.1deg-CMC-L4-GLOB-v3.0";
/// The catalogue's granule search: the newest days first. Three hundred is
/// ten months, more than a 240-step daily timeline can show.
const CATALOGUE: &str = "https://cmr.earthdata.nasa.gov/search/granules.json?short_name=CMC0.1deg-CMC-L4-GLOB-v3.0&sort_key=-start_date&page_size=300";
/// Where every file is downloaded from, and the one host the token is sent to.
const ARCHIVE: &str = "https://archive.podaac.earthdata.nasa.gov/";
/// Where the archive's redirect may lead: its CloudFront distribution.
const REDIRECT_DOMAIN: &str = ".cloudfront.net";

/// The days the catalogue lists, as each day's midnight in hours since the
/// Unix epoch and the address of its file on the archive, oldest first.
///
/// A granule is titled `YYYYMMDD120000-CMC-L4_GHRSST-…` — the analysis is
/// stamped at noon — and filed here under its midnight, as every daily
/// product is.
pub fn days_listed(catalogue: &[u8]) -> Result<Vec<(i64, String)>> {
    let bad = |what: &str| ZarrError::Layout(format!("NASA's catalogue answered {what}"));
    let feed: serde_json::Value =
        serde_json::from_slice(catalogue).map_err(|_| bad("something other than JSON"))?;
    let entries = feed
        .pointer("/feed/entry")
        .and_then(|e| e.as_array())
        .ok_or_else(|| bad("with no list of files"))?;
    let mut days: Vec<(i64, String)> = entries
        .iter()
        .filter_map(|entry| {
            let title = entry.get("title")?.as_str()?;
            let date = title
                .get(..8)
                .filter(|d| d.bytes().all(|b| b.is_ascii_digit()))?;
            let iso = format!("{}-{}-{}T00:00", &date[..4], &date[4..6], &date[6..]);
            let day = Utc::parse(&iso)?.hours_since_unix_epoch();
            let href = entry
                .get("links")?
                .as_array()?
                .iter()
                .filter_map(|link| link.get("href")?.as_str())
                .find(|href| href.starts_with(ARCHIVE) && href.ends_with(".nc"))?;
            Some((day, href.to_owned()))
        })
        .collect();
    days.sort();
    days.dedup_by_key(|(day, _)| *day);
    Ok(days)
}

/// Where a download goes after the archive's first answer.
#[derive(Debug, PartialEq, Eq)]
enum Hop {
    /// Fetch this signed address, without the token.
    Signed(String),
    /// The token was not accepted.
    Refused,
    /// Somewhere this does not go.
    Elsewhere(String),
}

/// Reads the archive's redirect. NASA's login page means the token was not
/// good enough; CloudFront over HTTPS is the file; anything else is not
/// followed.
fn next_hop(location: &str) -> Hop {
    // Parsed as the client will parse it, so the host checked is the host
    // connected to: a `#` or a `\` ends a host for the client and would not
    // for a split of our own.
    let host = reqwest::Url::parse(location)
        .ok()
        .filter(|url| url.scheme() == "https")
        .and_then(|url| {
            url.host_str()
                .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
        });
    match host.as_deref() {
        Some("urs.earthdata.nasa.gov") => Hop::Refused,
        Some(host) if host.ends_with(REDIRECT_DOMAIN) => Hop::Signed(location.to_owned()),
        Some(host) => Hop::Elsewhere(host.to_owned()),
        None => Hop::Elsewhere("an address that is not HTTPS".to_owned()),
    }
}

/// A failed request, said without its address: reqwest ends its message
/// with the URL, and on the signed hop the query holds a credential and the
/// person's username.
fn describe(err: reqwest::Error) -> String {
    with_causes(&err.without_url())
}

/// The archive, with the token. Follows no redirect by itself.
pub struct Earthdata {
    client: reqwest::blocking::Client,
    token: String,
}

impl std::fmt::Debug for Earthdata {
    // Never the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Earthdata")
    }
}

impl Earthdata {
    pub fn new(token: &str) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| ZarrError::Open(format!("no HTTP client: {err}")))?;
        Ok(Self {
            client,
            token: token.trim().to_owned(),
        })
    }

    /// One request, retried while the server says "not now".
    fn request(&self, url: &str, token: Option<&str>) -> Result<reqwest::blocking::Response> {
        retrying(|| {
            let mut request = self.client.get(url);
            if let Some(token) = token {
                request = request.header(AUTHORIZATION, format!("Bearer {token}"));
            }
            let response = request
                .send()
                .map_err(|err| Retry::Later(zarrs_storage::StorageError::Other(describe(err))))?;
            let status = response.status();
            if transient(status) {
                return Err(Retry::Later(zarrs_storage::StorageError::Other(format!(
                    "the server answered {status}"
                ))));
            }
            Ok(response)
        })
        .map_err(|err| ZarrError::Open(format!("could not read {}: {err}", file_name(url))))
    }
}

impl Fetch for Earthdata {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        if !url.starts_with(ARCHIVE) {
            return Err(ZarrError::Open(format!(
                "{} is not on NASA's archive, and the token goes nowhere else",
                file_name(url)
            )));
        }
        let name = file_name(url);
        let refused = || {
            ZarrError::Open(format!(
                "NASA Earthdata did not accept the token for {name}; check it in Settings"
            ))
        };
        let first = self.request(url, Some(&self.token))?;
        let status = first.status();
        let response = if status.is_redirection() {
            let location = first
                .headers()
                .get(LOCATION)
                .and_then(|l| l.to_str().ok())
                .unwrap_or_default();
            match next_hop(location) {
                Hop::Signed(signed) => self.request(&signed, None)?,
                Hop::Refused => return Err(refused()),
                Hop::Elsewhere(host) => {
                    return Err(ZarrError::Open(format!(
                        "NASA's archive sent {name} on to {host}, which is not followed"
                    )));
                }
            }
        } else {
            first
        };
        match response.status() {
            StatusCode::OK => response.bytes().map(|b| b.to_vec()).map_err(|err| {
                ZarrError::Open(format!("could not read {name}: {}", describe(err)))
            }),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(refused()),
            other => Err(ZarrError::Open(format!(
                "{name}: the server answered {other}"
            ))),
        }
    }
}

/// The last part of an address, for a message: never its query, which on
/// a signed address is a credential too.
fn file_name(url: &str) -> &str {
    let path = url.split('?').next().unwrap_or(url);
    path.rsplit('/').next().unwrap_or(path)
}

/// The product, open: the days the catalogue lists.
pub struct CmcStore {
    download: Box<dyn Fetch>,
    days: Vec<(i64, String)>,
    times: Vec<i64>,
}

impl std::fmt::Debug for CmcStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CmcStore")
            .field("days", &self.days.len())
            .finish()
    }
}

impl CmcStore {
    /// Asks the catalogue, through `catalogue`, which days there are; files
    /// are then read through `download`, which holds the token.
    pub fn open(catalogue: &dyn Fetch, download: Box<dyn Fetch>) -> Result<Self> {
        let days = days_listed(&catalogue.get(CATALOGUE)?)?;
        if days.is_empty() {
            return Err(ZarrError::Open(format!(
                "NASA's catalogue lists no {SHORT_NAME} day"
            )));
        }
        let times = days.iter().map(|&(day, _)| day).collect();
        Ok(Self {
            download,
            days,
            times,
        })
    }
}

/// A day's file: `analysed_sst`, packed shorts of kelvin, to Celsius on the
/// common grid, NaN over land and wherever the fill value is.
fn read_day(bytes: Vec<u8>) -> Result<Vec<f32>> {
    let bad = |err: ve_hdf5::Error| ZarrError::Layout(format!("{NAME}: {err}"));
    let file = ve_hdf5::File::from_bytes(bytes).map_err(bad)?;
    let axis = |name: &str| -> Result<Vec<f32>> {
        let values = file.dataset(name).and_then(|d| d.read_f64()).map_err(bad)?;
        Ok(values.into_iter().map(|v| v as f32).collect())
    };
    let (lat, lon) = (axis("lat")?, axis("lon")?);
    let grid = cell_grid_of(&lat, &lon)?;
    let sst = file.dataset("analysed_sst").map_err(bad)?;
    let number = |a: &str| sst.attribute(a).and_then(|a| a.number());
    let fill = number("_FillValue");
    let scale = number("scale_factor").unwrap_or(1.0);
    let offset = number("add_offset").unwrap_or(0.0);
    let kelvin = sst
        .attribute("units")
        .and_then(|a| a.text())
        .is_some_and(|u| u.eq_ignore_ascii_case("kelvin") || u == "K");
    let values = sst.read_f64().map_err(bad)?;
    if values.len() != grid.len() {
        return Err(ZarrError::Layout(format!(
            "{NAME}: analysed_sst holds {} values, not one day of {}",
            values.len(),
            grid.len()
        )));
    }
    let celsius: Vec<f32> = values
        .into_iter()
        .map(|raw| {
            if Some(raw) == fill || !raw.is_finite() {
                f32::NAN
            } else {
                let value = raw * scale + offset;
                (if kelvin { value - 273.15 } else { value }) as f32
            }
        })
        .collect();
    let (lo, hi) = (f64::from(lat[0]), f64::from(lat[lat.len() - 1]));
    Ok(regrid_within(grid, (lo.min(hi), lo.max(hi)), &celsius))
}

impl FieldSource for CmcStore {
    fn name(&self) -> &'static str {
        NAME
    }

    fn variables(&self) -> Vec<Variable> {
        vec![Variable::SeaSurfaceTemperature]
    }

    fn coverage(&self) -> Option<(Utc, Utc)> {
        Some((
            Utc::from_hours_since_unix_epoch(*self.times.first()?),
            Utc::from_hours_since_unix_epoch(*self.times.last()?),
        ))
    }

    fn step_at(&self, time: Utc) -> Option<Step> {
        step_at_hour(&self.times, time)
    }

    fn steps_in_range(&self, start: Utc, end: Utc) -> Result<Vec<Step>> {
        steps_between(&self.times, start, end, NAME)
    }

    fn read_step(&self, step: &Step) -> Result<Vec<Field>> {
        let (_, url) = self
            .days
            .get(step.index as usize)
            .ok_or_else(|| ZarrError::TimeRange(format!("{NAME} has no step {}", step.index)))?;
        let u = read_day(self.download.get(url)?)?;
        Ok(vec![Field {
            variable: Variable::SeaSurfaceTemperature,
            u,
            v: Vec::new(),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{NI, NJ};
    use std::sync::Mutex;

    fn hours(iso: &str) -> i64 {
        Utc::parse(iso).expect("a time").hours_since_unix_epoch()
    }

    /// The catalogue's answer, trimmed to what is read: newest first, each
    /// entry with its archive link, its S3 one and an unrelated page.
    fn catalogue() -> Vec<u8> {
        let entry = |date: &str| {
            let file = format!("{date}120000-CMC-L4_GHRSST-SSTfnd-CMC0.1deg-GLOB-v02.0-fv03.0");
            serde_json::json!({
                "title": file,
                "time_start": format!("{}-{}-{}T00:00:00.000Z", &date[..4], &date[4..6], &date[6..]),
                "links": [
                    { "rel": "s3#", "href": format!("s3://podaac-ops-cumulus-protected/{SHORT_NAME}/{file}.nc") },
                    { "rel": "data#", "href": format!("{ARCHIVE}podaac-ops-cumulus-protected/{SHORT_NAME}/{file}.nc") },
                    { "rel": "documentation#", "href": format!("{ARCHIVE}podaac-ops-cumulus-docs/ghrsst/open/docs/{file}.md5") },
                ],
            })
        };
        serde_json::to_vec(&serde_json::json!({
            "feed": { "entry": [entry("20261002"), entry("20261001"), entry("20260930")] }
        }))
        .expect("json")
    }

    #[test]
    fn the_catalogue_lists_each_day_at_midnight_with_its_archive_file() {
        let days = days_listed(&catalogue()).expect("parses");
        assert_eq!(days.len(), 3);
        assert_eq!(days[0].0, hours("2026-09-30T00:00"));
        assert_eq!(days[2].0, hours("2026-10-02T00:00"));
        assert!(days[1].1.starts_with(ARCHIVE), "{}", days[1].1);
        assert!(
            days[1].1.contains("/20261001120000-CMC-L4_"),
            "{}",
            days[1].1
        );
        assert!(days[1].1.ends_with(".nc"));
        assert!(days_listed(b"<html>maintenance</html>").is_err());
    }

    #[test]
    fn the_redirect_is_followed_only_to_cloudfront() {
        let signed = "https://d1abc.cloudfront.net/s3-x/podaac/CMC/f.nc?A-userid=me&Signature=s";
        assert_eq!(next_hop(signed), Hop::Signed(signed.to_owned()));
        assert_eq!(
            next_hop("https://urs.earthdata.nasa.gov/oauth/authorize?client_id=x"),
            Hop::Refused
        );
        assert_eq!(
            next_hop("https://elsewhere.invalid/cloudfront.net/f.nc"),
            Hop::Elsewhere("elsewhere.invalid".into())
        );
        assert_eq!(
            next_hop("http://d1abc.cloudfront.net/f.nc"),
            Hop::Elsewhere("an address that is not HTTPS".into())
        );
        assert_eq!(
            next_hop("https://cloudfront.net.invalid/f.nc"),
            Hop::Elsewhere("cloudfront.net.invalid".into())
        );
    }

    /// A failed request's error, as reqwest writes it, ends with the whole
    /// address — on the signed hop a credential and the person's username.
    /// What is kept is the error without it.
    #[test]
    fn a_failed_request_is_described_without_its_address() {
        let client = reqwest::blocking::Client::new();
        let err = client
            .get("http://127.0.0.1:1/f.nc?A-userid=someone&Signature=secret")
            .send()
            .expect_err("nothing listens on port 1");
        assert!(
            with_causes(&err).contains("secret"),
            "the premise: reqwest says it"
        );
        let described = describe(err);
        assert!(
            !described.contains("secret") && !described.contains("someone"),
            "{described}"
        );
    }

    #[test]
    fn the_redirect_host_is_read_as_the_client_reads_it() {
        assert_eq!(
            next_hop("https://evil.invalid#.cloudfront.net/f.nc"),
            Hop::Elsewhere("evil.invalid".into())
        );
        assert_eq!(
            next_hop("https://evil.invalid\\x.cloudfront.net/f.nc"),
            Hop::Elsewhere("evil.invalid".into())
        );
        // Upper case throughout, scheme included, as a server may write it.
        let upper = "HTTPS://D1ABC.CLOUDFRONT.NET./f.nc?Signature=s";
        assert_eq!(next_hop(upper), Hop::Signed(upper.to_owned()));
        assert_eq!(
            next_hop("not an address"),
            Hop::Elsewhere("an address that is not HTTPS".into())
        );
    }

    #[test]
    fn a_message_names_the_file_and_never_a_signature() {
        assert_eq!(
            file_name("https://d1abc.cloudfront.net/x/f.nc?Signature=secret"),
            "f.nc"
        );
        let earthdata = Earthdata::new("  secret-token  ").expect("client");
        assert_eq!(format!("{earthdata:?}"), "Earthdata");
        let err = earthdata
            .get("https://elsewhere.invalid/f.nc")
            .expect_err("not the archive")
            .to_string();
        assert!(err.contains("token goes nowhere else"), "{err}");
        assert!(!err.contains("secret"), "{err}");
    }

    /// The catalogue, and `fixtures/cmc`'s day file for every download;
    /// records what was asked for.
    struct Server {
        asked: Mutex<Vec<String>>,
    }

    impl Fetch for &'static Server {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            self.asked.lock().expect("lock").push(url.to_owned());
            if url == CATALOGUE {
                return Ok(catalogue());
            }
            let name = file_name(url);
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/cmc")
                .join(name);
            std::fs::read(&path).map_err(|_| ZarrError::Open(format!("{name}: 404")))
        }
    }

    /// The common grid's value nearest a place.
    fn at(field: &[f32], lat: f64, lon: f64) -> f32 {
        let j = ((90.0 - lat) / 0.25).round() as usize;
        let i = (lon.rem_euclid(360.0) / 0.25).round() as usize % NI as usize;
        field[j * NI as usize + i]
    }

    /// The fixture is 20 °C equatorward of 60°, −1.8 °C poleward, and land
    /// from 0° to 9° E, packed as CMC packs it: shorts of kelvin.
    #[test]
    fn a_day_reads_as_celsius_on_the_common_grid_from_the_catalogues_file() {
        let server: &'static Server = Box::leak(Box::new(Server {
            asked: Mutex::new(Vec::new()),
        }));
        let store = CmcStore::open(&server, Box::new(server)).expect("opens");
        assert_eq!(store.variables(), [Variable::SeaSurfaceTemperature]);
        let step = store
            .step_at(Utc::from_hours_since_unix_epoch(hours("2026-10-01T00:00")))
            .expect("listed");
        let field = &store.read_step(&step).expect("reads")[0];
        assert_eq!(field.u.len(), (NI * NJ) as usize);
        assert!(field.v.is_empty(), "a scalar");
        assert!((at(&field.u, -30.0, 100.0) - 20.0).abs() < 1e-3);
        assert!(
            (at(&field.u, 30.0, -100.0) - 20.0).abs() < 1e-3,
            "west of 0"
        );
        assert!((at(&field.u, 75.0, 100.0) + 1.8).abs() < 1e-3);
        assert!(at(&field.u, 0.0, 5.0).is_nan(), "land");
        let asked = server.asked.lock().expect("lock");
        assert_eq!(asked[0], CATALOGUE);
        assert!(asked[1].starts_with(ARCHIVE), "{}", asked[1]);
        assert!(asked[1].contains("/20261001120000-CMC-L4_"), "{}", asked[1]);
    }

    #[test]
    fn a_day_the_archive_cannot_give_is_an_error_naming_the_file() {
        let server: &'static Server = Box::leak(Box::new(Server {
            asked: Mutex::new(Vec::new()),
        }));
        let store = CmcStore::open(&server, Box::new(server)).expect("opens");
        let step = store
            .step_at(Utc::from_hours_since_unix_epoch(hours("2026-09-30T00:00")))
            .expect("listed");
        let err = store.read_step(&step).expect_err("404").to_string();
        assert!(err.contains("20260930120000-CMC"), "{err}");
    }
}
