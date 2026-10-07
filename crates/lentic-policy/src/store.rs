//! The signer's state on disk: its FROST share of every loan, the loans it holds and what it
//! has co-signed.
//!
//! In development these are plain JSON files under `--state-dir`. In a dstack CVM the
//! directory is the app's encrypted volume, sealed to keys only this code receives, so the
//! shares never leave the TEE in the clear.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        for sub in ["shares", "intents"] {
            std::fs::create_dir_all(dir.join(sub)).with_context(|| format!("creating {}", dir.join(sub).display()))?;
        }
        Ok(Store { dir: dir.to_path_buf() })
    }

    fn path(&self, kind: &str, id: &str) -> PathBuf {
        self.dir.join(kind).join(format!("{id}.json"))
    }

    /// Writes atomically (temp file, then rename), so a crash never leaves half a share.
    pub fn put<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<()> {
        let path = self.path(kind, id);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn all<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(self.dir.join(kind))? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let raw = std::fs::read(&path)?;
            out.push(serde_json::from_slice(&raw).with_context(|| format!("parsing {}", path.display()))?);
        }
        Ok(out)
    }
}
