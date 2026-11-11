//! The geoid: how far mean sea level is above the WGS-84 ellipsoid.
//!
//! The sim's heights are above mean sea level (the terrain is built from
//! elevation data that is). DIS and HLA give positions as geocentric
//! coordinates on the WGS-84 ellipsoid. At the boundary, height above the
//! ellipsoid h = height above mean sea level H + the geoid undulation N,
//! about +30 m over the Baltic and from −107 m to +85 m worldwide. Without
//! this, every entity would be published that far above or below the ground.
//!
//! The model is EGM96 on a 15-minute grid (`data/geoid/`), bilinearly
//! interpolated: 0.04 m RMS, 1.15 m at worst, by the grid's own header.

use std::sync::OnceLock;

/// The EGM96 15-minute grid, in GeographicLib's PGM format.
static EGM96_15: &[u8] = include_bytes!("../../../data/geoid/egm96-15.pgm");

/// A geoid height grid: rows from 90° N southwards, columns from 0° E
/// eastwards, at an equal spacing in degrees.
#[derive(Debug)]
pub struct Geoid {
    width: usize,
    height: usize,
    offset: f64,
    scale: f64,
    samples: Vec<u16>,
}

/// Why a geoid file could not be read.
#[derive(Debug, PartialEq, Eq)]
pub struct GeoidError(pub String);

impl Geoid {
    /// EGM96, parsed once.
    pub fn egm96() -> &'static Geoid {
        static G: OnceLock<Geoid> = OnceLock::new();
        G.get_or_init(|| Geoid::from_pgm(EGM96_15).expect("the vendored EGM96 grid is valid"))
    }

    /// Read GeographicLib's PGM geoid format: `P5`, comment lines carrying
    /// `# Offset` and `# Scale`, then width, height and 65535, then the
    /// samples as 16-bit big-endian values.
    pub fn from_pgm(bytes: &[u8]) -> Result<Geoid, GeoidError> {
        let err = |m: &str| GeoidError(m.to_string());
        let mut pos = 0;
        let mut line = || -> Option<String> {
            let rest = bytes.get(pos..)?;
            let end = rest.iter().position(|&b| b == b'\n')?;
            pos += end + 1;
            Some(String::from_utf8_lossy(&rest[..end]).trim().to_string())
        };
        if line().as_deref() != Some("P5") {
            return Err(err("not a binary PGM (P5)"));
        }
        let (mut offset, mut scale) = (None, None);
        let dims = loop {
            let l = line().ok_or_else(|| err("header ends early"))?;
            if let Some(c) = l.strip_prefix('#') {
                let mut it = c.split_whitespace();
                match (it.next(), it.next().and_then(|v| v.parse::<f64>().ok())) {
                    (Some("Offset"), Some(v)) => offset = Some(v),
                    (Some("Scale"), Some(v)) => scale = Some(v),
                    _ => {}
                }
                continue;
            }
            break l;
        };
        let mut wh = dims.split_whitespace().map(|v| v.parse::<usize>());
        let (Some(Ok(width)), Some(Ok(height))) = (wh.next(), wh.next()) else { return Err(err("no width and height")) };
        if line().as_deref() != Some("65535") {
            return Err(err("samples are not 16-bit"));
        }
        let data = &bytes[pos..];
        if data.len() != width * height * 2 {
            return Err(err("sample count does not match width × height"));
        }
        let samples = data.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        Ok(Geoid { width, height, offset: offset.ok_or_else(|| err("no Offset"))?, scale: scale.ok_or_else(|| err("no Scale"))?, samples })
    }

    fn at(&self, row: usize, col: usize) -> f64 {
        self.offset + self.scale * self.samples[row * self.width + col % self.width] as f64
    }

    /// The geoid undulation N at a latitude and longitude (degrees), m:
    /// height above the ellipsoid = height above mean sea level + N.
    pub fn undulation(&self, lat_deg: f64, lon_deg: f64) -> f64 {
        let step = 180.0 / (self.height - 1) as f64;
        let y = ((90.0 - lat_deg.clamp(-90.0, 90.0)) / step).clamp(0.0, (self.height - 1) as f64);
        let x = lon_deg.rem_euclid(360.0) / step;
        let (r0, c0) = (y.floor() as usize, x.floor() as usize);
        let r1 = (r0 + 1).min(self.height - 1);
        let (fy, fx) = (y - r0 as f64, x - c0 as f64);
        let top = self.at(r0, c0) * (1.0 - fx) + self.at(r0, c0 + 1) * fx;
        let bottom = self.at(r1, c0) * (1.0 - fx) + self.at(r1, c0 + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// Height above the ellipsoid from height above mean sea level.
    pub fn to_ellipsoid(&self, lat_deg: f64, lon_deg: f64, msl_m: f64) -> f64 {
        msl_m + self.undulation(lat_deg, lon_deg)
    }

    /// Height above mean sea level from height above the ellipsoid.
    pub fn to_msl(&self, lat_deg: f64, lon_deg: f64, ellipsoid_m: f64) -> f64 {
        ellipsoid_m - self.undulation(lat_deg, lon_deg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NGA's EGM96 check points (the test values published with the
    /// model's interpolation program): latitude, longitude, N in metres.
    const NGA: [(f64, f64, f64); 6] = [
        (38.6281550, 269.7791550, -31.628),
        (-14.6212170, 305.0211140, -2.969),
        (46.8743190, 102.4487290, -43.575),
        (-23.6174460, 133.8747120, 15.871),
        (38.6254730, 359.9995000, 50.066),
        (-0.4667440, 0.0023000, 17.329),
    ];

    #[test]
    fn egm96_matches_ngas_check_points() {
        let g = Geoid::egm96();
        for (lat, lon, n) in NGA {
            let got = g.undulation(lat, lon);
            // Bilinear on the 15' grid: within 6 cm at these points.
            assert!((got - n).abs() < 0.1, "N at {lat}, {lon}: {got:.3} m, NGA {n} m");
        }
    }

    #[test]
    fn the_conversion_round_trips_and_the_baltic_is_about_30_m() {
        let g = Geoid::egm96();
        let (lat, lon) = (54.2, 18.6);
        let n = g.undulation(lat, lon);
        assert!((25.0..40.0).contains(&n), "N at the theatre origin {n:.1} m");
        let h = g.to_ellipsoid(lat, lon, 112.0);
        assert!((g.to_msl(lat, lon, h) - 112.0).abs() < 1e-9);
        // Longitude wraps; the poles clamp.
        assert!((g.undulation(lat, lon + 360.0) - n).abs() < 1e-9);
        assert!(g.undulation(90.0, 0.0).is_finite() && g.undulation(-90.0, 0.0).is_finite());
    }

    #[test]
    fn a_bad_file_is_refused() {
        assert!(Geoid::from_pgm(b"P2\n").is_err());
        assert!(Geoid::from_pgm(b"P5\n# Offset 0\n# Scale 1\n2 2\n65535\n\x00").is_err());
    }
}
