# Tunnel

A macOS menu-bar app that supervises the b2c2 AWS SSM tunnels
(primary/secondary/options Redis + options config Postgres) and keeps them
alive, re-authenticating when your AWS SSO session expires.

The menu-bar icon is color-coded so you can see the state at a glance:

| Icon | State |
|------|-------|
| 🟢 green | Running — read-only (replica) |
| 🔵 blue | Running — write mode (primary) |
| ⚪️ gray | Stopped |
| 🟠 amber | Starting / logging in / reconnecting |
| 🔴 red | Needs AWS login, or an error |

When the SSO session expires the app detects the dropped tunnels, posts a
macOS notification, re-runs `b2c2 aws login` (opening a browser in your GUI
session), and rebuilds the tunnels with backoff. It runs on a supervised
worker thread and never crashes on a failed tunnel — failures become a
visible state instead.

## Prerequisites

- **Rust** (stable): https://rustup.rs
- **The `b2c2` CLI** on your `PATH` (the app also reads your login-shell
  `PATH`, so wherever your shell finds `b2c2` works).
- **`redis-cli`** and **`psql`** for the health checks
  (`brew install redis libpq`).
- macOS 11+ (Apple Silicon or Intel).

## Build & install

```sh
git clone <this repo>
cd tunnel
make install
```

`make install` builds a release binary, assembles `Tunnel.app`, copies it to
`~/Applications` (per-user, so no admin rights are needed — works on managed
Macs), and launches it. Look for the tunnel-arch icon in your menu bar.

To install system-wide instead:

```sh
sudo make install INSTALL_DIR=/Applications
```

Other targets:

```sh
make build      # build + assemble ./dist/Tunnel.app (no install)
make run        # build + run from ./dist (for testing)
make uninstall  # quit, remove the login item, delete ~/Applications/Tunnel.app
make help       # list everything
```

## Usage

Click the menu-bar icon:

- **Start** — bring the tunnels up. The first time (or after SSO expiry) this
  opens a browser to complete `b2c2 aws login`.
- **Stop** — tear the tunnels down.
- **Write mode (primary)** — target the primary DBs instead of the replicas.
  Off = replica (read-only), the safe default. As a safety guard, write mode
  **auto-reverts to read-only after 30 minutes** (you get a notification and
  the checkbox unticks); re-check it to extend. The 30-minute clock survives
  reconnects, so a dropped tunnel won't silently extend write access.
- **Start at login** — register the app as a macOS login item so it launches
  automatically (it starts idle; press Start to bring up tunnels — or leave
  it and Start yourself).
- **Open logs** — opens `~/Library/Logs/Tunnel/tunnel.log`.
- **Quit** — stop tunnels and exit the app entirely.

To restart after Quit, launch it again: `open ~/Applications/Tunnel.app`, or
find "Tunnel" in Spotlight / `~/Applications`. If **Start at login** is
checked it also comes back automatically at your next login.

### First-launch permissions

- Toggling **Start at login** asks for permission to control **System
  Events** (that's how the login item is added). Allow it.
- Because the app is only ad-hoc signed, on some machines the first launch may
  need **Right-click → Open** (or approval in System Settings → Privacy &
  Security).

## Repo layout

- `src/main.rs` — menu-bar UI (tray icon, menu, event loop), PATH fixup,
  login-item management.
- `src/worker.rs` — the supervised tunnel worker (start/stop, health checks,
  auto re-auth, backoff).
- `assets/gen-icons.sh` — regenerates the committed icons (needs
  `brew install librsvg`); run via `make icons`.
- `bundle.sh` — assembles `Tunnel.app`.
- `Info.plist` — bundle metadata (`LSUIElement` = menu-bar only, no Dock icon).
