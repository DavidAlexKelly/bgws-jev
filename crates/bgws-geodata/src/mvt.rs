//! Mapbox Vector Tiles (https://github.com/mapbox/vector-tile-spec, v2):
//! just enough protobuf to read layers, features, their properties and their
//! geometry in tile coordinates (0..extent, y down).

use crate::GeoError;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    String(String),
    Float(f64),
    Int(i64),
    Bool(bool),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    /// The protobuf encodings of a boolean or a "yes" all count.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::String(s) => !matches!(s.as_str(), "" | "no" | "false" | "0"),
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            Value::String(s) => s.parse().ok(),
            Value::Bool(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeomType {
    Point,
    Line,
    Polygon,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Feature {
    pub id: Option<u64>,
    pub kind: GeomType,
    pub props: BTreeMap<String, Value>,
    /// Points: one part per point. Lines: one part per line. Polygons: one
    /// part per ring (exteriors and holes alike, in tile order).
    pub parts: Vec<Vec<[f64; 2]>>,
}

impl Feature {
    pub fn str(&self, k: &str) -> Option<&str> {
        self.props.get(k).and_then(Value::as_str)
    }
    pub fn flag(&self, k: &str) -> bool {
        self.props.get(k).map(Value::truthy).unwrap_or(false)
    }
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    pub extent: u32,
    pub features: Vec<Feature>,
}

struct Pb<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Pb<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn done(&self) -> bool {
        self.p >= self.b.len()
    }
    fn varint(&mut self) -> Result<u64, GeoError> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let byte = *self.b.get(self.p).ok_or_else(|| GeoError::Format("vector tile ends inside a varint".into()))?;
            self.p += 1;
            v |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Ok(v);
            }
            shift += 7;
            if shift > 63 {
                return Err(GeoError::Format("varint too long".into()));
            }
        }
    }
    fn key(&mut self) -> Result<(u32, u8), GeoError> {
        let k = self.varint()?;
        Ok(((k >> 3) as u32, (k & 7) as u8))
    }
    fn bytes(&mut self) -> Result<&'a [u8], GeoError> {
        let n = self.varint()? as usize;
        let s = self.b.get(self.p..self.p + n).ok_or_else(|| GeoError::Format("vector tile field runs past its end".into()))?;
        self.p += n;
        Ok(s)
    }
    fn fixed(&mut self, n: usize) -> Result<&'a [u8], GeoError> {
        let s = self.b.get(self.p..self.p + n).ok_or_else(|| GeoError::Format("vector tile ends inside a fixed field".into()))?;
        self.p += n;
        Ok(s)
    }
    fn skip(&mut self, wire: u8) -> Result<(), GeoError> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => {
                self.fixed(8)?;
            }
            2 => {
                self.bytes()?;
            }
            5 => {
                self.fixed(4)?;
            }
            w => return Err(GeoError::Format(format!("protobuf wire type {w}"))),
        }
        Ok(())
    }
    fn packed(&mut self) -> Result<Vec<u32>, GeoError> {
        let mut inner = Pb::new(self.bytes()?);
        let mut v = Vec::new();
        while !inner.done() {
            v.push(inner.varint()? as u32);
        }
        Ok(v)
    }
}

fn zigzag(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

fn value(b: &[u8]) -> Result<Value, GeoError> {
    let mut pb = Pb::new(b);
    let mut out = Value::Bool(false);
    while !pb.done() {
        let (f, w) = pb.key()?;
        out = match (f, w) {
            (1, 2) => Value::String(String::from_utf8_lossy(pb.bytes()?).into_owned()),
            (2, 5) => Value::Float(f32::from_le_bytes(pb.fixed(4)?.try_into().expect("4")) as f64),
            (3, 1) => Value::Float(f64::from_le_bytes(pb.fixed(8)?.try_into().expect("8"))),
            (4, 0) => Value::Int(pb.varint()? as i64),
            (5, 0) => Value::Int(pb.varint()? as i64),
            (6, 0) => {
                let v = pb.varint()?;
                Value::Int(((v >> 1) as i64) ^ -((v & 1) as i64))
            }
            (7, 0) => Value::Bool(pb.varint()? != 0),
            (_, w) => {
                pb.skip(w)?;
                continue;
            }
        };
    }
    Ok(out)
}

fn geometry(kind: GeomType, cmds: &[u32]) -> Vec<Vec<[f64; 2]>> {
    let mut parts: Vec<Vec<[f64; 2]>> = Vec::new();
    let (mut x, mut y) = (0i32, 0i32);
    let mut i = 0;
    while i < cmds.len() {
        let (id, count) = (cmds[i] & 7, cmds[i] >> 3);
        i += 1;
        match id {
            1 | 2 => {
                for _ in 0..count {
                    let (Some(&dx), Some(&dy)) = (cmds.get(i), cmds.get(i + 1)) else { return parts };
                    i += 2;
                    x += zigzag(dx);
                    y += zigzag(dy);
                    if id == 1 || parts.is_empty() || kind == GeomType::Point {
                        parts.push(Vec::new());
                    }
                    parts.last_mut().expect("a part").push([x as f64, y as f64]);
                }
            }
            7 => {
                // ClosePath: repeat the ring's first point.
                if let Some(r) = parts.last_mut() {
                    if let Some(&p0) = r.first() {
                        r.push(p0);
                    }
                }
            }
            _ => return parts,
        }
    }
    parts
}

/// Decode a tile (already decompressed).
pub fn decode(b: &[u8]) -> Result<Vec<Layer>, GeoError> {
    let mut pb = Pb::new(b);
    let mut layers = Vec::new();
    while !pb.done() {
        let (f, w) = pb.key()?;
        if f != 3 || w != 2 {
            pb.skip(w)?;
            continue;
        }
        let mut lp = Pb::new(pb.bytes()?);
        let (mut name, mut extent) = (String::new(), 4096u32);
        let (mut keys, mut values, mut raw_features) = (Vec::new(), Vec::new(), Vec::new());
        while !lp.done() {
            let (f, w) = lp.key()?;
            match (f, w) {
                (1, 2) => name = String::from_utf8_lossy(lp.bytes()?).into_owned(),
                (2, 2) => raw_features.push(lp.bytes()?),
                (3, 2) => keys.push(String::from_utf8_lossy(lp.bytes()?).into_owned()),
                (4, 2) => values.push(value(lp.bytes()?)?),
                (5, 0) => extent = lp.varint()? as u32,
                (_, w) => lp.skip(w)?,
            }
        }
        let mut features = Vec::with_capacity(raw_features.len());
        for fb in raw_features {
            let mut fp = Pb::new(fb);
            let (mut id, mut kind, mut tags, mut geom) = (None, GeomType::Unknown, Vec::new(), Vec::new());
            while !fp.done() {
                let (f, w) = fp.key()?;
                match (f, w) {
                    (1, 0) => id = Some(fp.varint()?),
                    (2, 2) => tags = fp.packed()?,
                    (3, 0) => {
                        kind = match fp.varint()? {
                            1 => GeomType::Point,
                            2 => GeomType::Line,
                            3 => GeomType::Polygon,
                            _ => GeomType::Unknown,
                        }
                    }
                    (4, 2) => geom = fp.packed()?,
                    (_, w) => fp.skip(w)?,
                }
            }
            let mut props = BTreeMap::new();
            for kv in tags.as_chunks::<2>().0 {
                if let (Some(k), Some(v)) = (keys.get(kv[0] as usize), values.get(kv[1] as usize)) {
                    props.insert(k.clone(), v.clone());
                }
            }
            features.push(Feature { id, kind, props, parts: geometry(kind, &geom) });
        }
        layers.push(Layer { name, extent, features });
    }
    Ok(layers)
}

// ── Estimated building heights, for the viewer's 3D buildings ──────────────
//
// Most buildings in OpenStreetMap have no height, so a viewer would draw them
// all at one guessed height. A pack's basemap gives each building without
// one an `est_height` (m) from what its footprint says: its area, and how
// long and narrow it is. A row of garages is long and thin and low; a long
// block of flats 12–14 m deep is a panel block of five or nine storeys; a
// big compact footprint is a hall or a shop, low for its size; a small one
// is a house. It is a guess for the picture only: the simulation reads its
// buildings from the planet archive, not from the basemap.
//
// The tile is patched, not re-encoded: every field is copied byte for byte,
// each estimated feature gets a second packed `tags` field (repeated packed
// fields concatenate), and the layer gains one key and its values. Geometry
// is never touched.

/// A footprint's height estimate, m, from its area (m²), perimeter (m) and
/// whether it is a building part (parts keep what they are given).
pub fn estimate_height(area_m2: f64, perimeter_m: f64) -> f64 {
    if area_m2 <= 0.0 || perimeter_m <= 0.0 {
        return 6.0;
    }
    // Roughly its width for a long building, half its side for a square one.
    let thickness = 2.0 * area_m2 / perimeter_m;
    // 1.27 for a square, 1.7 for 1:3, 2.3 for 1:5.
    let elongation = perimeter_m * perimeter_m / (4.0 * std::f64::consts::PI * area_m2);
    let long = elongation > 1.9;
    match area_m2 {
        a if a < 30.0 => 3.0,
        _ if long && thickness < 7.0 => 3.5, // garage rows, sheds, barns
        a if a < 80.0 => 4.5,
        a if a < 200.0 => 7.0,
        a if a < 500.0 => {
            if long {
                12.0
            } else {
                9.0
            }
        }
        a if a < 2000.0 => {
            if long {
                15.0
            } else {
                11.0
            }
        }
        _ => {
            if long {
                27.0
            } else {
                12.0
            }
        }
    }
}

/// Area (m², net of holes) and perimeter (m) of a polygon feature in a tile.
fn footprint(parts: &[Vec<[f64; 2]>], unit_m: f64) -> (f64, f64) {
    let (mut area, mut perim) = (0.0, 0.0);
    for ring in parts {
        let n = ring.len();
        if n < 3 {
            continue;
        }
        let mut a = 0.0;
        for i in 0..n {
            let (p, q) = (ring[i], ring[(i + 1) % n]);
            a += p[0] * q[1] - q[0] * p[1];
            perim += ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
        }
        // Exterior rings are positive in tile space (y down), holes negative.
        area += a / 2.0;
    }
    (area.abs() * unit_m * unit_m, perim * unit_m)
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_field(out: &mut Vec<u8>, field: u32, payload: &[u8]) {
    put_varint(out, ((field as u64) << 3) | 2);
    put_varint(out, payload.len() as u64);
    out.extend_from_slice(payload);
}

/// The tile (uncompressed MVT at z/x/y) with `est_height` on every building
/// in its `buildings` layer that has no `height`. Other layers, and any tile
/// without such a layer, come back unchanged.
pub fn with_estimated_heights(b: &[u8], z: u8, _x: u32, y: u32) -> Result<Vec<u8>, GeoError> {
    let layers = decode(b)?;
    let Some(li) = layers.iter().position(|l| l.name == "buildings") else { return Ok(b.to_vec()) };
    let layer = &layers[li];
    // Metres per tile unit at the tile's middle.
    let n = (1u64 << z) as f64;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * (y as f64 + 0.5) / n)).sinh().atan();
    let unit_m = 40_075_016.686 * lat.cos() / n / layer.extent.max(1) as f64;
    let est: Vec<Option<f64>> = layer
        .features
        .iter()
        .map(|f| {
            if f.kind != GeomType::Polygon || f.props.contains_key("height") {
                return None;
            }
            let (a, p) = footprint(&f.parts, unit_m);
            Some(estimate_height(a, p))
        })
        .collect();
    if est.iter().all(Option::is_none) {
        return Ok(b.to_vec());
    }
    // Distinct heights become the layer's new values, after its own.
    let mut heights: Vec<f64> = est.iter().flatten().copied().collect();
    heights.sort_by(|a, b| a.total_cmp(b));
    heights.dedup();

    let mut out = Vec::with_capacity(b.len() + 4096);
    let mut pb = Pb::new(b);
    let mut index = 0usize;
    while !pb.done() {
        let start = pb.p;
        let (f, w) = pb.key()?;
        if f != 3 || w != 2 {
            pb.skip(w)?;
            out.extend_from_slice(&b[start..pb.p]);
            continue;
        }
        let payload = pb.bytes()?;
        let this = index;
        index += 1;
        if this != li {
            out.extend_from_slice(&b[start..pb.p]);
            continue;
        }
        // The buildings layer: copy each field, adding tags to estimated features.
        let mut lp = Pb::new(payload);
        let (mut keys, mut values, mut feature) = (0u32, 0u32, 0usize);
        let mut body = Vec::with_capacity(payload.len() + 2048);
        while !lp.done() {
            let fs = lp.p;
            let (lf, lw) = lp.key()?;
            match (lf, lw) {
                (2, 2) => {
                    let fb = lp.bytes()?;
                    let mut fbody = fb.to_vec();
                    if let Some(Some(h)) = est.get(feature) {
                        let vi = heights.iter().position(|x| x == h).unwrap_or(0);
                        let mut tags = Vec::new();
                        // The key and value indices are known only after the
                        // whole layer is read: written as markers, fixed below.
                        put_varint(&mut tags, 0x3fff_ffff);
                        put_varint(&mut tags, vi as u64);
                        put_field(&mut fbody, 2, &tags);
                    }
                    feature += 1;
                    put_field(&mut body, 2, &fbody);
                }
                (3, 2) => {
                    lp.bytes()?;
                    keys += 1;
                    body.extend_from_slice(&payload[fs..lp.p]);
                }
                (4, 2) => {
                    lp.bytes()?;
                    values += 1;
                    body.extend_from_slice(&payload[fs..lp.p]);
                }
                _ => {
                    lp.skip(lw)?;
                    body.extend_from_slice(&payload[fs..lp.p]);
                }
            }
        }
        // Now the indices: the new key after the layer's keys, the new values after its values.
        let body = fix_markers(&body, keys, values)?;
        let mut body = body;
        put_field(&mut body, 3, b"est_height");
        for h in &heights {
            let mut v = Vec::with_capacity(9);
            v.push((3 << 3) | 1); // Value.double_value, 64-bit
            v.extend_from_slice(&h.to_le_bytes());
            put_field(&mut body, 4, &v);
        }
        put_field(&mut out, 3, &body);
    }
    Ok(out)
}

/// Rewrite the marker tags in a buildings layer's features: the key index
/// (written as 0x3fffffff) becomes `keys`, and each value index gains `values`.
fn fix_markers(body: &[u8], keys: u32, values: u32) -> Result<Vec<u8>, GeoError> {
    let mut out = Vec::with_capacity(body.len() + 1024);
    let mut lp = Pb::new(body);
    while !lp.done() {
        let fs = lp.p;
        let (lf, lw) = lp.key()?;
        if (lf, lw) != (2, 2) {
            lp.skip(lw)?;
            out.extend_from_slice(&body[fs..lp.p]);
            continue;
        }
        let fb = lp.bytes()?;
        let mut fp = Pb::new(fb);
        let mut fbody = Vec::with_capacity(fb.len() + 8);
        while !fp.done() {
            let s = fp.p;
            let (f, w) = fp.key()?;
            if (f, w) == (2, 2) {
                let tags = fp.packed()?;
                if tags.len() == 2 && tags[0] == 0x3fff_ffff {
                    let mut t = Vec::new();
                    put_varint(&mut t, keys as u64);
                    put_varint(&mut t, (values + tags[1]) as u64);
                    put_field(&mut fbody, 2, &t);
                    continue;
                }
                fbody.extend_from_slice(&fb[s..fp.p]);
                continue;
            }
            fp.skip(w)?;
            fbody.extend_from_slice(&fb[s..fp.p]);
        }
        put_field(&mut out, 2, &fbody);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb_varint(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            out.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }
    fn field_bytes(out: &mut Vec<u8>, f: u32, b: &[u8]) {
        pb_varint(out, ((f as u64) << 3) | 2);
        pb_varint(out, b.len() as u64);
        out.extend_from_slice(b);
    }
    fn packed(v: &[u32]) -> Vec<u8> {
        let mut o = Vec::new();
        for &x in v {
            pb_varint(&mut o, x as u64);
        }
        o
    }
    fn zz(v: i32) -> u32 {
        ((v << 1) ^ (v >> 31)) as u32
    }

    #[test]
    fn decodes_a_layer_with_a_line_and_a_polygon() {
        // A line (2,3)→(10,3) and a square ring with ClosePath.
        let mut line = Vec::new();
        pb_varint(&mut line, (3 << 3) as u64);
        pb_varint(&mut line, 2);
        field_bytes(&mut line, 2, &packed(&[0, 0]));
        field_bytes(&mut line, 4, &packed(&[(1 << 3) | 1, zz(2), zz(3), (1 << 3) | 2, zz(8), zz(0)]));
        let mut poly = Vec::new();
        pb_varint(&mut poly, (3 << 3) as u64);
        pb_varint(&mut poly, 3);
        field_bytes(&mut poly, 2, &packed(&[1, 1]));
        field_bytes(&mut poly, 4, &packed(&[(1 << 3) | 1, zz(0), zz(0), (3 << 3) | 2, zz(4), zz(0), zz(0), zz(4), zz(-4), zz(0), 7 | (1 << 3)]));
        let mut val_road = Vec::new();
        field_bytes(&mut val_road, 1, b"major_road");
        let mut val_true = Vec::new();
        pb_varint(&mut val_true, 7 << 3);
        pb_varint(&mut val_true, 1);
        let mut layer = Vec::new();
        field_bytes(&mut layer, 1, b"roads");
        field_bytes(&mut layer, 2, &line);
        field_bytes(&mut layer, 2, &poly);
        field_bytes(&mut layer, 3, b"kind");
        field_bytes(&mut layer, 3, b"is_bridge");
        field_bytes(&mut layer, 4, &val_road);
        field_bytes(&mut layer, 4, &val_true);
        pb_varint(&mut layer, 5 << 3);
        pb_varint(&mut layer, 4096);
        let mut tile = Vec::new();
        field_bytes(&mut tile, 3, &layer);

        let ls = decode(&tile).unwrap();
        assert_eq!(ls[0].name, "roads");
        assert_eq!(ls[0].extent, 4096);
        let l = &ls[0].features[0];
        assert_eq!((l.kind, l.str("kind")), (GeomType::Line, Some("major_road")));
        assert_eq!(l.parts, vec![vec![[2.0, 3.0], [10.0, 3.0]]]);
        let p = &ls[0].features[1];
        assert!(p.flag("is_bridge"));
        assert_eq!(p.parts[0], vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]]);
    }

    /// A rectangle w × h tile units at (x, y), as polygon commands.
    fn rect(x: i32, y: i32, w: i32, h: i32) -> Vec<u32> {
        vec![(1 << 3) | 1, zz(x), zz(y), (3 << 3) | 2, zz(w), zz(0), zz(0), zz(h), zz(-w), zz(0), 7 | (1 << 3)]
    }

    fn building(tags: &[u32], geom: &[u32]) -> Vec<u8> {
        let mut f = Vec::new();
        pb_varint(&mut f, (3 << 3) as u64);
        pb_varint(&mut f, 3);
        if !tags.is_empty() {
            field_bytes(&mut f, 2, &packed(tags));
        }
        field_bytes(&mut f, 4, &packed(geom));
        f
    }

    #[test]
    fn buildings_without_a_height_gain_an_estimate_and_nothing_else_changes() {
        // z15 tile at 54.7°N: a tile unit is about 0.17 m.
        let (z, x, y) = (15u8, 18248u32, 10314u32);
        let mut roads = Vec::new();
        field_bytes(&mut roads, 1, b"roads");
        pb_varint(&mut roads, 5 << 3);
        pb_varint(&mut roads, 4096);
        let mut double = Vec::new();
        pb_varint(&mut double, (3 << 3) | 1);
        double.extend_from_slice(&20.0f64.to_le_bytes());
        let mut layer = Vec::new();
        field_bytes(&mut layer, 1, b"buildings");
        // A mapped 20 m building, an unmapped long narrow block (≈ 80 m × 13 m),
        // an unmapped garage row (≈ 60 m × 5 m) and an unmapped small house (≈ 10 m × 10 m).
        field_bytes(&mut layer, 2, &building(&[0, 0], &rect(0, 0, 60, 60)));
        field_bytes(&mut layer, 2, &building(&[], &rect(100, 100, 470, 76)));
        field_bytes(&mut layer, 2, &building(&[], &rect(1000, 1000, 352, 29)));
        field_bytes(&mut layer, 2, &building(&[], &rect(2000, 2000, 58, 58)));
        field_bytes(&mut layer, 3, b"height");
        field_bytes(&mut layer, 4, &double);
        pb_varint(&mut layer, 5 << 3);
        pb_varint(&mut layer, 4096);
        let mut tile = Vec::new();
        field_bytes(&mut tile, 3, &roads);
        field_bytes(&mut tile, 3, &layer);

        let out = with_estimated_heights(&tile, z, x, y).unwrap();
        let before = decode(&tile).unwrap();
        let after = decode(&out).unwrap();
        // The other layer is byte for byte the same.
        assert_eq!(&out[..roads.len() + 2], &tile[..roads.len() + 2]);
        let b = &after[1];
        assert_eq!(b.name, "buildings");
        for (f0, f1) in before[1].features.iter().zip(&b.features) {
            assert_eq!(f0.parts, f1.parts, "geometry untouched");
        }
        let est = |i: usize| b.features[i].props.get("est_height").and_then(Value::as_f64);
        assert_eq!(b.features[0].props.get("height").and_then(Value::as_f64), Some(20.0));
        assert_eq!(est(0), None, "a mapped height is kept, not estimated");
        assert_eq!(est(1), Some(15.0), "a long block: a five-storey panel block");
        assert_eq!(est(2), Some(3.5), "a garage row");
        assert_eq!(est(3), Some(7.0), "a house");
        // Running it again changes nothing more than once (idempotent in effect).
        assert!(decode(&with_estimated_heights(&out, z, x, y).unwrap()).unwrap()[1].features.iter().all(|f| f.props.contains_key("est_height") || f.props.contains_key("height")));
    }

    #[test]
    fn estimates_by_footprint() {
        let rect = |a: f64, b: f64| estimate_height(a * b, 2.0 * (a + b));
        assert_eq!(rect(4.0, 5.0), 3.0, "a shed");
        assert_eq!(rect(10.0, 12.0), 7.0, "a house");
        assert_eq!(rect(60.0, 12.0), 15.0, "a five-storey block");
        assert_eq!(rect(150.0, 14.0), 27.0, "a nine-storey block");
        assert_eq!(rect(60.0, 50.0), 12.0, "a hall: low for its size");
        assert_eq!(rect(50.0, 6.0), 3.5, "garages");
    }
}
