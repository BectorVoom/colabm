use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::{self, Paths};

/// Everything colabm remembers. Holds no credentials: those stay in each
/// account's colab-cli directory, in the format the colab CLI wrote them.
#[derive(Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<Passphrase>,
    #[serde(default)]
    pub accounts: BTreeMap<String, Account>,
}

#[derive(Serialize, Deserialize)]
pub struct Passphrase {
    pub salt: String,
    pub hash: String,
}

#[derive(Serialize, Deserialize)]
pub struct Account {
    pub email: Option<String>,
    pub added_at: u64,
    /// Result of the last successful `limits` check.
    pub usage: Option<Usage>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Paid compute units.
    pub balance: f64,
    pub rate_hourly: f64,
    pub assignments: u32,
    pub checked_at: u64,
    /// Accelerators Colab will currently grant the account. `None` when
    /// only the plain `colab usage` figures could be read.
    #[serde(default)]
    pub accelerators: Option<Vec<String>>,
    #[serde(default)]
    pub free: Option<FreeQuota>,
}

/// The free-tier allowance. Colab reports it only while the account has a
/// runtime running, so it carries its own timestamp and can be older than
/// the rest of the check.
#[derive(Clone, Serialize, Deserialize)]
pub struct FreeQuota {
    /// Compute units left before the next refill.
    pub remaining: f64,
    pub refill_at: u64,
    pub checked_at: u64,
}

impl State {
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.state_file();
        match fs::read_to_string(&file) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("parsing {}", file.display()))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", file.display())),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        paths::private_dirs(&paths.data)?;
        let file = paths.state_file();
        let tmp = file.with_extension("json.tmp");
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("writing {}", tmp.display()))?;
        out.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        out.sync_all()?;
        fs::rename(&tmp, &file).with_context(|| format!("writing {}", file.display()))
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
