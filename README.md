# colabm

Manage several Google accounts for the Colab CLI (`google-colab-cli`, the
`colab` command): switch between them, check each one's compute-unit limits, and keep
their tokens out of an AI agent's view.

For a step-by-step walkthrough, see [USER_MANUAL.md](USER_MANUAL.md).

## Install

Pick one. Each builds the `colabm` binary from source, so Rust (`cargo`) must
be installed.

```bash
uv tool install git+https://github.com/BectorVoom/colabm            # with uv, like the Colab CLI itself
cargo install --git https://github.com/BectorVoom/colabm --locked   # with cargo
cargo install --path .                                              # from a local clone
```

Requires the `colab` command (`uv tool install google-colab-cli`) on Linux or macOS.

## Quick start

```bash
colabm import main            # adopt the login you already have in ~/.config/colab-cli
colabm add work               # register a second account and sign in to it
colabm limits                 # paid balance, free allowance, usage rate, VMs for every account
colabm switch work            # plain `colab` now acts as `work`
colab new --gpu T4            # ...as usual
colabm run -a main sessions   # one-off command as another account, without switching
```

## Commands

| Command | What it does |
| --- | --- |
| `colabm add NAME [--no-login]` | Register an account and sign in (Google's copy-paste code flow). |
| `colabm import NAME` | Turn the existing `~/.config/colab-cli` login into an account. |
| `colabm login NAME` | Sign in again, e.g. after the login was revoked. |
| `colabm list [--json]` | Accounts, which one is active, and the last limits seen. |
| `colabm switch NAME` | Make `NAME` the account plain `colab` uses. |
| `colabm current` | Print the active account. |
| `colabm limits [NAME...] [--probe] [--json]` | Check paid and free limits of all (or the named) accounts in parallel. |
| `colabm run [-a NAME] ARGS...` | Run `colab ARGS...` as `NAME` (default: the active account). |
| `colabm remove NAME [--yes] [--force]` | Delete an account's login, session records and history. |
| `colabm secrets [NAME] [--reveal]` | Show tokens on your terminal. People only. |
| `colabm passphrase` | Set or change the passphrase that guards `secrets`. |

`limits` shows, per account: paid compute-unit balance, free-tier allowance
and when it refills, hourly burn rate, an estimate of the hours left, running
VMs, and the accelerators Colab will currently grant. It exits non-zero if any
account could not be checked.

Colab reports the free allowance only while the account has a runtime
running. colabm remembers the last figure it saw and keeps showing it (with
its age) until the refill time; an account it has never seen running shows
`?`. `colabm limits --probe` fills those in: on each idle account it starts a
CPU runtime for a few seconds, reads the allowance, stops the runtime and
confirms with Colab that it is gone. That spends a sliver of the allowance,
which is why it is opt-in.

The free allowance and accelerator list are not printed by `colab usage`, so
colabm reads them through the colab CLI's own Python client. If a future
colab release breaks that, `limits` falls back to `colab usage` and shows the
paid figures only.

## How accounts are kept apart

The colab CLI hard-codes its files under `~/.config/colab-cli` (login,
session records, history, logs). colabm gives every account its own copy:

```
~/.local/share/colabm/            (owner-only; override with $COLABM_DIR)
  state.json                      account names, emails, last limits, passphrase hash
  accounts/NAME/home/.config/colab-cli/
```

`~/.config/colab-cli` becomes a symlink to the active account's directory, so
switching is one atomic rename and each account keeps its own sessions and
history. Commands for a non-active account (`limits`, `run -a`) run `colab`
with `HOME` set to that account's `home/`; `~/.ssh` is linked in so
`colab ssh` still finds your keys, but other dotfiles (for example your
editor's config under `colab edit`) are not visible there.

colabm never replaces a `~/.config/colab-cli` it did not create: an existing
login has to be adopted with `import` first.

## What an AI agent can and cannot see

An agent such as Claude Code may run colabm for you. The split is:

- **Open to anyone running the CLI:** `list`, `current`, `limits`, `switch`,
  `run`. Their output carries account names, emails and compute-unit numbers,
  never tokens. Error text relayed from `colab` is scrubbed of token-shaped
  strings.
- **People only:** `secrets`, `passphrase`, and signing in (`add`, `login`).
  - `secrets` refuses when it detects an agent's shell (for example
    `CLAUDECODE` is set) or when no terminal is attached.
  - It then asks for a passphrase that only you know (stored as an Argon2id
    hash), read from the terminal with echo off.
  - Tokens are written to the terminal device, not to stdout, so redirecting
    or capturing the command's output yields nothing. They are masked unless
    you pass `--reveal`.
  - Signing in needs you to paste Google's authorization code at a terminal.

### The limit of that guarantee

The colab CLI needs each login as a plain `token.json` file, so the tokens
exist unencrypted under `~/.local/share/colabm/` (as they already do under
`~/.config/colab-cli` without colabm). colabm keeps them out of its own output
and makes the directory owner-only, but it cannot stop a process running as
you from opening those files. Close that gap in the agent's own permission
settings. For Claude Code, in `~/.claude/settings.json`:

```json
{
  "permissions": {
    "deny": [
      "Read(~/.local/share/colabm/**)",
      "Read(~/.config/colab-cli/**)"
    ]
  }
}
```

Deny rules cover Claude Code's file tools. To also block shell commands such
as `cat` from reading those paths, enable its sandbox and restrict reads
there (see Claude Code's sandboxing documentation).

## Development

```bash
cargo test    # unit tests plus end-to-end tests against a stand-in `colab`
```

`COLABM_COLAB_BIN` points colabm at a different `colab` executable.
