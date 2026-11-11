//! The HLA data encoding (IEEE 1516.2 §4.13): big-endian basic types, fixed
//! records with each field aligned to its own octet boundary, variant
//! records (discriminant, then the alternative at its alignment), fixed and
//! variable arrays (a 32-bit count first), and HLAunicodeString (a count of
//! UTF-16 code units, then each big-endian). Padding is counted from the
//! start of the value, so every value is encoded in one buffer from 0.

#[derive(Default, Debug, Clone)]
pub struct Enc {
    pub buf: Vec<u8>,
}

impl Enc {
    pub fn new() -> Enc {
        Enc::default()
    }
    /// Pad with zeros to a multiple of `n` octets.
    pub fn align(&mut self, n: usize) -> &mut Self {
        while !self.buf.len().is_multiple_of(n) {
            self.buf.push(0);
        }
        self
    }
    pub fn octet(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.align(2);
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.align(4);
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.u32(v as u32)
    }
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.u32(v.to_bits())
    }
    pub fn f64(&mut self, v: f64) -> &mut Self {
        self.align(8);
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn f64x3(&mut self, v: [f64; 3]) -> &mut Self {
        v.into_iter().for_each(|x| {
            self.f64(x);
        });
        self
    }
    pub fn f32x3(&mut self, v: [f32; 3]) -> &mut Self {
        v.into_iter().for_each(|x| {
            self.f32(x);
        });
        self
    }
    /// HLAunicodeString.
    pub fn unicode(&mut self, s: &str) -> &mut Self {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.u32(units.len() as u32);
        for u in units {
            self.buf.extend_from_slice(&u.to_be_bytes());
        }
        self
    }
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }
    pub fn done(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HLA decoding: {}", self.0)
    }
}

pub type DResult<T> = Result<T, DecodeError>;

pub struct Dec<'a> {
    pub b: &'a [u8],
    pub at: usize,
}

impl<'a> Dec<'a> {
    pub fn new(b: &'a [u8]) -> Dec<'a> {
        Dec { b, at: 0 }
    }
    fn take(&mut self, n: usize) -> DResult<&'a [u8]> {
        let s = self.b.get(self.at..self.at + n).ok_or_else(|| DecodeError(format!("{} octets short at {}", n, self.at)))?;
        self.at += n;
        Ok(s)
    }
    pub fn align(&mut self, n: usize) -> DResult<()> {
        let pad = (n - self.at % n) % n;
        self.take(pad).map(|_| ())
    }
    pub fn octet(&mut self) -> DResult<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> DResult<u16> {
        self.align(2)?;
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> DResult<u32> {
        self.align(4)?;
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> DResult<i32> {
        Ok(self.u32()? as i32)
    }
    pub fn f32(&mut self) -> DResult<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub fn f64(&mut self) -> DResult<f64> {
        self.align(8)?;
        Ok(f64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn f64x3(&mut self) -> DResult<[f64; 3]> {
        Ok([self.f64()?, self.f64()?, self.f64()?])
    }
    pub fn f32x3(&mut self) -> DResult<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    pub fn unicode(&mut self) -> DResult<String> {
        let n = self.u32()? as usize;
        if n > 1 << 20 {
            return Err(DecodeError(format!("string of {n} code units")));
        }
        let units: Vec<u16> = self.take(2 * n)?.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        Ok(String::from_utf16_lossy(&units))
    }
    pub fn bytes(&mut self, n: usize) -> DResult<&'a [u8]> {
        self.take(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_align_from_the_start_of_the_value() {
        let b = Enc::new().octet(7).u16(0x0102).f64(1.0).unicode("Aé").done();
        assert_eq!(b[..4], [7, 0, 1, 2]);
        assert_eq!(b[4..8], [0; 4], "padding to the double");
        assert_eq!(b[8..16], 1.0f64.to_be_bytes());
        assert_eq!(b[16..], [0, 0, 0, 2, 0, 0x41, 0, 0xE9]);
        let mut d = Dec::new(&b);
        assert_eq!((d.octet().unwrap(), d.u16().unwrap(), d.f64().unwrap(), d.unicode().unwrap()), (7, 0x0102, 1.0, "Aé".to_string()));
        assert!(Dec::new(&b[..6]).f64().is_err(), "six octets are not a double");
    }
}
