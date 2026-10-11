//! The line between what an AI agent driving this CLI may see and what only
//! the person at the keyboard may see. Secrets are written to the terminal
//! device itself, never to stdout, and only after a passphrase the agent
//! does not have.

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

use anyhow::{Context, Result, anyhow, bail};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::paths::Paths;
use crate::state::{Passphrase, State};

/// Environment variables that coding agents set in the shells they spawn.
const AGENT_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CURSOR_AGENT",
    "CODEX_SANDBOX",
    "GEMINI_CLI",
    "OPENCODE",
    "AI_AGENT",
];

const MIN_PASSPHRASE: usize = 8;

/// Proves a person is present and returns the terminal to write secrets to.
/// The flag is true when this call created the passphrase.
pub fn human(paths: &Paths, state: &mut State) -> Result<(File, bool)> {
    if let Some(var) = AGENT_ENV.iter().find(|var| env::var_os(var).is_some()) {
        bail!(
            "refused: this shell belongs to an AI agent ({var} is set). \
             Run this yourself in a normal terminal."
        );
    }
    let mut tty = OpenOptions::new()
        .write(true)
        .open("/dev/tty")
        .map_err(|_| {
            anyhow!("refused: no terminal attached. Run this yourself in a normal terminal.")
        })?;
    match &state.passphrase {
        Some(stored) => {
            let entered = rpassword::prompt_password("colabm passphrase: ")?;
            if !verify(stored, &entered)? {
                bail!("wrong passphrase");
            }
            Ok((tty, false))
        }
        None => {
            writeln!(
                tty,
                "No passphrase yet. Choose one; colabm asks for it before showing secrets."
            )?;
            state.passphrase = Some(choose()?);
            state.save(paths)?;
            Ok((tty, true))
        }
    }
}

/// Prompts twice for a new passphrase.
pub fn choose() -> Result<Passphrase> {
    let first = rpassword::prompt_password("New passphrase: ")?;
    if first.chars().count() < MIN_PASSPHRASE {
        bail!("passphrase must be at least {MIN_PASSPHRASE} characters");
    }
    if rpassword::prompt_password("Repeat passphrase: ")? != first {
        bail!("passphrases do not match");
    }
    let mut salt = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut salt))
        .context("reading /dev/urandom")?;
    Ok(Passphrase {
        salt: hex(&salt),
        hash: hex(&derive(&first, &salt)?),
    })
}

fn verify(stored: &Passphrase, entered: &str) -> Result<bool> {
    let salt = unhex(&stored.salt).context("stored passphrase salt is corrupt")?;
    let expected = unhex(&stored.hash).context("stored passphrase hash is corrupt")?;
    let actual = derive(entered, &salt)?;
    // Compare every byte regardless of where the first difference is.
    let diff = actual
        .iter()
        .zip(&expected)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    Ok(expected.len() == actual.len() && diff == 0)
}

fn derive(passphrase: &str, salt: &[u8]) -> Result<[u8; 32]> {
    // Pinned so a crate upgrade cannot change the hash of a stored passphrase.
    let params = Params::new(19 * 1024, 2, 1, Some(32)).map_err(|e| anyhow!("argon2: {e}"))?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow!("argon2: {e}"))?;
    Ok(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_right_passphrase_verifies() {
        let salt = [7u8; 16];
        let stored = Passphrase {
            salt: hex(&salt),
            hash: hex(&derive("correct horse", &salt).unwrap()),
        };
        assert!(verify(&stored, "correct horse").unwrap());
        assert!(!verify(&stored, "correct horsf").unwrap());
        assert!(!verify(&stored, "").unwrap());
    }

    #[test]
    fn hex_round_trips_and_rejects_garbage() {
        assert_eq!(unhex(&hex(&[0, 15, 255])).unwrap(), vec![0, 15, 255]);
        assert!(unhex("abc").is_none());
        assert!(unhex("zz").is_none());
    }
}
