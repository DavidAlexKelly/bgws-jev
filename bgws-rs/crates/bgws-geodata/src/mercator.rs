//! Web Mercator tiles: which tiles cover a box, and where a point falls in one.

/// A box in degrees: west, south, east, north.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BBox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl BBox {
    pub fn new(west: f64, south: f64, east: f64, north: f64) -> Self {
        Self { west, south, east, north }
    }
    /// "west,south,east,north".
    pub fn parse(s: &str) -> Result<Self, String> {
        let v: Vec<f64> = s.split(',').map(|x| x.trim().parse::<f64>()).collect::<Result<_, _>>().map_err(|e| format!("bbox `{s}`: {e}"))?;
        let [w, so, e, n] = v[..] else { return Err(format!("bbox `{s}`: want west,south,east,north")) };
        let b = Self::new(w, so, e, n);
        if !(w < e && so < n && (-180.0..=180.0).contains(&w) && (-180.0..=180.0).contains(&e) && (-85.0..=85.0).contains(&so) && (-85.0..=85.0).contains(&n)) {
            return Err(format!("bbox `{s}`: west < east and south < north, in degrees"));
        }
        Ok(b)
    }
    pub fn centre(&self) -> (f64, f64) {
        ((self.west + self.east) / 2.0, (self.south + self.north) / 2.0)
    }
    /// The box grown by `m` metres on every side.
    pub fn grown(&self, m: f64) -> Self {
        let dlat = m / 111_320.0;
        let dlon = m / (111_320.0 * self.centre().1.to_radians().cos().max(0.01));
        Self::new(self.west - dlon, self.south - dlat, self.east + dlon, self.north + dlat)
    }
}

/// Fractional tile coordinates of a point at zoom `z` (y down).
pub fn tile_xy(lon: f64, lat: f64, z: u8) -> (f64, f64) {
    let n = (1u64 << z) as f64;
    let x = (lon + 180.0) / 360.0 * n;
    let lr = lat.to_radians();
    let y = (1.0 - (lr.tan() + 1.0 / lr.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// The point at fractional tile coordinates.
pub fn tile_lonlat(x: f64, y: f64, z: u8) -> (f64, f64) {
    let n = (1u64 << z) as f64;
    let lon = x / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh().atan().to_degrees();
    (lon, lat)
}

/// Every tile at zoom `z` touching the box, row by row.
pub fn tiles_covering(b: &BBox, z: u8) -> Vec<(u32, u32)> {
    let max = (1u32 << z) - 1;
    let (x0, y0) = tile_xy(b.west, b.north, z);
    let (x1, y1) = tile_xy(b.east, b.south, z);
    let (x0, x1) = ((x0.floor() as i64).clamp(0, max as i64) as u32, (x1.floor() as i64).clamp(0, max as i64) as u32);
    let (y0, y1) = ((y0.floor() as i64).clamp(0, max as i64) as u32, (y1.floor() as i64).clamp(0, max as i64) as u32);
    (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).collect()
}

/// Ground metres per tile at a latitude.
pub fn tile_size_m(lat: f64, z: u8) -> f64 {
    40_075_016.686 * lat.to_radians().cos() / (1u64 << z) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn round_trips_and_covers() {
        let (x, y) = tile_xy(20.5, 54.7, 12);
        let (lon, lat) = tile_lonlat(x, y, 12);
        assert_relative_eq!(lon, 20.5, epsilon = 1e-9);
        assert_relative_eq!(lat, 54.7, epsilon = 1e-9);
        assert_eq!(tiles_covering(&BBox::new(-1.0, -1.0, 1.0, 1.0), 1), vec![(0, 0), (1, 0), (0, 1), (1, 1)]);
        let b = BBox::parse("20.3,54.5,21.5,55.1").unwrap();
        assert!(BBox::parse("21,54,20,55").is_err());
        assert!(tile_size_m(54.7, 14) > 1_300.0 && tile_size_m(54.7, 14) < 1_500.0);
        let g = b.grown(1000.0);
        assert!(g.west < b.west && g.north > b.north);
    }
}
