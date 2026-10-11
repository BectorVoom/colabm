use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Error, Result, anyhow, bail};
use serde_json::Value;

use crate::paths::Paths;
use crate::state::{self, FreeQuota, Usage};

const SIGN_IN_REQUIRED: &str = "sign-in required";

/// Name of the short-lived runtime `probe` starts.
const PROBE_SESSION: &str = "colabm-probe";

/// Asks Colab for the account's full usage record. `colab usage` prints only
/// the paid balance; the free allowance and usable accelerators come back in
/// the same response but are dropped, so this fetches it through the colab
/// CLI's own authenticated client and prints just the fields below. Tokens
/// stay inside that process.
const DETAIL_SCRIPT: &str = r#"
import json, sys
from colab_cli.common import state
if len(sys.argv) > 1:
    state.client_oauth_config = sys.argv[1]
client = state.client
resp = client.session.get(
    client.colab_domain + "/tun/m/ccu-info",
    params={"authuser": "0"},
    headers={"Accept": "application/json", "X-Colab-Client-Agent": "colab-cli"},
    timeout=30,
)
resp.raise_for_status()
info = json.loads(resp.text.removeprefix(")]}'\n"))
keep = ("currentBalance", "consumptionRateHourly", "assignmentsCount",
        "eligibleGpus", "eligibleTpus", "freeCcuQuotaInfo")
print(json.dumps({k: info[k] for k in keep if k in info}))
"#;

/// Counts the account's runtimes as Colab's assignment list has them. The
/// count inside the usage record keeps showing a runtime for minutes after
/// it stopped, so it cannot confirm a stop.
const ASSIGNMENTS_SCRIPT: &str = r#"
import sys
from colab_cli.common import state
if len(sys.argv) > 1:
    state.client_oauth_config = sys.argv[1]
print(len(state.client.list_assignments()))
"#;

fn binary_name() -> PathBuf {
    env::var_os("COLABM_COLAB_BIN").map_or_else(|| "colab".into(), PathBuf::from)
}

fn binary() -> Command {
    Command::new(binary_name())
}

/// The Python interpreter the colab CLI is installed into, read from the
/// launcher script's `#!` line.
fn interpreter() -> Option<PathBuf> {
    let name = binary_name();
    let script = if name.components().count() > 1 {
        name
    } else {
        env::split_paths(&env::var_os("PATH")?)
            .map(|dir| dir.join(&name))
            .find(|candidate| candidate.is_file())?
    };
    let head: Vec<u8> = fs::read(script).ok()?.into_iter().take(512).collect();
    let head = String::from_utf8_lossy(&head).into_owned();
    let program = head
        .lines()
        .next()?
        .strip_prefix("#!")?
        .split_whitespace()
        .next()?;
    let program = PathBuf::from(program);
    let is_python = program.file_name()?.to_str()?.starts_with("python");
    is_python.then_some(program)
}

/// `colab` exactly as the user would run it: acts on the active account.
pub fn plain() -> Command {
    binary()
}

/// `colab` confined to one account. The colab CLI hard-codes its token,
/// session and settings paths under `~`, so the account's own home is the
/// only way to aim it at a different login.
pub fn for_account(paths: &Paths, account: &str) -> Command {
    let mut cmd = binary();
    cmd.env("HOME", paths.account_home(account));
    // A custom OAuth client config lives in the real home; keep honouring it.
    let oauth = paths.home.join(".colab-cli-oauth-config.json");
    if oauth.exists() {
        cmd.arg("-c").arg(oauth);
    }
    cmd
}

pub fn spawn_error(e: io::Error) -> Error {
    if e.kind() == io::ErrorKind::NotFound {
        anyhow!(
            "the `colab` command was not found. Install it with \
             `uv tool install google-colab-cli`, or set COLABM_COLAB_BIN"
        )
    } else {
        anyhow!("could not run colab: {e}")
    }
}

pub struct Captured {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Starts colab detached from the terminal, so a missing login fails fast
/// instead of waiting for an authorization code.
pub fn spawn_captured(mut cmd: Command) -> Result<Child> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_error)
}

pub fn finish(mut child: Child, deadline: Instant) -> Result<Captured> {
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Captured {
                ok: false,
                stdout: String::new(),
                stderr: "timed out waiting for colab".into(),
            });
        }
        thread::sleep(Duration::from_millis(50));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    Ok(Captured {
        ok: status.success(),
        stdout,
        stderr,
    })
}

/// Whether the account holds a login the colab CLI can use unattended.
/// Looks only at whether a refresh token is present, never at its value.
pub fn signed_in(cfg: &Path) -> bool {
    fs::read_to_string(cfg.join("token.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|v| v["refresh_token"].as_str().is_some_and(|t| !t.is_empty()))
}

/// A limits check in flight for one account.
pub struct Check {
    child: Child,
    detailed: bool,
}

/// Runs a script inside the colab CLI's own Python environment as the
/// account, or `None` when that environment cannot be found.
fn python(paths: &Paths, account: &str, script: &str) -> Option<Command> {
    let mut cmd = Command::new(interpreter()?);
    cmd.env("HOME", paths.account_home(account))
        .args(["-c", script]);
    let oauth = paths.home.join(".colab-cli-oauth-config.json");
    if oauth.exists() {
        cmd.arg(oauth);
    }
    Some(cmd)
}

pub fn start_check(paths: &Paths, account: &str) -> Result<Check> {
    let detail = python(paths, account, DETAIL_SCRIPT);
    let detailed = detail.is_some();
    let child = spawn_captured(detail.unwrap_or_else(|| usage_command(paths, account)))?;
    Ok(Check { child, detailed })
}

/// The account's usage, or a one-line reason it could not be read.
pub fn finish_check(
    paths: &Paths,
    account: &str,
    check: Check,
    deadline: Instant,
    timeout: Duration,
) -> Result<Result<Usage, String>> {
    let mut out = finish(check.child, deadline)?;
    if check.detailed {
        if let Some(usage) = parse_detail(&out.stdout).filter(|_| out.ok) {
            return Ok(Ok(usage));
        }
        if failure_reason(&out) == SIGN_IN_REQUIRED {
            return Ok(Err(SIGN_IN_REQUIRED.into()));
        }
        // The detailed route leans on colab-cli internals; if a new release
        // moved them, the supported command still gives the paid figures.
        let plain = spawn_captured(usage_command(paths, account))?;
        out = finish(plain, Instant::now() + timeout)?;
    }
    Ok(match parse_usage(&out.stdout).filter(|_| out.ok) {
        Some(usage) => Ok(usage),
        None if out.ok => Err("unrecognised `colab usage` output".into()),
        None => Err(failure_reason(&out)),
    })
}

/// Reads the free allowance of an idle account by running a CPU runtime
/// just long enough to ask for it, since Colab reports it at no other time.
/// The inner error is a reason the allowance could not be read; the outer
/// one means the runtime may have been left running.
pub fn probe(paths: &Paths, account: &str, timeout: Duration) -> Result<Result<FreeQuota, String>> {
    let colab = |args: &[&str], limit: Duration| {
        let mut cmd = for_account(paths, account);
        cmd.args(args);
        finish(spawn_captured(cmd)?, Instant::now() + limit)
    };
    let check = || {
        let started = start_check(paths, account)?;
        finish_check(paths, account, started, Instant::now() + timeout, timeout)
    };

    let started = colab(&["new", "-s", PROBE_SESSION], timeout * 4)?;
    let reading = if started.ok {
        check()?
    } else {
        Err(format!(
            "could not start a runtime: {}",
            failure_reason(&started)
        ))
    };
    // Stop even after a failed start: one that timed out may still have
    // created the runtime.
    let _ = colab(&["stop", "-s", PROBE_SESSION], timeout);
    // `colab stop` exits 0 whether or not it stopped anything, so ask Colab.
    let count = || -> Result<Option<usize>> {
        let Some(cmd) = python(paths, account, ASSIGNMENTS_SCRIPT) else {
            return Ok(None);
        };
        let out = finish(spawn_captured(cmd)?, Instant::now() + timeout)?;
        Ok(out.stdout.trim().parse().ok().filter(|_| out.ok))
    };
    let mut running = None;
    for attempt in 0..3 {
        if attempt > 0 {
            thread::sleep(Duration::from_secs(2));
        }
        running = count()?;
        if running == Some(0) {
            break;
        }
    }
    if running != Some(0) {
        bail!(
            "the temporary runtime on '{account}' may still be running and using compute units; \
             stop it with `colabm run -a {account} stop -s {PROBE_SESSION}`"
        );
    }
    Ok(reading.and_then(|usage| {
        usage
            .free
            .ok_or_else(|| "Colab did not report a free allowance".to_string())
    }))
}

fn usage_command(paths: &Paths, account: &str) -> Command {
    let mut cmd = for_account(paths, account);
    cmd.arg("usage");
    cmd
}

/// Parses the JSON line printed by `DETAIL_SCRIPT`.
pub fn parse_detail(out: &str) -> Option<Usage> {
    let line = out
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))?;
    let info: Value = serde_json::from_str(line).ok()?;
    // Colab sends some counters as strings.
    let number = |v: &Value| v.as_f64().or_else(|| v.as_str()?.parse().ok());
    let now = state::now();
    let names = |key: &str| {
        let list = info[key].as_array()?;
        Some(
            list.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>(),
        )
    };
    let free = info.get("freeCcuQuotaInfo").and_then(|quota| {
        Some(FreeQuota {
            // The allowance is counted in thousandths of a compute unit.
            remaining: number(&quota["remainingTokens"])? / 1000.0,
            refill_at: number(&quota["nextRefillTimestampSec"])? as u64,
            checked_at: now,
        })
    });
    Some(Usage {
        balance: number(&info["currentBalance"])?,
        rate_hourly: number(&info["consumptionRateHourly"])?,
        assignments: number(&info["assignmentsCount"])? as u32,
        checked_at: now,
        accelerators: names("eligibleGpus").map(|mut gpus| {
            gpus.extend(names("eligibleTpus").unwrap_or_default());
            gpus
        }),
        free,
    })
}

/// Parses the three labelled lines printed by `colab usage`.
pub fn parse_usage(out: &str) -> Option<Usage> {
    let field = |label: &str| out.lines().find_map(|l| l.trim().strip_prefix(label));
    Some(Usage {
        balance: field("Current balance:")?
            .split_whitespace()
            .next()?
            .parse()
            .ok()?,
        rate_hourly: field("Usage rate:")?
            .trim()
            .trim_end_matches("/hr")
            .parse()
            .ok()?,
        assignments: field("Active assignments:")?.trim().parse().ok()?,
        checked_at: state::now(),
        accelerators: None,
        free: None,
    })
}

/// Parses the `Email:` line printed by `colab whoami`.
pub fn parse_email(out: &str) -> Option<String> {
    out.lines()
        .find_map(|l| l.trim().strip_prefix("Email:"))
        .map(str::trim)
        .filter(|e| e.contains('@') && !e.contains(char::is_whitespace))
        .map(str::to_string)
}

/// A one-line, secret-free explanation of why a captured colab call failed.
pub fn failure_reason(out: &Captured) -> String {
    let needs_login = [&out.stderr, &out.stdout]
        .iter()
        .any(|s| s.contains("To authorize colab-cli") || s.contains("authorization code"));
    if needs_login {
        return SIGN_IN_REQUIRED.into();
    }
    // colab writes some of its errors to stdout.
    let last = [&out.stderr, &out.stdout]
        .iter()
        .find_map(|s| s.lines().rev().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("colab exited with an error");
    scrub(&last.chars().take(200).collect::<String>())
}

/// Colab's own error text can embed credentials (URLs with token
/// parameters, echoed responses), so nothing from it is relayed unscrubbed.
pub fn scrub(text: &str) -> String {
    let secret = |word: &str| {
        let lower = word.to_ascii_lowercase();
        word.contains("ya29.") || word.contains("1//") || lower.contains("token=")
    };
    text.split(' ')
        .map(|word| if secret(word) { "[redacted]" } else { word })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_output_is_parsed() {
        let out =
            "Current balance: 87.50 compute units\nUsage rate: 1.76/hr\nActive assignments: 2\n";
        let usage = parse_usage(out).unwrap();
        assert_eq!(usage.balance, 87.5);
        assert_eq!(usage.rate_hourly, 1.76);
        assert_eq!(usage.assignments, 2);
    }

    #[test]
    fn usage_survives_surrounding_noise() {
        let out = "[colab] update available\nCurrent balance: 0.00 compute units\nUsage rate: 0.00/hr\nActive assignments: 0";
        assert_eq!(parse_usage(out).unwrap().balance, 0.0);
        assert!(parse_usage("Current balance: lots").is_none());
    }

    #[test]
    fn detail_carries_the_free_allowance_while_a_runtime_is_up() {
        let out = r#"{"currentBalance": 0, "consumptionRateHourly": 0.08, "assignmentsCount": 1, "eligibleGpus": ["T4"], "eligibleTpus": ["V5E1"], "freeCcuQuotaInfo": {"remainingTokens": "1565", "nextRefillTimestampSec": 1791719222}}"#;
        let usage = parse_detail(out).unwrap();
        assert_eq!(usage.rate_hourly, 0.08);
        assert_eq!(usage.accelerators.unwrap(), ["T4", "V5E1"]);
        let free = usage.free.unwrap();
        assert_eq!(free.remaining, 1.565);
        assert_eq!(free.refill_at, 1791719222);
    }

    #[test]
    fn detail_without_a_runtime_has_no_free_allowance() {
        let out = r#"{"currentBalance": 12.5, "consumptionRateHourly": 0, "assignmentsCount": 0, "eligibleGpus": []}"#;
        let usage = parse_detail(out).unwrap();
        assert_eq!(usage.balance, 12.5);
        assert!(usage.free.is_none());
        assert_eq!(usage.accelerators.unwrap(), Vec::<String>::new());
        assert!(parse_detail("Traceback (most recent call last):").is_none());
    }

    #[test]
    fn email_is_parsed_only_when_present() {
        assert_eq!(
            parse_email("Auth provider: oauth2\nEmail:         a@example.com\n").as_deref(),
            Some("a@example.com")
        );
        assert_eq!(
            parse_email("Email:         <unavailable: token has no scope>"),
            None
        );
    }

    #[test]
    fn error_text_never_carries_tokens() {
        let out = Captured {
            ok: false,
            stdout: String::new(),
            stderr: "Traceback\nfailed GET https://x/y?colab-runtime-proxy-token=abc123 with ya29.SECRET and 1//REFRESH".into(),
        };
        let reason = failure_reason(&out);
        assert!(
            !reason.contains("abc123") && !reason.contains("SECRET") && !reason.contains("REFRESH")
        );
        assert!(reason.starts_with("failed GET"));
    }

    #[test]
    fn auth_prompt_is_reported_as_sign_in_required() {
        let out = Captured {
            ok: false,
            stdout: String::new(),
            stderr: "To authorize colab-cli, visit this URL in any browser:\nEOFError".into(),
        };
        assert_eq!(failure_reason(&out), "sign-in required");
    }
}
