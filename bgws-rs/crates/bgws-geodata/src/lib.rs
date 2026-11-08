//! `bgws-geodata`: real ground for theatre packs, from planet PMTiles read
//! by HTTP range (plan §14.2). Protomaps' OpenStreetMap build gives land
//! cover, water, roads and the viewer's basemap; Mapterhorn's terrarium
//! build gives elevation. A build fetches only the tiles its box needs and
//! keeps them in a disk cache.

pub mod build;
pub mod elevation;
pub mod fill;
pub mod mercator;
pub mod mvt;
pub mod pack;
pub mod pmtiles;
pub mod preview;
pub mod roads;
pub mod source;

#[derive(Debug, thiserror::Error)]
pub enum GeoError {
    #[error("io: {0}")]
    Io(String),
    #[error("http: {0}")]
    Http(String),
    #[error("format: {0}")]
    Format(String),
}

impl From<std::io::Error> for GeoError {
    fn from(e: std::io::Error) -> Self {
        GeoError::Io(e.to_string())
    }
}
