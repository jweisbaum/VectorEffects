//! Fetching tiles, and keeping them so a pan does not ask twice.
//!
//! OpenStreetMap's tile usage policy is what shapes this: a real User-Agent
//! naming the application, tiles kept on disk, and no bulk download — the
//! map asks for the tiles it is showing and nothing beyond them. A tile is
//! fetched once and then read from the disk; the store is a cache and
//! deleting it costs nothing but the fetching again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::mercator::{Decoded, SourceTile, decode};
use crate::{OsmError, Result};

/// The finest zoom the tile servers publish.
pub const MAX_SOURCE_ZOOM: u32 = 19;

/// How long a tile on disk is used before it is fetched again. The map does
/// not change often, and a week is well inside the policy's guidance.
const KEEP: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How many decoded tiles are held in memory. A viewport is a few dozen.
const HELD: usize = 192;

/// A tile source: where to fetch from, where to keep them, and what is held.
#[derive(Debug)]
pub struct Tiles {
    template: String,
    directory: PathBuf,
    agent: String,
    client: reqwest::blocking::Client,
    held: Mutex<HashMap<SourceTile, Option<Decoded>>>,
    order: Mutex<Vec<SourceTile>>,
}

impl Tiles {
    /// A source. `agent` identifies this application to the tile server, and
    /// the policy requires it to be a real one.
    pub fn new(template: &str, directory: &Path, agent: &str) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(agent.to_owned())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|err| OsmError::Fetch(err.to_string()))?;
        std::fs::create_dir_all(directory)?;
        Ok(Self {
            template: template.to_owned(),
            directory: directory.to_path_buf(),
            agent: agent.to_owned(),
            client,
            held: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
        })
    }

    /// What this identifies itself as.
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// Where a tile is kept.
    fn path_of(&self, tile: SourceTile) -> PathBuf {
        self.directory
            .join(tile.z.to_string())
            .join(tile.x.to_string())
            .join(format!("{}.png", tile.y))
    }

    /// The address a tile is fetched from.
    fn url_of(&self, tile: SourceTile) -> String {
        self.template
            .replace("{z}", &tile.z.to_string())
            .replace("{x}", &tile.x.to_string())
            .replace("{y}", &tile.y.to_string())
    }

    /// A tile's bytes from the disk, where they are there and still fresh.
    fn kept(&self, tile: SourceTile) -> Option<Vec<u8>> {
        let path = self.path_of(tile);
        let age = std::fs::metadata(&path)
            .ok()?
            .modified()
            .ok()?
            .elapsed()
            .ok()?;
        (age < KEEP).then(|| std::fs::read(&path).ok())?
    }

    /// Fetches a tile, writes it to the disk, and returns its bytes.
    fn fetch(&self, tile: SourceTile) -> Result<Vec<u8>> {
        let url = self.url_of(tile);
        let response = self
            .client
            .get(&url)
            .send()
            .map_err(|err| OsmError::Fetch(err.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(OsmError::Status {
                status: status.as_u16(),
                url,
            });
        }
        let bytes = response
            .bytes()
            .map_err(|err| OsmError::Fetch(err.to_string()))?
            .to_vec();
        let path = self.path_of(tile);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // A failed write costs the fetching again and nothing else.
        if let Err(err) = std::fs::write(&path, &bytes) {
            tracing::debug!(%err, "could not keep an OpenStreetMap tile");
        }
        Ok(bytes)
    }

    /// One tile, decoded: from memory, then the disk, then the network.
    ///
    /// A tile the server will not give is remembered as absent rather than
    /// asked for again at every frame — which is both the policy's rule and
    /// the difference between a gap in the map and a request storm.
    pub fn tile(&self, tile: SourceTile, may_fetch: bool) -> Option<Decoded> {
        if let Some(known) = self
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&tile)
        {
            return known.clone();
        }
        let bytes = match self.kept(tile) {
            Some(bytes) => Some(bytes),
            None if may_fetch => match self.fetch(tile) {
                Ok(bytes) => Some(bytes),
                Err(err) => {
                    // A tile the server will not give is worth knowing about
                    // once: a wrong address or a blocked agent is otherwise
                    // an empty map with nothing said about why.
                    tracing::warn!(%err, z = tile.z, x = tile.x, y = tile.y,
                        "could not fetch an OpenStreetMap tile");
                    None
                }
            },
            None => return None,
        };
        let decoded = bytes.and_then(|bytes| match decode(tile, &bytes) {
            Ok(decoded) => Some(decoded),
            Err(err) => {
                tracing::debug!(%err, "could not decode an OpenStreetMap tile");
                None
            }
        });
        self.hold(tile, decoded.clone());
        decoded
    }

    fn hold(&self, tile: SourceTile, decoded: Option<Decoded>) {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let mut order = self.order.lock().unwrap_or_else(PoisonError::into_inner);
        if held.insert(tile, decoded).is_none() {
            order.push(tile);
        }
        while order.len() > HELD {
            let oldest = order.remove(0);
            held.remove(&oldest);
        }
    }

    /// Drops every tile held in memory. What is on disk stays.
    pub fn release(&self) {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 PNG of one colour, encoded with no alpha channel — which is
    /// what the tile servers actually serve.
    fn two_by_two(colour: &[u8; 3]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 2, 2);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer
                .write_image_data(&colour.repeat(4))
                .expect("image data");
        }
        out
    }

    fn tiles(directory: &Path) -> Tiles {
        Tiles::new(
            "https://example.invalid/{z}/{x}/{y}.png",
            directory,
            "VectorEffects/test",
        )
        .expect("a source")
    }

    #[test]
    fn a_tiles_address_and_its_place_on_disk_follow_its_numbers() {
        let root = tempfile::tempdir().expect("temp");
        let tiles = tiles(root.path());
        let tile = SourceTile { z: 4, x: 5, y: 6 };
        assert_eq!(tiles.url_of(tile), "https://example.invalid/4/5/6.png");
        assert_eq!(
            tiles.path_of(tile),
            root.path().join("4").join("5").join("6.png")
        );
    }

    /// Nothing is fetched unless the caller says it may be: the map asks
    /// without fetching while it is only deciding what it has.
    #[test]
    fn a_tile_that_is_not_kept_is_not_fetched_unless_asked_for() {
        let root = tempfile::tempdir().expect("temp");
        let tiles = tiles(root.path());
        assert!(tiles.tile(SourceTile { z: 1, x: 0, y: 0 }, false).is_none());
        // Nothing was written, and nothing reached the network.
        assert!(
            std::fs::read_dir(root.path())
                .expect("dir")
                .next()
                .is_none()
        );
    }

    /// A tile on disk is used without asking the server, which is the whole
    /// of the usage policy's caching rule.
    #[test]
    fn a_kept_tile_is_read_from_the_disk() {
        let root = tempfile::tempdir().expect("temp");
        let tiles = tiles(root.path());
        let tile = SourceTile { z: 2, x: 1, y: 1 };
        let path = tiles.path_of(tile);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(&path, two_by_two(&[9, 99, 199])).expect("write");
        let decoded = tiles.tile(tile, false).expect("read from disk");
        assert_eq!(decoded.size, 2);
        assert_eq!(
            &decoded.rgba[..4],
            &[9, 99, 199, 255],
            "a tile with no alpha channel decodes as opaque"
        );
        // And held: a second ask does not touch the disk again.
        std::fs::remove_file(&path).expect("remove");
        assert!(tiles.tile(tile, false).is_some(), "held in memory");
        tiles.release();
        assert!(tiles.tile(tile, false).is_none(), "released");
    }
}
