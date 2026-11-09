//! Import a GeoTIFF elevation model (§14.2): a lat/lon grid such as
//! Copernicus GLO-30, georeferenced by the ModelTiepoint and ModelPixelScale
//! tags, resampled bilinearly onto a theatre's ENU grid. This is the offline
//! tool's job (the sim never reads files); `bgws pack --dem` uses it.

use anyhow::{anyhow, bail, Context, Result};
use bgws_env::{Raster, Terrain};
use std::path::Path;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;


/// A decoded DEM on a lat/lon grid.
#[derive(Clone, Debug)]
pub struct Dem {
    pub width: usize,
    pub height: usize,
    /// Longitude and latitude of the corner of pixel (0, 0), degrees.
    pub lon0: f64,
    pub lat0: f64,
    /// Degrees per pixel, east and south.
    pub dlon: f64,
    pub dlat: f64,
    pub heights: Vec<f32>,
    pub nodata: Option<f32>,
}

impl Dem {
    pub fn read(path: &Path) -> Result<Dem> {
        let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut dec = Decoder::new(std::io::BufReader::new(file))?;
        let (w, h) = dec.dimensions()?;
        let scale = dec.get_tag_f64_vec(Tag::ModelPixelScaleTag).map_err(|e| anyhow!("no ModelPixelScale tag ({e}): not a GeoTIFF"))?;
        let tie = dec.get_tag_f64_vec(Tag::ModelTiepointTag).map_err(|_| anyhow!("no ModelTiepoint tag: not a GeoTIFF"))?;
        if scale.len() < 2 || tie.len() < 6 {
            bail!("malformed georeferencing tags");
        }
        let nodata = dec.get_tag_ascii_string(Tag::GdalNodata).ok().and_then(|s| s.trim_matches(char::from(0)).trim().parse::<f32>().ok());
        let heights: Vec<f32> = match dec.read_image()? {
            DecodingResult::F32(v) => v,
            DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::I16(v) => v.into_iter().map(f32::from).collect(),
            DecodingResult::U16(v) => v.into_iter().map(f32::from).collect(),
            DecodingResult::I32(v) => v.into_iter().map(|x| x as f32).collect(),
            _ => bail!("unsupported sample format for a DEM"),
        };
        if heights.len() != (w * h) as usize {
            bail!("only single-band DEMs are supported");
        }
        // Tie point (I, J, K) → (X, Y, Z): the model position of pixel (I, J).
        let (i, j, x, y) = (tie[0], tie[1], tie[3], tie[4]);
        Ok(Dem { width: w as usize, height: h as usize, lon0: x - i * scale[0], lat0: y + j * scale[1], dlon: scale[0], dlat: scale[1], heights, nodata })
    }

    /// Bilinear height at a lat/lon (pixel-is-area: centres at +0.5), or None outside.
    pub fn height(&self, lat: f64, lon: f64) -> Option<f64> {
        let fx = (lon - self.lon0) / self.dlon - 0.5;
        let fy = (self.lat0 - lat) / self.dlat - 0.5;
        if fx < 0.0 || fy < 0.0 || fx > (self.width - 1) as f64 || fy > (self.height - 1) as f64 {
            return None;
        }
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let at = |x: usize, y: usize| {
            let v = self.heights[y * self.width + x];
            if Some(v) == self.nodata || !v.is_finite() {
                0.0
            } else {
                v as f64
            }
        };
        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        Some(top * (1.0 - ty) + bottom * ty)
    }
}

/// Replace a terrain's elevation with the DEM's, sampled at every node of its
/// grid; nodes outside the DEM keep their height. Returns how many were set.
pub fn apply_dem(t: &mut Terrain, dem: &Dem) -> usize {
    let frame = t.frame();
    let spec = t.spec;
    let mut set = 0;
    let elevation = Raster::from_fn(spec, |i, j| {
        let (e, n) = spec.pos(i, j);
        let g = frame.enu_to_geodetic(bgws_core::Enu::new(e, n, 0.0));
        match dem.height(g.lat, g.lon) {
            Some(h) => {
                set += 1;
                h as f32
            }
            None => t.elevation.get(i, j),
        }
    });
    t.elevation = elevation;
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiff::encoder::{colortype, TiffEncoder};

    #[test]
    fn reads_a_geotiff_and_samples_it() {
        let dir = std::env::temp_dir().join(format!("bgws-dem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dem.tif");
        // 4 × 3 pixels of 0.01°, a ramp rising 10 m per pixel eastward.
        let (w, h) = (4u32, 3u32);
        let data: Vec<f32> = (0..h).flat_map(|_| (0..w).map(|x| 100.0 + 10.0 * x as f32)).collect();
        {
            let mut enc = TiffEncoder::new(std::fs::File::create(&path).unwrap()).unwrap();
            let mut img = enc.new_image::<colortype::Gray32Float>(w, h).unwrap();
            img.encoder().write_tag(Tag::ModelPixelScaleTag, &[0.01f64, 0.01, 0.0][..]).unwrap();
            img.encoder().write_tag(Tag::ModelTiepointTag, &[0.0f64, 0.0, 0.0, 18.0, 54.03, 0.0][..]).unwrap();
            img.write_data(&data).unwrap();
        }
        let dem = Dem::read(&path).unwrap();
        assert_eq!((dem.width, dem.height), (4, 3));
        // Centre of pixel (1, 1).
        let hgt = dem.height(54.03 - 0.015, 18.015).unwrap();
        assert!((hgt - 110.0).abs() < 1e-6, "{hgt}");
        // Halfway between pixel centres 1 and 2.
        assert!((dem.height(54.015, 18.02).unwrap() - 115.0).abs() < 1e-6);
        assert!(dem.height(60.0, 18.0).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
