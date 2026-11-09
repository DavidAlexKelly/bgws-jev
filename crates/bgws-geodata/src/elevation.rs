//! Elevation tiles in the terrarium encoding (Mapterhorn's planet archive):
//! height = R·256 + G + B/256 − 32768 metres, as WebP or PNG images.

use crate::GeoError;

/// A decoded elevation tile: `size`×`size` heights, row by row from the top.
#[derive(Clone, Debug)]
pub struct HeightTile {
    pub size: usize,
    pub h: Vec<f32>,
}

impl HeightTile {
    /// Bilinear height at fractional pixel coordinates (pixel centres at +0.5).
    pub fn sample(&self, px: f64, py: f64) -> f32 {
        let n = self.size as f64;
        let x = (px - 0.5).clamp(0.0, n - 1.0);
        let y = (py - 0.5).clamp(0.0, n - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.size - 1), (y0 + 1).min(self.size - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let at = |x: usize, y: usize| self.h[y * self.size + x];
        let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
        let bot = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
        top + (bot - top) * fy
    }
}

pub fn terrarium(r: u8, g: u8, b: u8) -> f32 {
    (r as f32 * 256.0 + g as f32 + b as f32 / 256.0) - 32_768.0
}

/// Decode a terrarium WebP tile.
pub fn decode_webp(bytes: &[u8]) -> Result<HeightTile, GeoError> {
    let mut d = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| GeoError::Format(format!("webp: {e}")))?;
    let (w, h) = d.dimensions();
    if w != h {
        return Err(GeoError::Format(format!("elevation tile {w}×{h} is not square")));
    }
    let channels = if d.has_alpha() { 4 } else { 3 };
    let mut buf = vec![0u8; (w * h) as usize * channels];
    d.read_image(&mut buf).map_err(|e| GeoError::Format(format!("webp: {e}")))?;
    let hts = buf.chunks_exact(channels).map(|p| terrarium(p[0], p[1], p[2])).collect();
    Ok(HeightTile { size: w as usize, h: hts })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrarium_heights() {
        assert_eq!(terrarium(128, 0, 0), 0.0);
        assert_eq!(terrarium(128, 100, 128), 100.5);
        assert_eq!(terrarium(127, 255, 0), -1.0);
        let t = HeightTile { size: 2, h: vec![0.0, 10.0, 20.0, 30.0] };
        assert_eq!(t.sample(1.0, 1.0), 15.0);
        assert_eq!(t.sample(0.0, 0.0), 0.0);
    }
}
