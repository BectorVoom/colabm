mod colab;
mod gate;
mod paths;
mod state;

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde::Serialize;

use paths::{Link, Paths};
use state::{Account, State, Usage};

const COLAB_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Parser)]
#[command(
    name = "colabm",
    version,
    about = "Manage several Google Colab CLI accounts"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Register a new account and sign in to it
    Add {
        name: String,
        /// Create the account without signing in yet
        #[arg(long)]
        no_login: bool,
    },
    /// Adopt the login already in ~/.config/colab-cli as an account
    Import { name: String },
    /// Sign in to an account (interactive)
    Login { name: String },
    /// List accounts with their last known limits
    #[command(alias = "ls")]
    List {
        #[arg(long)]
        json: bool,
    },
    /// Make an account the one plain `colab` uses
    #[command(alias = "use")]
    Switch { name: String },
    /// Print the active account
    Current,
    /// Check compute-unit balance and usage live (all accounts by default)
    #[command(alias = "usage")]
    Limits {
        names: Vec<String>,
        #[arg(long)]
        json: bool,
        /// Read the free limit of idle accounts by starting a CPU runtime
        /// on each for a few seconds (this uses a little of the limit)
        #[arg(long)]
        probe: bool,
    },
    /// Run a colab command as an account without switching to it
    Run {
        /// Account to run as (default: the active one)
        #[arg(short, long)]
        account: Option<String>,
        /// Arguments passed to colab, e.g. `sessions` or `new --gpu T4`
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Delete an account's stored login, session records and history
    #[command(alias = "rm")]
    Remove {
        name: String,
        /// Do not ask for confirmation
        #[arg(short, long)]
        yes: bool,
        /// Allow removing the active account
        #[arg(long)]
        force: bool,
    },
    /// Show an account's tokens on your terminal (people only, never agents)
    Secrets {
        /// Account to show (default: the active one)
        name: Option<String>,
        /// Print full values instead of masked ones
        #[arg(long)]
        reveal: bool,
    },
    /// Set or change the passphrase that guards `secrets`
    Passphrase,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("colabm: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let paths = Paths::discover()?;
    let mut state = State::load(&paths)?;
    match cli.cmd {
        Cmd::Add { name, no_login } => add(&paths, &mut state, &name, no_login)?,
        Cmd::Import { name } => import(&paths, &mut state, &name)?,
        Cmd::Login { name } => login(&paths, &mut state, &name)?,
        Cmd::List { json } => list(&paths, &state, json)?,
        Cmd::Switch { name } => switch(&paths, &state, &name)?,
        Cmd::Current => return current(&paths, &state),
        Cmd::Limits { names, json, probe } => {
            return limits(&paths, &mut state, &names, json, probe);
        }
        Cmd::Run { account, args } => run_colab(&paths, &state, account, &args)?,
        Cmd::Remove { name, yes, force } => remove(&paths, &mut state, &name, yes, force)?,
        Cmd::Secrets { name, reveal } => secrets(&paths, &mut state, name, reveal)?,
        Cmd::Passphrase => passphrase(&paths, &mut state)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn active(paths: &Paths) -> Result<Option<String>> {
    Ok(match paths.link_state()? {
        Link::Account(name) => Some(name),
        _ => None,
    })
}

fn require(state: &State, name: &str) -> Result<()> {
    if !state.accounts.contains_key(name) {
        bail!("no account named '{name}' (see `colabm list`)");
    }
    Ok(())
}

fn new_account(state: &State, name: &str) -> Result<()> {
    paths::validate_name(name)?;
    if state.accounts.contains_key(name) {
        bail!("account '{name}' already exists");
    }
    Ok(())
}

fn add(paths: &Paths, state: &mut State, name: &str, no_login: bool) -> Result<()> {
    new_account(state, name)?;
    paths.create_account(name, true)?;
    let account = Account {
        email: None,
        added_at: state::now(),
        usage: None,
    };
    state.accounts.insert(name.to_string(), account);
    state.save(paths)?;
    println!("Added account '{name}'.");
    match paths.link_state()? {
        Link::Missing => {
            paths.point_link(name)?;
            println!("'{name}' is now the active account.");
        }
        Link::RealDir => eprintln!(
            "note: {} holds a login colabm does not manage yet; \
             `colabm import <name>` adopts it so you can switch away from it.",
            paths.link().display()
        ),
        _ => {}
    }
    if no_login {
        println!("Sign in with `colabm login {name}`.");
        return Ok(());
    }
    login(paths, state, name)
}

fn import(paths: &Paths, state: &mut State, name: &str) -> Result<()> {
    new_account(state, name)?;
    let link = paths.link();
    match paths.link_state()? {
        Link::RealDir => {}
        Link::Missing => bail!(
            "nothing to import: {} does not exist; use `colabm add`",
            link.display()
        ),
        Link::Account(current) => {
            bail!("{} already belongs to account '{current}'", link.display())
        }
        Link::Foreign => bail!("{} is not a directory colabm can adopt", link.display()),
    }
    let cfg = paths.account_cfg(name);
    if cfg.exists() {
        bail!("{} already exists", cfg.display());
    }
    paths.create_account(name, false)?;
    fs::rename(&link, &cfg).with_context(|| {
        format!(
            "moving {} to {} (both must be on one filesystem)",
            link.display(),
            cfg.display()
        )
    })?;
    if let Err(e) = paths.point_link(name) {
        fs::rename(&cfg, &link).context("restoring the original directory")?;
        return Err(e);
    }
    let account = Account {
        email: None,
        added_at: state::now(),
        usage: None,
    };
    state.accounts.insert(name.to_string(), account);
    state.save(paths)?;
    match identify(paths, state, name) {
        Some(email) => {
            println!("Imported the existing login as '{name}' ({email}); it stays active.")
        }
        None => println!("Imported the existing login as '{name}'; it stays active."),
    }
    state.save(paths)
}

fn login(paths: &Paths, state: &mut State, name: &str) -> Result<()> {
    require(state, name)?;
    if !io::stdin().is_terminal() {
        bail!(
            "signing in is interactive: Google shows an authorization code that you paste back. \
             Run `colabm login {name}` yourself in a terminal."
        );
    }
    // Any authenticated call makes the colab CLI run its sign-in flow when
    // the login is missing or no longer valid; `usage` is the cheapest.
    let status = colab::for_account(paths, name)
        .arg("usage")
        .status()
        .map_err(colab::spawn_error)?;
    if !status.success() || !colab::signed_in(&paths.account_cfg(name)) {
        bail!("sign-in did not complete for '{name}'");
    }
    match identify(paths, state, name) {
        Some(email) => println!("'{name}' is signed in as {email}."),
        None => println!("'{name}' is signed in."),
    }
    state.save(paths)
}

/// Asks Google which address the account's login belongs to and records it.
fn identify(paths: &Paths, state: &mut State, name: &str) -> Option<String> {
    if !colab::signed_in(&paths.account_cfg(name)) {
        return None;
    }
    let mut cmd = colab::for_account(paths, name);
    cmd.arg("whoami");
    let out = colab::finish(
        colab::spawn_captured(cmd).ok()?,
        Instant::now() + COLAB_TIMEOUT,
    )
    .ok()?;
    let email = colab::parse_email(&out.stdout).filter(|_| out.ok)?;
    state.accounts.get_mut(name)?.email = Some(email.clone());
    Some(email)
}

#[derive(Serialize)]
struct Row {
    name: String,
    active: bool,
    email: Option<String>,
    status: String,
    usage: Option<Usage>,
}

fn list(paths: &Paths, state: &State, json: bool) -> Result<()> {
    let active = active(paths)?;
    let rows: Vec<Row> = state
        .accounts
        .iter()
        .map(|(name, account)| Row {
            name: name.clone(),
            active: active.as_deref() == Some(name),
            email: account.email.clone(),
            status: if colab::signed_in(&paths.account_cfg(name)) {
                "signed in".into()
            } else {
                "not signed in".into()
            },
            usage: account.usage.clone(),
        })
        .collect();
    print_rows(&rows, json)?;
    if !json && paths.link_state()? == Link::RealDir {
        println!(
            "\n{} holds a login colabm does not manage; `colabm import <name>` adopts it.",
            paths.link().display()
        );
    }
    Ok(())
}

fn limits(
    paths: &Paths,
    state: &mut State,
    names: &[String],
    json: bool,
    probe: bool,
) -> Result<ExitCode> {
    for name in names {
        require(state, name)?;
    }
    let targets: Vec<String> = if names.is_empty() {
        state.accounts.keys().cloned().collect()
    } else {
        names.to_vec()
    };
    let active = active(paths)?;

    // Start every check before waiting on any, so accounts are queried at once.
    let mut checks = Vec::new();
    for name in &targets {
        let signed_in = colab::signed_in(&paths.account_cfg(name));
        checks.push(if signed_in {
            Some(colab::start_check(paths, name)?)
        } else {
            None
        });
    }
    let now = state::now();
    let deadline = Instant::now() + COLAB_TIMEOUT;
    let mut rows = Vec::new();
    for (name, check) in targets.iter().zip(checks) {
        let (status, mut usage) = match check {
            None => ("not signed in".to_string(), None),
            Some(check) => {
                match colab::finish_check(paths, name, check, deadline, COLAB_TIMEOUT)? {
                    Ok(usage) => ("ok".to_string(), Some(usage)),
                    Err(reason) => (reason, None),
                }
            }
        };
        let account = state.accounts.get_mut(name).context("account vanished")?;
        if let Some(usage) = &mut usage {
            // Colab omits the free allowance while nothing is running, and
            // an idle account spends none, so the last figure seen still
            // holds until the refill.
            if usage.free.is_none() {
                let last = account.usage.take().and_then(|old| old.free);
                usage.free = last.filter(|free| free.refill_at > now);
            }
            account.usage = Some(usage.clone());
        }
        rows.push(Row {
            name: name.clone(),
            active: active.as_deref() == Some(name),
            email: account.email.clone(),
            status,
            usage,
        });
    }
    let left_running = if probe {
        probe_idle(paths, state, &mut rows)
    } else {
        Vec::new()
    };
    state.save(paths)?;
    print_rows(&rows, json)?;
    for problem in &left_running {
        eprintln!("warning: {problem:#}");
    }
    let all_ok = left_running.is_empty() && rows.iter().all(|row| row.usage.is_some());
    Ok(if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Fills in the free allowance of accounts that have nothing running, which
/// Colab only reveals once a runtime exists. Returns the failures that may
/// have left a runtime behind.
fn probe_idle(paths: &Paths, state: &mut State, rows: &mut [Row]) -> Vec<anyhow::Error> {
    // Without the accelerator list the detailed query is unavailable, and a
    // probe could not read the allowance either.
    let idle = |row: &&mut Row| {
        let usage = row.usage.as_ref();
        usage.is_some_and(|u| u.assignments == 0 && u.accelerators.is_some())
    };
    let idle: Vec<&mut Row> = rows.iter_mut().filter(idle).collect();
    if idle.is_empty() {
        return Vec::new();
    }
    let names: Vec<&str> = idle.iter().map(|row| row.name.as_str()).collect();
    eprintln!(
        "Starting a temporary CPU runtime on {} to read the free limit; it is stopped right after.",
        names.join(", ")
    );
    let results: Vec<_> = thread::scope(|scope| {
        let probes: Vec<_> = names
            .iter()
            .map(|&name| scope.spawn(move || colab::probe(paths, name, COLAB_TIMEOUT)))
            .collect();
        probes
            .into_iter()
            .map(|probe| probe.join().expect("probe thread panicked"))
            .collect()
    });
    let mut left_running = Vec::new();
    for (row, result) in idle.into_iter().zip(results) {
        match result {
            Ok(Ok(free)) => {
                if let Some(account) = state.accounts.get_mut(&row.name)
                    && let Some(saved) = &mut account.usage
                {
                    saved.free = Some(free.clone());
                }
                if let Some(usage) = &mut row.usage {
                    usage.free = Some(free);
                }
            }
            Ok(Err(reason)) => {
                eprintln!(
                    "note: could not read the free limit of '{}': {reason}",
                    row.name
                );
            }
            Err(problem) => left_running.push(problem),
        }
    }
    left_running
}

fn print_rows(rows: &[Row], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(rows)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("No accounts yet. Start with `colabm add <name>` or `colabm import <name>`.");
        return Ok(());
    }
    let header = [
        "", "ACCOUNT", "EMAIL", "STATUS", "PAID", "FREE", "REFILL", "RATE/HR", "LEFT", "VMS",
        "ACCEL", "CHECKED",
    ];
    let now = state::now();
    let dash = || "-".to_string();
    let mut table = vec![header.map(String::from).to_vec()];
    let mut free_unknown = false;
    for row in rows {
        let usage = row.usage.as_ref();
        let free = usage
            .and_then(|u| u.free.as_ref())
            .filter(|f| f.refill_at > now);
        let free_cell = match (usage, free) {
            // Colab lets the allowance run slightly below zero.
            (_, Some(f)) if f.remaining <= 0.0 => "used up".to_string(),
            (Some(u), Some(f)) if f.checked_at < u.checked_at => {
                format!("{:.2} ({})", f.remaining, ago(f.checked_at))
            }
            (_, Some(f)) => format!("{:.2}", f.remaining),
            (Some(u), None) if u.accelerators.is_some() => {
                free_unknown = true;
                "?".to_string()
            }
            _ => dash(),
        };
        let spendable =
            usage.map_or(0.0, |u| u.balance) + free.map_or(0.0, |f| f.remaining.max(0.0));
        table.push(vec![
            if row.active {
                "*".into()
            } else {
                String::new()
            },
            row.name.clone(),
            row.email.clone().unwrap_or_else(dash),
            row.status.clone(),
            usage.map_or_else(dash, |u| format!("{:.2}", u.balance)),
            free_cell,
            free.map_or_else(dash, |f| until(f.refill_at - now)),
            usage.map_or_else(dash, |u| format!("{:.2}", u.rate_hourly)),
            usage
                .filter(|u| u.rate_hourly > 0.0)
                .map_or_else(dash, |u| format!("{:.1}h", spendable / u.rate_hourly)),
            usage.map_or_else(dash, |u| u.assignments.to_string()),
            match usage.and_then(|u| u.accelerators.as_ref()) {
                Some(list) if list.is_empty() => "none".to_string(),
                Some(list) => list.join(","),
                None => dash(),
            },
            usage.map_or_else(dash, |u| ago(u.checked_at)),
        ]);
    }
    let widths: Vec<usize> = (0..header.len())
        .map(|i| {
            table
                .iter()
                .map(|r| r[i].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for cells in &table {
        let line: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, &width)| format!("{cell:<width$}"))
            .collect();
        println!("{}", line.join("  ").trim_end());
    }
    if free_unknown {
        println!(
            "\nFREE '?': Colab reports the free limit only while the account has a runtime running.\n\
             `colabm limits --probe` starts one for a few seconds to read it."
        );
    }
    Ok(())
}

fn until(seconds: u64) -> String {
    match (seconds / 3600, seconds % 3600 / 60) {
        (0, minutes) => format!("in {minutes}m"),
        (hours, minutes) => format!("in {hours}h {minutes}m"),
    }
}

fn ago(then: u64) -> String {
    match state::now().saturating_sub(then) {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86400),
    }
}

fn switch(paths: &Paths, state: &State, name: &str) -> Result<()> {
    require(state, name)?;
    let link = paths.link();
    match paths.link_state()? {
        Link::RealDir => bail!(
            "{} holds a login colabm does not manage. Adopt it first with \
             `colabm import <name>`, then switch.",
            link.display()
        ),
        Link::Foreign => bail!(
            "{} is not something colabm created; leaving it alone",
            link.display()
        ),
        Link::Account(current) if current == name => {
            println!("'{name}' is already active.");
            return Ok(());
        }
        Link::Account(_) | Link::Missing => {}
    }
    paths.point_link(name)?;
    match &state.accounts[name].email {
        Some(email) => println!("Switched to '{name}' ({email})."),
        None => println!("Switched to '{name}'."),
    }
    if !colab::signed_in(&paths.account_cfg(name)) {
        eprintln!("note: '{name}' is not signed in yet; run `colabm login {name}`.");
    }
    Ok(())
}

fn current(paths: &Paths, state: &State) -> Result<ExitCode> {
    match paths.link_state()? {
        Link::Account(name) => {
            match state.accounts.get(&name).and_then(|a| a.email.as_ref()) {
                Some(email) => println!("{name} ({email})"),
                None => println!("{name}"),
            }
            return Ok(ExitCode::SUCCESS);
        }
        Link::RealDir => eprintln!(
            "No active account: {} holds a login colabm does not manage (`colabm import <name>`).",
            paths.link().display()
        ),
        _ => eprintln!("No active account (`colabm switch <name>`)."),
    }
    Ok(ExitCode::FAILURE)
}

fn run_colab(paths: &Paths, state: &State, account: Option<String>, args: &[String]) -> Result<()> {
    let active = active(paths)?;
    let name = account
        .or_else(|| active.clone())
        .context("no active account: pass `-a <name>` or run `colabm switch <name>`")?;
    require(state, &name)?;
    // The active account is what plain colab already uses, so leave its
    // environment alone; any other account gets its own home.
    let mut cmd = if active.as_deref() == Some(name.as_str()) {
        colab::plain()
    } else {
        colab::for_account(paths, &name)
    };
    Err(colab::spawn_error(cmd.args(args).exec()))
}

fn remove(paths: &Paths, state: &mut State, name: &str, yes: bool, force: bool) -> Result<()> {
    require(state, name)?;
    let is_active = active(paths)?.as_deref() == Some(name);
    if is_active && !force {
        bail!("'{name}' is the active account; switch to another first, or pass --force");
    }
    let sessions = read_json(&paths.account_cfg(name).join("sessions.json"))
        .and_then(|v| v.as_object().map(|o| o.len()))
        .unwrap_or(0);
    if sessions > 0 {
        eprintln!(
            "warning: '{name}' has {sessions} session record(s). Removing the account does not \
             stop running VMs; stop them first with `colabm run -a {name} stop`."
        );
    }
    if !yes {
        if !io::stdin().is_terminal() {
            bail!("removing '{name}' deletes its login and history; pass --yes to confirm");
        }
        print!("Delete account '{name}', its login, session records and history? [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            bail!("aborted");
        }
    }
    if is_active {
        fs::remove_file(paths.link())?;
    }
    let dir = paths.account_dir(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("deleting {}", dir.display()))?;
    }
    state.accounts.remove(name);
    state.save(paths)?;
    println!("Removed account '{name}'.");
    Ok(())
}

fn secrets(paths: &Paths, state: &mut State, name: Option<String>, reveal: bool) -> Result<()> {
    let name = match name {
        Some(name) => name,
        None => active(paths)?.context("no active account: name the account to show")?,
    };
    require(state, &name)?;
    let (mut tty, _) = gate::human(paths, state)?;
    let cfg = paths.account_cfg(&name);
    let show = |value: &str| match value {
        "" => "(none)".to_string(),
        v if reveal => v.to_string(),
        v => mask(v),
    };

    let email = state.accounts[&name]
        .email
        .as_deref()
        .unwrap_or("email unknown");
    writeln!(tty, "Account '{name}' ({email})")?;
    match read_json(&cfg.join("token.json")) {
        Some(token) => {
            let text = |key: &str| token[key].as_str().unwrap_or("").to_string();
            writeln!(tty, "  Access token    {}", show(&text("token")))?;
            writeln!(tty, "  Refresh token   {}", show(&text("refresh_token")))?;
            writeln!(tty, "  Client secret   {}", show(&text("client_secret")))?;
            writeln!(tty, "  Client ID       {}", text("client_id"))?;
            writeln!(tty, "  Access expiry   {}", text("expiry"))?;
            for scope in token["scopes"].as_array().into_iter().flatten() {
                writeln!(tty, "  Scope           {}", scope.as_str().unwrap_or(""))?;
            }
        }
        None => writeln!(tty, "  Not signed in.")?,
    }
    let sessions = read_json(&cfg.join("sessions.json"));
    for (session, info) in sessions.iter().filter_map(|v| v.as_object()).flatten() {
        let text = |key: &str| info[key].as_str().unwrap_or("").to_string();
        writeln!(tty, "  Session '{session}' ({})", text("endpoint"))?;
        writeln!(tty, "    Proxy token   {}", show(&text("token")))?;
        writeln!(tty, "    Token expiry  {}", text("token_expires_at"))?;
    }
    writeln!(tty, "  Stored in       {}", cfg.display())?;
    if !reveal {
        writeln!(
            tty,
            "Values are masked; add --reveal to print them in full."
        )?;
    }
    Ok(())
}

fn passphrase(paths: &Paths, state: &mut State) -> Result<()> {
    let (mut tty, created) = gate::human(paths, state)?;
    if !created {
        state.passphrase = Some(gate::choose()?);
        state.save(paths)?;
    }
    writeln!(tty, "Passphrase saved.")?;
    Ok(())
}

fn read_json(file: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&fs::read_to_string(file).ok()?).ok()
}

/// Enough of a secret to recognise it, not enough to use it.
fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let count = chars.len();
    if count < 16 {
        return format!("•••• ({count} chars)");
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[count - 4..].iter().collect();
    format!("{head}…{tail} ({count} chars)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking_hides_the_usable_part() {
        assert_eq!(
            mask("ya29.abcdefghijklmnopqrstuvwxyz"),
            "ya29…wxyz (31 chars)"
        );
        assert_eq!(mask("short"), "•••• (5 chars)");
    }
}
