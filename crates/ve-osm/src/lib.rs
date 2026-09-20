//! OpenStreetMap raster tiles as a map background (spec.md 5.4).
//!
//! **This crate fetches over the network, and it is the only part of the
//! application that fetches tiles.** Invariant 5 forbids that everywhere
//! else, and it still does: the exception is this crate, named in
//! `tools/check-offline.sh`, entered only while the user has *OpenStreetMap*
//! switched on in the view controls, and never touched by an export.
//!
//! What is fetched is standard `z/x/y` PNG from OpenStreetMap's own servers,
//! under their tile usage policy: a real User-Agent, tiles kept on disk so a
//! pan does not re-fetch what it just had, and no bulk download — the map
//! asks for the tiles it is showing, one viewport at a time. Attribution is
//! the map's (`OpenStreetMap contributors`), shown while the layer is on.
//!
//! The tiles are Web Mercator and the application's are a lat/lon grid, so
//! they are resampled here rather than in the webview: the map then draws
//! them as ordinary textures, in every projection, through the paths that
//! already exist.

mod cache;
mod mercator;

pub use cache::{MAX_SOURCE_ZOOM, Tiles};
pub use mercator::{SourceTile, source_tiles, source_zoom_for, warp};

/// Where tiles come from. A `{z}/{x}/{y}.png` template.
pub const DEFAULT_TILE_URL: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

/// What the map must show while OpenStreetMap tiles are drawn.
pub const ATTRIBUTION: &str = "© OpenStreetMap contributors";

/// How an application identifies itself to the tile servers.
///
/// The usage policy requires a real one, naming the application and a way to
/// reach whoever runs it. It lives here rather than in the application
/// because the host it names is part of this crate's one exception to
/// invariant 5, and `tools/check-offline.sh` allows it here and nowhere else.
pub fn user_agent(version: &str) -> String {
    format!("VectorEffects/{version} (+https://github.com/jweisbaum/VectorEffects)")
}

/// What went wrong fetching or decoding a tile.
#[derive(Debug, thiserror::Error)]
pub enum OsmError {
    /// The tile could not be fetched.
    #[error("could not reach the tile server: {0}")]
    Fetch(String),
    /// The server answered, but not with a tile.
    #[error("the tile server answered {status} for {url}")]
    Status {
        /// The HTTP status.
        status: u16,
        /// What was asked for.
        url: String,
    },
    /// The bytes are not a PNG this decoder reads.
    #[error("tile is not a readable PNG: {0}")]
    Decode(String),
    /// The tile cache could not be read or written.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// This crate's result type.
pub type Result<T> = std::result::Result<T, OsmError>;
