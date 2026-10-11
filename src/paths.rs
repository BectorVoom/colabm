use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, symlink};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// What currently sits at `~/.config/colab-cli`, the directory the colab CLI reads.
#[derive(Debug, PartialEq)]
pub enum Link {
    Missing,
    /// Symlink into one of our accounts.
    Account(String),
    /// A real directory: a colab login that predates colabm.
    RealDir,
    /// Something we did not create and must not replace.
    Foreign,
}

pub struct Paths {
    pub home: PathBuf,
    pub data: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let var = |key| {
            env::var_os(key)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = var("HOME").context("HOME is not set")?;
        let data = var("COLABM_DIR")
            .or_else(|| var("XDG_DATA_HOME").map(|x| x.join("colabm")))
            .unwrap_or_else(|| home.join(".local/share/colabm"));
        let data =
            std::path::absolute(&data).with_context(|| format!("resolving {}", data.display()))?;
        Ok(Self { home, data })
    }

    pub fn state_file(&self) -> PathBuf {
        self.data.join("state.json")
    }

    fn accounts_dir(&self) -> PathBuf {
        self.data.join("accounts")
    }

    pub fn account_dir(&self, name: &str) -> PathBuf {
        self.accounts_dir().join(name)
    }

    /// The directory an account's colab processes see as `$HOME`.
    pub fn account_home(&self, name: &str) -> PathBuf {
        self.account_dir(name).join("home")
    }

    /// The account's own copy of `~/.config/colab-cli`.
    pub fn account_cfg(&self, name: &str) -> PathBuf {
        self.account_home(name).join(".config/colab-cli")
    }

    pub fn link(&self) -> PathBuf {
        self.home.join(".config/colab-cli")
    }

    pub fn link_state(&self) -> Result<Link> {
        let link = self.link();
        let meta = match fs::symlink_metadata(&link) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Link::Missing),
            Err(e) => return Err(e).with_context(|| format!("inspecting {}", link.display())),
        };
        if !meta.file_type().is_symlink() {
            return Ok(if meta.is_dir() {
                Link::RealDir
            } else {
                Link::Foreign
            });
        }
        let target = fs::read_link(&link)?;
        let name = target
            .strip_prefix(self.accounts_dir())
            .ok()
            .and_then(|rest| rest.components().next())
            .and_then(|c| c.as_os_str().to_str());
        Ok(match name {
            Some(name) if self.account_cfg(name) == target => Link::Account(name.to_string()),
            _ => Link::Foreign,
        })
    }

    /// Creates the account's private home, owner-only all the way down.
    pub fn create_account(&self, name: &str, with_cfg: bool) -> Result<()> {
        let dir = if with_cfg {
            self.account_cfg(name)
        } else {
            self.account_home(name).join(".config")
        };
        private_dirs(&dir)?;
        // `colab ssh` looks for keys under $HOME/.ssh.
        let ssh = self.home.join(".ssh");
        let shim = self.account_home(name).join(".ssh");
        if ssh.is_dir() && fs::symlink_metadata(&shim).is_err() {
            symlink(&ssh, &shim).with_context(|| format!("linking {}", shim.display()))?;
        }
        Ok(())
    }

    /// Atomically repoints `~/.config/colab-cli` at the account. The caller
    /// checks `link_state` first: this replaces a symlink, never a directory.
    pub fn point_link(&self, name: &str) -> Result<()> {
        let link = self.link();
        let parent = link.parent().context("link has no parent")?;
        fs::create_dir_all(parent)?;
        let tmp = parent.join(".colab-cli.colabm-tmp");
        let _ = fs::remove_file(&tmp);
        symlink(self.account_cfg(name), &tmp)?;
        fs::rename(&tmp, &link).with_context(|| format!("updating {}", link.display()))
    }
}

pub fn private_dirs(dir: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))
}

pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(['.', '-'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !ok {
        bail!("invalid account name '{name}': use letters, digits, '.', '_' or '-'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_cannot_escape_the_accounts_dir() {
        for bad in ["", "..", ".hidden", "a/b", "-x", "a b", "é"] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be rejected");
        }
        for good in ["work", "me.personal", "acct_2", "a-b"] {
            assert!(validate_name(good).is_ok(), "{good:?} should be accepted");
        }
    }
}
