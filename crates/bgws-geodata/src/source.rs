//! Byte ranges from an archive: a local file, or a URL read with HTTP range
//! requests. Remote ranges are kept in a disk cache, keyed by the URL and the
//! range, so a rebuild or a grown box fetches only what is new.

use crate::GeoError;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Somewhere to read byte ranges from.
pub trait RangeSource: Send + Sync {
    fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>, GeoError>;
    /// For messages and the pack's provenance.
    fn name(&self) -> &str;
}

pub struct FileSource {
    path: PathBuf,
    name: String,
    file: Mutex<std::fs::File>,
}

impl FileSource {
    pub fn open(path: &Path) -> Result<Self, GeoError> {
        Ok(Self { path: path.to_path_buf(), name: path.display().to_string(), file: Mutex::new(std::fs::File::open(path)?) })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl RangeSource for FileSource {
    fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>, GeoError> {
        let mut f = self.file.lock().map_err(|_| GeoError::Io("file lock poisoned".into()))?;
        f.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0; len as usize];
        f.read_exact(&mut buf)?;
        Ok(buf)
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// A remote archive read by HTTP range requests, cached on disk.
pub struct HttpSource {
    url: String,
    client: reqwest::blocking::Client,
    cache: Option<PathBuf>,
}

impl HttpSource {
    /// `cache`: a directory for fetched ranges (created if missing); `None` keeps nothing.
    pub fn new(url: &str, cache: Option<&Path>) -> Result<Self, GeoError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("bgws-geodata/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| GeoError::Http(e.to_string()))?;
        let cache = match cache {
            Some(dir) => {
                let d = dir.join(format!("{:016x}", bgws_core::rng::fnv1a(url.as_bytes())));
                std::fs::create_dir_all(&d)?;
                // A note of what the folder holds, for whoever clears the cache.
                let _ = std::fs::write(d.join("URL"), url);
                Some(d)
            }
            None => None,
        };
        Ok(Self { url: url.to_string(), client, cache })
    }

    fn fetch(&self, offset: u64, len: u64) -> Result<Vec<u8>, GeoError> {
        let mut last = String::new();
        // A few tries: planet archives sit behind CDNs that drop the odd request.
        for attempt in 0..4 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(500 << attempt));
            }
            let res = self.client.get(&self.url).header("Range", format!("bytes={}-{}", offset, offset + len - 1)).send();
            match res {
                Ok(r) if r.status() == reqwest::StatusCode::PARTIAL_CONTENT => {
                    let b = r.bytes().map_err(|e| GeoError::Http(e.to_string()))?;
                    if b.len() as u64 == len {
                        return Ok(b.to_vec());
                    }
                    last = format!("{} bytes for a {len}-byte range", b.len());
                }
                Ok(r) if r.status() == reqwest::StatusCode::OK => return Err(GeoError::Http(format!("{} ignores range requests", self.url))),
                Ok(r) => last = format!("HTTP {}", r.status()),
                Err(e) => last = e.to_string(),
            }
        }
        Err(GeoError::Http(format!("{} [{offset}+{len}]: {last}", self.url)))
    }
}

impl RangeSource for HttpSource {
    fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>, GeoError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let Some(dir) = &self.cache else { return self.fetch(offset, len) };
        let file = dir.join(format!("{offset}-{len}"));
        if let Ok(b) = std::fs::read(&file) {
            if b.len() as u64 == len {
                return Ok(b);
            }
        }
        let b = self.fetch(offset, len)?;
        // Write then rename, so a parallel reader never sees half a range; two
        // threads fetching the same range each write their own part file.
        static PART: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = PART.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = dir.join(format!("{offset}-{len}.{:x}-{n}.part", std::process::id()));
        std::fs::write(&tmp, &b)?;
        std::fs::rename(&tmp, &file)?;
        Ok(b)
    }
    fn name(&self) -> &str {
        &self.url
    }
}

/// A source from a path or an http(s) URL.
pub fn open_source(location: &str, cache: Option<&Path>) -> Result<Box<dyn RangeSource>, GeoError> {
    if location.starts_with("http://") || location.starts_with("https://") {
        Ok(Box::new(HttpSource::new(location, cache)?))
    } else {
        Ok(Box::new(FileSource::open(Path::new(location))?))
    }
}
