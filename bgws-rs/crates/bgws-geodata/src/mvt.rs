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
            for kv in tags.chunks_exact(2) {
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
}
