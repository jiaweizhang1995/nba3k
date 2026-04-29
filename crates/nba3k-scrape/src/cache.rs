//! File-based response cache. Hit before going to network.
//!
//! Layout: `<root>/cache/<source>/<safe_key>.<ext>`.
//!
//! TTL is enforced at read time via mtime. Expired entries return `None`
//! so callers fetch fresh; we don't proactively evict.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).with_context(|| format!("create cache root {root:?}"))?;
        Ok(Self { root })
    }

    fn path(&self, source: &str, key: &str, ext: &str) -> PathBuf {
        let safe: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let mut p = self.root.join(source);
        let _ = fs::create_dir_all(&p);
        p.push(format!("{safe}.{ext}"));
        p
    }

    pub fn get(&self, source: &str, key: &str, ext: &str, ttl: Duration) -> Option<Vec<u8>> {
        let p = self.path(source, key, ext);
        let meta = fs::metadata(&p).ok()?;
        let mtime = meta.modified().ok()?;
        let age = SystemTime::now()
            .duration_since(mtime)
            .unwrap_or(Duration::ZERO);
        if age > ttl {
            return None;
        }
        fs::read(&p).ok()
    }

    pub fn put(&self, source: &str, key: &str, ext: &str, bytes: &[u8]) -> Result<()> {
        let p = self.path(source, key, ext);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&p, bytes).with_context(|| format!("write cache {p:?}"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub fn html_ttl() -> Duration {
    Duration::from_secs(60 * 60 * 24 * 30) // 30 days
}

pub fn json_ttl() -> Duration {
    Duration::from_secs(60 * 60 * 24 * 7) // 7 days
}
