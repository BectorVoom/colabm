//! Drives the real binary against a throwaway HOME and a stand-in `colab`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const FAKE_TOKEN: &str = "ya29.FAKE-ACCESS-TOKEN-0123456789";
const FAKE_REFRESH: &str = "1//FAKE-REFRESH-TOKEN-0123456789";

/// Behaves like the colab CLI as far as colabm relies on it: state lives
/// under `$HOME/.config/colab-cli`, and a missing login asks for a code.
const STUB: &str = r#"#!/bin/sh
cfg="$HOME/.config/colab-cli"
while [ "$1" = "-c" ]; do shift 2; done
case "$1" in
  usage)
    if [ ! -f "$cfg/token.json" ] || [ -f "$cfg/revoked" ]; then
      echo "To authorize colab-cli, visit this URL in any browser:" >&2
      exit 1
    fi
    cat "$cfg/usage.txt" ;;
  whoami) echo "Email:         $(cat "$cfg/email")" ;;
  new) touch "$cfg/running"; echo "[colab] Session READY." ;;
  stop)
    # Like the real command, this exits 0 even when nothing was stopped.
    [ -f "$cfg/stuck" ] || rm -f "$cfg/running"
    echo "[colab] Session terminated." ;;
  *) echo "HOME=$HOME ARGS=$*" ;;
esac
"#;

/// Stands in for the Python interpreter the colab CLI is installed into.
/// `-c` is colabm's detailed usage query, answered from `detail.json` (or
/// with an error, like a colab release whose internals moved); anything
/// else is the launcher script being run.
const FAKE_PYTHON: &str = r#"#!/bin/sh
cfg="$HOME/.config/colab-cli"
if [ "$1" = "-c" ]; then
  case "$2" in *list_assignments*)
    if [ -f "$cfg/running" ]; then echo 1; else echo 0; fi
    exit 0 ;;
  esac
  if [ ! -f "$cfg/detail.json" ]; then
    echo "ImportError: cannot import name 'state'" >&2
    exit 1
  fi
  if [ -f "$cfg/running" ]; then cat "$cfg/detail_running.json"; else cat "$cfg/detail.json"; fi
  exit 0
fi
script="$1"; shift
exec /bin/sh "$script" "$@"
"#;

struct Env {
    home: PathBuf,
    stub: PathBuf,
}

impl Env {
    fn new(test: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let stub = root.join("colab");
        fs::write(&stub, STUB).unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        Self { home, stub }
    }

    /// Like `new`, but `colab` is a script launched by a Python interpreter,
    /// as a real installation is.
    fn with_python(test: &str) -> Self {
        let env = Self::new(test);
        let python = env.stub.with_file_name("python3");
        fs::write(&python, FAKE_PYTHON).unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
        let launcher = STUB.replacen("#!/bin/sh", &format!("#!{}", python.display()), 1);
        fs::write(&env.stub, launcher).unwrap();
        env
    }

    fn detail(&self, account: &str, json: &str) {
        fs::write(self.cfg(account).join("detail.json"), json).unwrap();
    }

    fn colabm(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_colabm"))
            .args(args)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", &self.home)
            .env("COLABM_COLAB_BIN", &self.stub)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.colabm(args);
        assert!(
            out.status.success(),
            "{args:?} failed: {}",
            text(&out.stderr)
        );
        text(&out.stdout)
    }

    fn fails(&self, args: &[&str]) -> String {
        let out = self.colabm(args);
        assert!(!out.status.success(), "{args:?} unexpectedly succeeded");
        text(&out.stderr)
    }

    fn link(&self) -> PathBuf {
        self.home.join(".config/colab-cli")
    }

    fn cfg(&self, account: &str) -> PathBuf {
        self.home
            .join(".local/share/colabm/accounts")
            .join(account)
            .join("home/.config/colab-cli")
    }

    /// Gives an account a login and a canned `colab usage` answer.
    fn sign_in(&self, account: &str, email: &str, balance: &str) {
        write_login(&self.cfg(account), email, balance);
    }
}

fn write_login(cfg: &Path, email: &str, balance: &str) {
    fs::create_dir_all(cfg).unwrap();
    let token = format!(r#"{{"token": "{FAKE_TOKEN}", "refresh_token": "{FAKE_REFRESH}"}}"#);
    fs::write(cfg.join("token.json"), token).unwrap();
    fs::write(cfg.join("email"), email).unwrap();
    let usage = format!(
        "Current balance: {balance} compute units\nUsage rate: 2.00/hr\nActive assignments: 1\n"
    );
    fs::write(cfg.join("usage.txt"), usage).unwrap();
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn first_account_becomes_active_and_switching_repoints_colab() {
    let env = Env::new("switch");
    env.ok(&["add", "work", "--no-login"]);
    env.ok(&["add", "personal", "--no-login"]);
    assert_eq!(env.ok(&["current"]).trim(), "work");
    assert_eq!(fs::read_link(env.link()).unwrap(), env.cfg("work"));

    env.ok(&["switch", "personal"]);
    assert_eq!(env.ok(&["current"]).trim(), "personal");
    assert_eq!(fs::read_link(env.link()).unwrap(), env.cfg("personal"));

    assert!(
        env.fails(&["switch", "nobody"])
            .contains("no account named 'nobody'")
    );
    assert!(
        env.fails(&["add", "work", "--no-login"])
            .contains("already exists")
    );
    assert!(
        env.fails(&["add", "../evil", "--no-login"])
            .contains("invalid account name")
    );
}

#[test]
fn account_data_is_private_to_the_owner() {
    let env = Env::new("perms");
    env.ok(&["add", "work", "--no-login"]);
    let data = env.home.join(".local/share/colabm");
    for path in [
        data.clone(),
        data.join("accounts/work"),
        data.join("state.json"),
    ] {
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{} is readable by others", path.display());
    }
}

#[test]
fn import_adopts_an_existing_login_without_losing_it() {
    let env = Env::new("import");
    write_login(&env.link(), "old@example.com", "5.00");
    env.ok(&["add", "second", "--no-login"]);

    // An unmanaged login is never overwritten by a switch.
    assert!(env.fails(&["switch", "second"]).contains("colabm import"));
    assert!(fs::symlink_metadata(env.link()).unwrap().is_dir());

    let out = env.ok(&["import", "main"]);
    assert!(out.contains("old@example.com"), "{out}");
    assert_eq!(fs::read_link(env.link()).unwrap(), env.cfg("main"));
    assert!(env.cfg("main").join("token.json").exists());
    assert_eq!(env.ok(&["current"]).trim(), "main (old@example.com)");

    env.ok(&["switch", "second"]);
    assert!(env.cfg("main").join("token.json").exists());
}

#[test]
fn limits_reports_every_account_and_remembers_the_result() {
    let env = Env::new("limits");
    for name in ["a", "b", "c"] {
        env.ok(&["add", name, "--no-login"]);
    }
    env.sign_in("a", "a@example.com", "87.50");
    env.sign_in("b", "b@example.com", "0.00");

    // One account cannot be checked, so the command reports failure overall.
    let out = env.colabm(&["limits"]);
    assert!(!out.status.success());
    let table = text(&out.stdout);
    let row = |name: &str| {
        table
            .lines()
            .find(|l| l.contains(&format!(" {name} ")))
            .unwrap()
            .to_string()
    };
    assert!(
        row("a").contains("87.50") && row("a").contains("43.8h"),
        "{table}"
    );
    assert!(row("b").contains("0.00"), "{table}");
    assert!(row("c").contains("not signed in"), "{table}");

    let json: serde_json::Value =
        serde_json::from_str(&env.ok(&["limits", "a", "--json"])).unwrap();
    assert_eq!(json[0]["usage"]["balance"], 87.5);
    assert_eq!(json[0]["usage"]["assignments"], 1);

    let listed: serde_json::Value = serde_json::from_str(&env.ok(&["list", "--json"])).unwrap();
    assert_eq!(listed[0]["usage"]["balance"], 87.5);
    assert_eq!(listed[2]["usage"], serde_json::Value::Null);
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn row(table: &str, name: &str) -> String {
    let found = table.lines().find(|l| l.contains(&format!(" {name} ")));
    found
        .unwrap_or_else(|| panic!("no row for {name} in:\n{table}"))
        .to_string()
}

#[test]
fn limits_shows_the_free_allowance_and_keeps_it_while_idle() {
    let env = Env::with_python("free");
    for name in ["busy", "idle"] {
        env.ok(&["add", name, "--no-login"]);
        env.sign_in(name, &format!("{name}@example.com"), "0.00");
    }
    let refill = now() + 5 * 3600 + 30;
    env.detail("busy", &format!(
        r#"{{"currentBalance": 0, "consumptionRateHourly": 0.5, "assignmentsCount": 1, "eligibleGpus": ["T4"], "eligibleTpus": ["V5E1"], "freeCcuQuotaInfo": {{"remainingTokens": "5500", "nextRefillTimestampSec": {refill}}}}}"#
    ));
    let idle = r#"{"currentBalance": 0, "consumptionRateHourly": 0, "assignmentsCount": 0, "eligibleGpus": ["T4"], "eligibleTpus": []}"#;
    env.detail("idle", idle);

    let table = env.ok(&["limits"]);
    let busy = row(&table, "busy");
    // 5500 thousandths of a unit, spent at 0.5/hr, lasts 11 hours.
    assert!(
        busy.contains("5.50") && busy.contains("in 5h 0m") && busy.contains("11.0h"),
        "{table}"
    );
    assert!(busy.contains("T4,V5E1"), "{table}");
    // Colab says nothing about the free allowance of an account with no runtime.
    assert!(row(&table, "idle").contains("?"), "{table}");
    assert!(
        table.contains("only while the account has a runtime running"),
        "{table}"
    );

    // The runtime stops: Colab no longer reports the allowance, colabm remembers it.
    env.detail("busy", idle);
    let json: serde_json::Value =
        serde_json::from_str(&env.ok(&["limits", "busy", "--json"])).unwrap();
    assert_eq!(json[0]["usage"]["free"]["remaining"], 5.5);
    assert_eq!(json[0]["usage"]["free"]["refill_at"], refill);
    assert_eq!(json[0]["usage"]["assignments"], 0);
    assert!(row(&env.ok(&["list"]), "busy").contains("5.50"));
}

#[test]
fn a_free_allowance_is_forgotten_once_its_refill_time_has_passed() {
    let env = Env::with_python("refilled");
    env.ok(&["add", "a", "--no-login"]);
    env.sign_in("a", "a@example.com", "0.00");
    let past = now() - 60;
    env.detail("a", &format!(
        r#"{{"currentBalance": 0, "consumptionRateHourly": 0.5, "assignmentsCount": 1, "eligibleGpus": ["T4"], "freeCcuQuotaInfo": {{"remainingTokens": "100", "nextRefillTimestampSec": {past}}}}}"#
    ));
    assert!(row(&env.ok(&["limits"]), "a").contains("?"));

    env.detail("a", r#"{"currentBalance": 0, "consumptionRateHourly": 0, "assignmentsCount": 0, "eligibleGpus": ["T4"]}"#);
    let json: serde_json::Value = serde_json::from_str(&env.ok(&["limits", "--json"])).unwrap();
    assert_eq!(json[0]["usage"]["free"], serde_json::Value::Null);
}

const IDLE: &str = r#"{"currentBalance": 0, "consumptionRateHourly": 0, "assignmentsCount": 0, "eligibleGpus": ["T4"]}"#;

fn running(tokens: i64) -> String {
    let refill = now() + 3600;
    format!(
        r#"{{"currentBalance": 0, "consumptionRateHourly": 0.08, "assignmentsCount": 1, "eligibleGpus": ["T4"], "freeCcuQuotaInfo": {{"remainingTokens": "{tokens}", "nextRefillTimestampSec": {refill}}}}}"#
    )
}

#[test]
fn probe_reads_the_free_limit_of_an_idle_account_and_stops_its_runtime() {
    let env = Env::with_python("probe");
    env.ok(&["add", "a", "--no-login"]);
    env.sign_in("a", "a@example.com", "0.00");
    env.detail("a", IDLE);
    fs::write(env.cfg("a").join("detail_running.json"), running(4200)).unwrap();

    // A plain check never starts anything.
    assert!(row(&env.ok(&["limits"]), "a").contains("?"));
    assert!(!env.cfg("a").join("running").exists());

    let table = env.ok(&["limits", "--probe"]);
    let a = row(&table, "a");
    assert!(a.contains("4.20") && a.contains(" in "), "{table}");
    // The row describes the account at rest, not the runtime that was borrowed.
    assert!(a.contains(" 0.00 ") && !a.contains("0.08"), "{table}");
    assert!(
        !env.cfg("a").join("running").exists(),
        "probe left its runtime running"
    );
    assert!(row(&env.ok(&["list"]), "a").contains("4.20"));
}

#[test]
fn probe_reports_a_runtime_it_could_not_stop() {
    let env = Env::with_python("stuck");
    env.ok(&["add", "a", "--no-login"]);
    env.sign_in("a", "a@example.com", "0.00");
    env.detail("a", IDLE);
    fs::write(env.cfg("a").join("detail_running.json"), running(4200)).unwrap();
    fs::write(env.cfg("a").join("stuck"), "").unwrap();

    let err = env.fails(&["limits", "--probe"]);
    assert!(
        err.contains("colabm run -a a stop -s colabm-probe"),
        "{err}"
    );
}

#[test]
fn an_overdrawn_free_limit_reads_as_used_up() {
    let env = Env::with_python("usedup");
    env.ok(&["add", "a", "--no-login"]);
    env.sign_in("a", "a@example.com", "0.00");
    env.detail("a", &running(-37));
    let table = env.ok(&["limits"]);
    assert!(
        row(&table, "a").contains("used up") && row(&table, "a").contains("0.0h"),
        "{table}"
    );
}

#[test]
fn limits_falls_back_to_colab_usage_when_the_detailed_query_breaks() {
    let env = Env::with_python("fallback");
    env.ok(&["add", "a", "--no-login"]);
    env.sign_in("a", "a@example.com", "42.00");

    let json: serde_json::Value = serde_json::from_str(&env.ok(&["limits", "--json"])).unwrap();
    assert_eq!(json[0]["status"], "ok");
    assert_eq!(json[0]["usage"]["balance"], 42.0);
    assert_eq!(json[0]["usage"]["accelerators"], serde_json::Value::Null);
    let table = env.ok(&["limits"]);
    assert!(
        !table.contains('?') && !table.contains("ImportError"),
        "{table}"
    );
}

#[test]
fn a_revoked_login_is_reported_as_needing_sign_in() {
    let env = Env::new("revoked");
    env.ok(&["add", "a", "--no-login"]);
    env.sign_in("a", "a@example.com", "1.00");
    fs::write(env.cfg("a").join("revoked"), "").unwrap();
    let out = env.colabm(&["limits"]);
    assert!(!out.status.success());
    assert!(
        text(&out.stdout).contains("sign-in required"),
        "{}",
        text(&out.stdout)
    );
}

#[test]
fn run_targets_another_account_without_switching() {
    let env = Env::new("run");
    env.ok(&["add", "work", "--no-login"]);
    env.ok(&["add", "personal", "--no-login"]);

    let active = env.ok(&["run", "ls", "-s", "trainer"]);
    assert_eq!(
        active.trim(),
        format!("HOME={} ARGS=ls -s trainer", env.home.display())
    );

    let other = env.ok(&["run", "-a", "personal", "sessions"]);
    let home = env
        .cfg("personal")
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf();
    assert_eq!(
        other.trim(),
        format!("HOME={} ARGS=sessions", home.display())
    );
    assert_eq!(env.ok(&["current"]).trim(), "work");
}

#[test]
fn remove_needs_confirmation_and_protects_the_active_account() {
    let env = Env::new("remove");
    env.ok(&["add", "work", "--no-login"]);
    env.ok(&["add", "old", "--no-login"]);
    env.sign_in("old", "old@example.com", "1.00");

    assert!(
        env.fails(&["remove", "work", "--yes"])
            .contains("active account")
    );
    assert!(env.fails(&["remove", "old"]).contains("--yes"));
    assert!(env.cfg("old").join("token.json").exists());

    env.ok(&["remove", "old", "--yes"]);
    assert!(!env.cfg("old").exists());
    assert!(!env.ok(&["list"]).contains("old"));

    env.ok(&["remove", "work", "--yes", "--force"]);
    assert!(fs::symlink_metadata(env.link()).is_err());
}

#[test]
fn signing_in_cannot_be_done_without_a_terminal() {
    let env = Env::new("login");
    let err = env.fails(&["add", "work"]);
    assert!(err.contains("interactive"), "{err}");
    // The account itself was still created, ready for the user to sign in.
    assert!(env.ok(&["list"]).contains("not signed in"));
}

#[test]
fn secrets_are_refused_to_agents_and_never_reach_ordinary_output() {
    let env = Env::new("secrets");
    env.ok(&["add", "work", "--no-login"]);
    env.sign_in("work", "w@example.com", "3.00");

    let refused = Command::new(env!("CARGO_BIN_EXE_colabm"))
        .args(["secrets", "work", "--reveal"])
        .env_clear()
        .env("HOME", &env.home)
        .env("CLAUDECODE", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(text(&refused.stderr).contains("AI agent"));

    let mut seen = text(&refused.stdout) + &text(&refused.stderr);
    for args in [
        &["list"][..],
        &["list", "--json"],
        &["limits"],
        &["limits", "--json"],
        &["current"],
    ] {
        let out = env.colabm(args);
        seen += &text(&out.stdout);
        seen += &text(&out.stderr);
    }
    assert!(seen.contains("w@example.com") || seen.contains("work"));
    assert!(
        !seen.contains("FAKE"),
        "a token leaked into command output:\n{seen}"
    );
}
