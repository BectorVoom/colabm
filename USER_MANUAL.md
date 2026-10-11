# colabm User Manual

`colabm` lets you use several Google accounts with the Colab CLI (`colab`).
With it you can:

- keep a separate login for each account,
- switch which account `colab` uses,
- check the paid and free compute-unit limits of every account at once,
- look at your tokens yourself, while an AI agent (such as Claude Code) cannot.

Follow the steps in order the first time. After that, use the
[command summary](#command-summary) at the end.

---

## Before you start

You need:

1. Linux or macOS.
2. The Colab CLI. Check that it is installed:

   ```bash
   colab version
   ```

   If the command is not found, install it:

   ```bash
   uv tool install google-colab-cli
   ```

3. Rust (`cargo`), because `colabm` is built on your computer when you
   install it. Check with `cargo --version`. If it is missing, install Rust
   from <https://rustup.rs>.

---

## Step 1. Install colabm

1. Install with **one** of these commands.

   With `uv` (the same tool that installs the Colab CLI):

   ```bash
   uv tool install git+https://github.com/BectorVoom/colabm
   ```

   Or with `cargo`:

   ```bash
   cargo install --git https://github.com/BectorVoom/colabm --locked
   ```

   Or, if you have downloaded this project, from inside its folder:

   ```bash
   cargo install --path .
   ```

2. Wait for the build to finish. The first build can take a minute.
3. Check that it works:

   ```bash
   colabm --help
   ```

   You should see the list of commands.

To update later, run `uv tool upgrade colabm`, or run the same `cargo install`
command again.

---

## Step 2. Bring in the login you already have

Do this step if you have used `colab` before on this computer. If you have
never signed in with `colab`, skip to Step 3.

1. Pick a short name for that account, for example `main`.
   Names may use letters, digits, `.`, `_` and `-`.
2. Run:

   ```bash
   colabm import main
   ```

3. You should see:

   ```
   Imported the existing login as 'main' (you@gmail.com); it stays active.
   ```

What happened: your `~/.config/colab-cli` folder was moved into colabm's
storage, and a link was left in its place. `colab` keeps working exactly as
before. Nothing was deleted.

---

## Step 3. Add another account

Do this in your own terminal. Signing in cannot be done by an AI agent.

1. Pick a name for the new account, for example `work`.
2. Run:

   ```bash
   colabm add work
   ```

3. The Colab CLI prints a Google link:

   ```
   To authorize colab-cli, visit this URL in any browser:

     https://accounts.google.com/o/oauth2/auth?...

   After approving, Google will display an authorization code.
   Enter the authorization code:
   ```

4. Open the link in a browser.
5. Choose the Google account you want to use for `work` and approve.
6. Google shows an authorization code. Copy it.
7. Paste the code into the terminal and press Enter.
8. You should see the account's balance and then:

   ```
   'work' is signed in as work@gmail.com.
   ```

Repeat this step for every account you want to add.

Notes:

- The first account you ever create becomes the active one automatically.
- To create an account now and sign in later, use
  `colabm add work --no-login`, and later `colabm login work`.

---

## Step 4. See your accounts

1. Run:

   ```bash
   colabm list
   ```

2. Example output:

   ```
      ACCOUNT  EMAIL           STATUS     PAID  FREE  REFILL     RATE/HR  LEFT   VMS  ACCEL    CHECKED
   *  main     you@gmail.com   signed in  0.00  1.57  in 9h 9m   0.08     19.6h  1    T4,V5E1  3m ago
      work     work@gmail.com  signed in  -     -     -          -        -      -    -        -
   ```

How to read it:

| Column | Meaning |
| --- | --- |
| `*` | The active account: the one plain `colab` uses now. |
| `ACCOUNT` | The name you chose. |
| `EMAIL` | The Google address of that login. |
| `STATUS` | `signed in` or `not signed in`. |
| `PAID` | Paid compute units left. `0.00` for a free account. |
| `FREE` | Free compute units left until the next refill. `used up` means none are left; `?` means unknown. See [About the free limit](#about-the-free-limit). |
| `REFILL` | Time until the free limit is refilled. |
| `RATE/HR` | Compute units being used per hour. |
| `LEFT` | Hours until `PAID` plus `FREE` run out at that rate. `-` when nothing is running. |
| `VMS` | Number of running Colab runtimes. |
| `ACCEL` | GPUs/TPUs Colab will give this account right now. `none` means no accelerator is available to it. |
| `CHECKED` | When those numbers were last fetched. `-` means never. |

`list` does not contact Google. It shows the numbers saved by the last
`colabm limits` (Step 5).

To see only the active account:

```bash
colabm current
```

---

## Step 5. Check the limits of each account

1. To check every account:

   ```bash
   colabm limits
   ```

2. Example output:

   ```
      ACCOUNT  EMAIL            STATUS  PAID   FREE           REFILL     RATE/HR  LEFT   VMS  ACCEL    CHECKED
   *  main     you@gmail.com    ok      0.00   1.57           in 9h 12m  0.08     19.6h  1    T4,V5E1  just now
      work     work@gmail.com   ok      0.00   5.50 (2h ago)  in 3h 40m  0.00     -      0    T4,V5E1  just now
      third    third@gmail.com  ok      0.00   ?              -          0.00     -      0    T4,V5E1  just now

   FREE '?': Colab reports the free limit only while the account has a runtime running.
   `colabm limits --probe` starts one for a few seconds to read it.
   ```

   In this example:

   - `main` has a runtime running, so its free limit is live: 1.57 compute
     units left, about 19.6 hours at the current rate, refilled in 9h 12m.
   - `work` has nothing running now. colabm shows the value it saw 2 hours
     ago, when a runtime was running.
   - `third` has never been checked while a runtime was running, so its free
     limit is unknown (`?`).

3. To also read the free limit of accounts that show `?`, add `--probe`:

   ```bash
   colabm limits --probe
   ```

   ```
   Starting a temporary CPU runtime on third to read the free limit; it is stopped right after.
      ACCOUNT  EMAIL            STATUS  PAID   FREE     REFILL      RATE/HR  LEFT   VMS  ACCEL    CHECKED
   *  main     you@gmail.com    ok      0.00   1.57     in 9h 12m   0.08     19.6h  1    T4,V5E1  just now
      work     work@gmail.com   ok      0.00   used up  in 3h 40m   0.00     -      0    T4,V5E1  just now
      third    third@gmail.com  ok      0.00   6.00     in 23h 59m  0.00     -      0    T4,V5E1  just now
   ```

   `used up` means the free limit is spent until the refill time.

4. To check only some accounts, name them:

   ```bash
   colabm limits work
   colabm limits work --probe
   ```

The `STATUS` column tells you if a check worked:

| Status | Meaning | What to do |
| --- | --- | --- |
| `ok` | The numbers are fresh. | Nothing. |
| `not signed in` | The account has no login. | `colabm login NAME` |
| `sign-in required` | Google rejected the login (expired or revoked). | `colabm login NAME` |
| `timed out waiting for colab` | No answer within 45 seconds. | Check your network and try again. |
| other text | The error reported by `colab`. | Read the message; try `colabm run -a NAME usage`. |

### About the free limit

Colab tells the free limit **only while the account has a runtime running**.
This is how Colab's service answers; colabm cannot change it.

- **Runtime running:** `FREE` and `REFILL` are live.
- **No runtime running, but checked earlier today:** `FREE` shows the last
  value colabm saw, with its age, for example `5.50 (2h ago)`. An idle
  account uses nothing, so the value is still right, unless you also used
  Colab in the browser in the meantime.
- **After the refill time has passed, or never checked while running:**
  `FREE` shows `?`.

To read the free limit of accounts that show `?`, use `--probe`:

```bash
colabm limits --probe
```

What `--probe` does, for each account that has no runtime running:

1. Starts a CPU runtime named `colabm-probe` on the account.
2. Reads the free limit.
3. Stops the runtime.
4. Asks Colab for the account's list of runtimes, to confirm it is gone.

This takes about 10 seconds. Accounts that already have a runtime running
are not touched; their free limit is read directly.

Things to know before using `--probe`:

- It uses a very small amount of the free limit (a CPU runtime for a few
  seconds).
- If the runtime could not be confirmed as stopped, colabm prints a warning
  with the exact command to stop it, and exits with an error:

  ```bash
  colabm run -a NAME stop -s colabm-probe
  ```

- Without `--probe`, `colabm limits` never starts anything.

The value stays visible in `colabm list` and `colabm limits` until the
refill time.

For a few minutes after a runtime stops, Colab still reports it: `VMS` and
`RATE/HR` keep the old values (about 5 minutes in one test). `colabm run -a
NAME sessions` shows what is really running.

You can also use the `ACCEL` column without starting anything: it lists the
accelerators Colab is willing to give the account right now.

### Good to know

- colabm reads the free limit and accelerator list through the Colab CLI's
  own program code, because `colab usage` does not print them. If a new
  Colab CLI version changes that code, colabm still shows `PAID`, `RATE/HR`
  and `VMS`, and shows `-` for `FREE` and `ACCEL`.
- For scripts, add `--json`. The command exits with an error code if any
  account could not be checked.

  ```bash
  colabm limits --json
  ```

  ```json
  [
    {
      "name": "main",
      "active": true,
      "email": "you@gmail.com",
      "status": "ok",
      "usage": {
        "balance": 0.0,
        "rate_hourly": 0.08,
        "assignments": 1,
        "checked_at": 1791683543,
        "accelerators": ["T4", "V5E1"],
        "free": {
          "remaining": 1.57,
          "refill_at": 1791719222,
          "checked_at": 1791683543
        }
      }
    }
  ]
  ```

  `balance` is the paid balance. `free` is `null` when the free limit is
  unknown; `remaining` at zero or below means it is used up; `refill_at` and
  `checked_at` are Unix times in seconds.

---

## Step 6. Switch accounts

1. Run:

   ```bash
   colabm switch work
   ```

2. You should see:

   ```
   Switched to 'work' (work@gmail.com).
   ```

3. From now on, every plain `colab` command uses `work`:

   ```bash
   colab new --gpu T4
   colab sessions
   ```

4. Switch back the same way:

   ```bash
   colabm switch main
   ```

Good to know:

- Each account keeps its own sessions and history. After switching,
  `colab sessions` shows only the new account's sessions.
- Switching does not stop anything. Runtimes of the other account keep
  running and keep using compute units until you stop them (see Step 7).

---

## Step 7. Run one command as another account

Use this when you do not want to switch.

1. Put the account after `-a`, then the `colab` command as usual:

   ```bash
   colabm run -a main sessions
   colabm run -a main stop -s trainer
   colabm run -a work new --gpu T4
   ```

2. Without `-a`, the active account is used. This is the same as plain `colab`:

   ```bash
   colabm run sessions
   ```

Good to know: a command for a non-active account runs with that account's own
home folder. Your SSH keys are still found (for `colab ssh`), but other
personal settings, such as your editor's configuration in `colab edit`, are
not. If you need those, switch to the account instead.

---

## Step 8. Look at your security information (tokens)

Only a person at a terminal can do this. An AI agent cannot.

### 8.1 First time: choose a passphrase

1. Open a normal terminal yourself (not one started by an AI agent).
2. Run:

   ```bash
   colabm secrets
   ```

3. colabm asks you to choose a passphrase (8 characters or more) and to
   repeat it. Nothing appears while you type; that is normal.

   ```
   No passphrase yet. Choose one; colabm asks for it before showing secrets.
   New passphrase:
   Repeat passphrase:
   ```

4. Remember this passphrase. Do not give it to an AI agent.

### 8.2 View the tokens

1. Run, for the active account:

   ```bash
   colabm secrets
   ```

   or for a named account:

   ```bash
   colabm secrets work
   ```

2. Type your passphrase.
3. The values are shown shortened, so you can recognise them safely:

   ```
   Account 'work' (work@gmail.com)
     Access token    ya29…6789 (220 chars)
     Refresh token   1//0…Xk2Q (103 chars)
     Client secret   d-FL…4uQg (24 chars)
     Client ID       1234567890-abc.apps.googleusercontent.com
     Access expiry   2026-10-11T12:00:00Z
     Scope           openid
     Scope           https://www.googleapis.com/auth/colaboratory
     Session 'trainer' (gpu-t4-s-abc123)
       Proxy token   eyJh…9f3A (512 chars)
       Token expiry  2026-10-11T11:00:00Z
     Stored in       /home/you/.local/share/colabm/accounts/work/home/.config/colab-cli
   Values are masked; add --reveal to print them in full.
   ```

4. To see the full values, add `--reveal`. Make sure nobody can see your
   screen, and that the terminal is not being recorded:

   ```bash
   colabm secrets work --reveal
   ```

### 8.3 Change the passphrase

1. Run:

   ```bash
   colabm passphrase
   ```

2. Type the current passphrase.
3. Type the new passphrase twice.

---

## Step 9. Remove an account

This deletes the account's login, session records and history from this
computer. It does not delete your Google account, and it does not stop
running runtimes.

1. Stop the account's runtimes first, so they do not keep using compute units:

   ```bash
   colabm run -a work sessions
   colabm run -a work stop -s SESSION_NAME
   ```

2. If the account is active, switch to another one:

   ```bash
   colabm switch main
   ```

3. Remove it:

   ```bash
   colabm remove work
   ```

4. Answer `y` to confirm.

Options:

- `--yes` skips the question (needed in scripts).
- `--force` allows removing the active account. Afterwards no account is
  active until you run `colabm switch`.

---

## Using colabm together with an AI agent

You can let an agent such as Claude Code run `colab` and `colabm` for you.

| The agent can | The agent cannot |
| --- | --- |
| `colabm list`, `current`, `limits` (also `--probe`) | `colabm secrets` (refused) |
| `colabm switch NAME` | `colabm passphrase` (refused) |
| `colabm run ...` and plain `colab ...` | sign in: `colabm add` / `login` need you to paste Google's code |
| `colabm remove NAME --yes` | |

What these commands print never includes tokens: only account names, emails
and compute-unit numbers.

Note that an agent can run `colabm remove NAME --yes`. Tell it not to, if you
do not want that.

### One more step to protect the token files

The Colab CLI needs the tokens stored as normal files on disk. colabm does not
print them, but a program running as you could still open the files directly.
To stop Claude Code from reading them:

1. Open `~/.claude/settings.json`.
2. Add these rules (merge with any `permissions` you already have):

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

3. These rules cover Claude Code's file tools. To also block shell commands
   such as `cat`, turn on Claude Code's sandbox and restrict reading of the
   same folders there (see Claude Code's sandboxing documentation).

---

## Troubleshooting

| Message | Cause | Fix |
| --- | --- | --- |
| `the colab command was not found` | The Colab CLI is not installed or not on `PATH`. | `uv tool install google-colab-cli` |
| `signing in is interactive ...` | `add` or `login` was run without a terminal (for example by an agent). | Run `colabm login NAME` yourself in a terminal. |
| `sign-in did not complete for 'NAME'` | The code was wrong, expired, or you cancelled. | Run `colabm login NAME` again and paste a fresh code. |
| `... holds a login colabm does not manage. Adopt it first with colabm import` | You had a `colab` login before installing colabm. | `colabm import NAME` (Step 2), then switch. |
| `nothing to import: ... does not exist` | There is no earlier login. | Use `colabm add NAME` instead. |
| `moving ... (both must be on one filesystem)` | `~/.config` and colabm's storage are on different disks. | Set `COLABM_DIR` to a folder on the same disk as `~/.config`, then import again. |
| `no account named 'NAME'` | Typo, or the account was never added. | `colabm list` to see the names. |
| `account 'NAME' already exists` | The name is taken. | Choose another name, or `colabm login NAME`. |
| `'NAME' is the active account; switch to another first, or pass --force` | You tried to remove the active account. | `colabm switch OTHER`, then remove. |
| `refused: this shell belongs to an AI agent (... is set)` | `secrets` was run inside an agent's shell, including a command you typed inside the agent's window. | Open a normal terminal and run it there. |
| `refused: no terminal attached` | `secrets` was run from a script or a pipe. | Run it directly in a terminal. |
| `wrong passphrase` | The passphrase did not match. | Try again. If you forgot it, see below. |
| `No active account` | Nothing is selected. | `colabm switch NAME` |

### I forgot my passphrase

The passphrase guards the `secrets` command; it does not encrypt your logins,
so nothing is lost.

1. Open `~/.local/share/colabm/state.json` in an editor.
2. Delete the whole `"passphrase": { ... },` block and save.
3. Run `colabm secrets`. It asks you to choose a new passphrase.

### I signed in with the wrong Google account

1. `colabm remove NAME`
2. `colabm add NAME`, and choose the correct Google account in the browser.

### I want to stop using colabm

1. Switch to the account you want to keep: `colabm switch NAME`.
2. Remove the link and move that account's folder back:

   ```bash
   rm ~/.config/colab-cli
   mv ~/.local/share/colabm/accounts/NAME/home/.config/colab-cli ~/.config/colab-cli
   ```

3. `colab` now works on its own again with that account. The other accounts
   are still under `~/.local/share/colabm/` until you delete that folder.

---

## Command summary

| Command | What it does |
| --- | --- |
| `colabm add NAME [--no-login]` | Add an account and sign in. |
| `colabm import NAME` | Adopt the existing `colab` login as an account. |
| `colabm login NAME` | Sign in to an account again. |
| `colabm list [--json]` | Show accounts and the last limits seen. Alias: `ls`. |
| `colabm current` | Show the active account. |
| `colabm limits [NAME...] [--probe] [--json]` | Check paid and free limits now. `--probe` also reads the free limit of idle accounts. Alias: `usage`. |
| `colabm switch NAME` | Change the active account. Alias: `use`. |
| `colabm run [-a NAME] ARGS...` | Run `colab ARGS...` as an account. |
| `colabm remove NAME [--yes] [--force]` | Delete an account from this computer. Alias: `rm`. |
| `colabm secrets [NAME] [--reveal]` | Show tokens on your terminal (people only). |
| `colabm passphrase` | Set or change the passphrase. |
| `colabm help [COMMAND]` | Show help. |

## Where things are stored

| Path | Contents |
| --- | --- |
| `~/.local/share/colabm/state.json` | Account names, emails, last limits, passphrase hash. No tokens. |
| `~/.local/share/colabm/accounts/NAME/home/.config/colab-cli/` | That account's login, sessions, history and logs, as written by `colab`. |
| `~/.config/colab-cli` | A link to the active account's folder. |

## Settings (environment variables)

| Variable | Effect |
| --- | --- |
| `COLABM_DIR` | Store colabm's data in this folder instead of `~/.local/share/colabm`. |
| `COLABM_COLAB_BIN` | Use this `colab` executable instead of the one on `PATH`. |
