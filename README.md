# slaunch

A fast application launcher for Wayland (think `rofi`/`wofi`). It runs as a
background daemon that keeps a GTK4 popup window and a fuzzy-search index
warm in memory, so `slaunch show` pops the window up instantly instead of
starting a fresh process each time.

For architecture and internals (threading model, plugin system, GTK
gotchas), see [CLAUDE.md](CLAUDE.md) once you're past this file.

<img width="665" height="344" alt="image" src="https://github.com/user-attachments/assets/d380d3a5-368e-46fb-afc5-13e655c8a033" />


## Prerequisites

- A Wayland compositor with `wlr-layer-shell` support (Sway, Hyprland,
  river, ... — **not** GNOME).
- Rust (stable) via [rustup](https://rustup.rs/).
- GTK4 + `gtk4-layer-shell` dev libraries (e.g. on Arch: `gtk4
  gtk4-layer-shell`). If `cargo build` fails with a linker error mentioning
  `gtk4`/`gtk_layer_shell`, it's a missing system package, not a Rust issue.

## Getting started

```sh
cargo check      # confirms everything compiles
make install     # builds --release, installs to ~/.local/bin,
                  # seeds ~/.config/slaunch/ only if it doesn't exist yet
```

Run the daemon (keep this terminal open so you can see the logs):

```sh
slaunch daemon
```

In another terminal:

```sh
slaunch show     # pops the launcher up
```

Type to search, `Enter` to launch, `Escape` to hide. Once that works, bind
`slaunch show` to a key in your compositor config (e.g. Sway/Hyprland:
`bindsym $mod+d exec slaunch show`) and start `slaunch daemon` on login
(your compositor's `exec`/`exec-once` config directive).

## Using it

| Input | Result |
|---|---|
| `firefox` | Installed apps, fuzzy-matched |
| `htop` | Anything on `$PATH` |
| `=2 + 2 * 6` | Inline math, answer copied to clipboard |
| `:fire` | Emoji search by name, copied to clipboard |
| `shutdown`, `lock` | Power/session actions |

`Up`/`Down` or `Tab`/`Shift+Tab` moves the selection.

## CLI

| Command | Does |
|---|---|
| `slaunch daemon` | Runs the long-lived process — everything else below needs this running first |
| `slaunch show` / `hide` | Show or hide the window |
| `slaunch reload` | Re-reads config + rescans apps without restarting the daemon |
| `slaunch kill` | Shuts the daemon down |

Add `RUST_LOG=slaunch=debug` before `slaunch daemon` for verbose logs.

## Configuration

Edit `~/.config/slaunch/config.toml` and `~/.config/slaunch/style.css` —
both apply live via `slaunch reload`, no restart needed.
[config/config.toml](config/config.toml) in this repo is the shipped
example with every option commented; that's the fastest reference for what
you can change. A partial config is fine — anything you don't set falls
back to a default.

## Development

```sh
cargo build           # debug build
cargo build --release # what `make install` uses
cargo test              # unit tests
cargo clippy             # lint
cargo fmt                 # format
```

The GTK UI (`src/ui/mod.rs`) has no automated tests — after UI changes,
verify by hand with `cargo run -- daemon` in one terminal and `cargo run --
show` in another.

## Troubleshooting

- **`slaunch show` errors out** — no daemon running; start one with
  `slaunch daemon`.
- **Window never appears** — your compositor probably doesn't support
  `wlr-layer-shell`.
- **Config changes don't seem to apply** — run `slaunch reload`, and make
  sure you edited `~/.config/slaunch/config.toml`, not the copy under
  `config/` in this repo (that one's only used to seed a fresh install).
