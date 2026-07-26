# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`slaunch` is a Wayland application launcher (rofi/wofi-style) built as a persistent background daemon with a GTK4 + layer-shell popup UI. Single Rust binary (`src/main.rs`) that acts as both the daemon and its own IPC client depending on the subcommand.

## Versioning

Bump `version` in `Cargo.toml` (which also updates `Cargo.lock`) any time a change adds a new feature — a new plugin, subcommand, config option, or other user-facing capability. Pre-1.0, bump the minor number for a feature (`0.1.0` -> `0.2.0`) and the patch number for bug fixes/refactors with no new capability (`0.1.0` -> `0.1.1`). Do this as part of the same commit that introduces the feature.

## Commands

```sh
cargo build --release      # release build (opt-level 3, lto, strip — matches install target)
cargo build                # debug build
cargo run -- daemon        # run the daemon in the foreground (Ctrl-C to stop; logs via tracing)
cargo check                # fast typecheck
cargo test                 # run the unit test suite
cargo clippy                # lint
cargo fmt                  # format

sudo make install           # builds --release, installs to $PREFIX/bin (default /usr/local/bin), seeds ~/.config/slaunch/ if absent
make install PREFIX=~/.local  # rootless alternative
sudo make uninstall

RUST_LOG=slaunch=debug cargo run -- daemon   # verbose logging (default filter is "slaunch=info")
```

There is no CI config in this repo. Unit tests (`#[cfg(test)] mod tests` blocks in-file, run via `cargo test`) cover the pure/testable logic: config parsing, defaults and range clamping, `Searcher`'s priority-bucketing and async settling, `.desktop`/PATH scanning and dedup, locale/`OnlyShowIn`/`TryExec` handling, the math/emoji query plugins, IPC command encoding, and `scan_entries`' priority assignment. `UiState` holds no GTK types, so the parts of `src/ui/mod.rs` that don't touch widgets (the power-action confirmation logic) are tested directly; everything involving a widget or a keypress is not, since it needs a live display. Validate those by exercising the daemon (`cargo run -- daemon` in one terminal, `cargo run -- show` in another).

For UI work, `grim -o <output>` gives you a screenshot to check against — it's the only way to verify geometry, anchoring and row rendering, and it caught a `reload` bug that logs alone showed as succeeding. Driving *keystrokes* needs `wtype` or `ydotool`, neither of which is assumed present; keyboard-path changes are hand-verified.

An isolated daemon can be run alongside the user's own without disturbing it: point `XDG_RUNTIME_DIR` at a scratch dir (symlink the real `wayland-*` socket into it) and `XDG_CONFIG_HOME` at a scratch config. Keep the runtime path short — the socket path has to fit `SUN_LEN` (108 bytes).

Tests that touch process-global env vars (HOME, PATH, XDG_*) serialize on a shared lock in `src/test_env.rs` (`#[cfg(test)]`-only module) since `cargo test` runs tests in parallel threads within one process and env vars are otherwise a shared-mutation hazard across them. Follow the same pattern (`test_env::lock()` + `EnvVarGuard`) for any new test that needs to set one of these vars.

Once a daemon is running, `slaunch show|hide|reload|kill` drive it over the IPC socket. `reload` re-reads config and rescans entries without restarting the process — prefer it over kill+relaunch when iterating on `config/config.toml` or `config/style.css`.

## Architecture

### Process model: one binary, two roles

`main.rs` dispatches on the CLI subcommand (clap). `Commands::Daemon` runs the full daemon; every other subcommand (`show`/`hide`/`reload`/`kill`) is a thin client that opens the Unix socket, writes one command byte, reads one response byte, and exits (`src/client.rs`, `src/ipc.rs`). The socket lives at `$XDG_RUNTIME_DIR/slaunch.sock` (falls back to `/run/user/<uid>` via `libc::getuid()` if the env var is unset), 0600 permissions.

Wire protocol is deliberately minimal — single command byte in, single status byte out (`RESP_OK`/`RESP_ERR`). `daemon::run_socket` probes an existing socket path by attempting to connect before removing it, so a stale socket file left by a crashed daemon doesn't collide with a live one.

### Daemon threading model

Three execution contexts coexist in the daemon, bridged by channels — this is the trickiest part of the codebase to modify safely:

1. **Main thread** — GTK4 event loop (`app.run_with_args` in `ui::run`). GTK/glib types are `!Send`; all widget mutation must happen here.
2. **Background thread** — a multi-thread tokio runtime running `daemon::run_socket`, which accepts IPC connections and also owns the `notify` filesystem watcher (`daemon::spawn_watcher`) that debounces (500ms) app/PATH directory changes into rescans.
3. **Bridge thread** — spawned inside `ui::run`, holds a single-thread tokio runtime whose only job is forwarding `tokio::mpsc::Receiver<DaemonEvent>` into an `async_channel::bounded` sender, because `glib::MainContext::spawn_local` needs a channel type that's `Send` to move across threads but whose receiver can be polled without `Send` on the main loop. (`glib::MainContext::channel` was removed upstream; this two-hop relay is the current replacement pattern — see `src/ui/mod.rs` top-of-file comments.)

`DaemonState` (`config: RwLock<Config>`, `entries: RwLock<Vec<Entry>>`) is the only state shared across threads (`Arc`-wrapped). UI-local state (`UiState`: query, selection, live search results) lives in an `Rc<RefCell<_>>` on the main thread only — never shared across the thread boundary.

**Critical invariant:** the daemon's only window must never be GTK-*destroyed*, only hidden (`set_visible(false)`). `GtkApplication` keeps `app.run()` alive solely by owning ≥1 window; losing the last window silently ends the process. The compositor can request destruction on its own (output hotplug, monitor sleep, layer-surface dismissal), so `window.connect_close_request` intercepts that and returns `glib::Propagation::Stop` instead of letting the default handler destroy the toplevel. If you add any new code path that closes the window, it must call `set_visible(false)`, never let a `close`/`destroy` request through. The only intended process exit is `app.quit()` on `DaemonEvent::Quit`.

### Plugin system — two different dispatch paths

`Plugin` (`src/plugins/mod.rs`) has two independent methods and plugins use one or the other, not both:

- **`scan(&self, out)`** — populates the persistent, searchable entry list once at startup and on rescan/reload. Implemented by `AppsPlugin` (parses `.desktop` files under XDG data dirs — honouring localised `Name[xx]`, `OnlyShowIn`/`NotShowIn` against `$XDG_CURRENT_DESKTOP`, and `TryExec` — dedup'd by filename with `$XDG_DATA_HOME` taking priority), `CommandsPlugin` (walks `$PATH`, dedup'd by executable name, executability via `access(X_OK)`) and `PowerPlugin` (four fixed session actions from `plugins.power_commands`). These three are wired into `daemon::scan_entries` — enabling/disabling and result ordering is config-driven (`plugins.priority`, `plugins.apps`, `plugins.commands`, `plugins.power`).
- **`query(&self, input, out)`** — computed fresh per keystroke from the raw query string, bypassing the entry list and the fuzzy matcher entirely. `MathPlugin` (triggers on a leading `=`, evaluates via `evalexpr`) and `EmojiPlugin` (triggers on a leading `:`, substring-matches emoji names/shortcodes) work this way. They are **not** registered in `daemon::scan_entries` at all — the prefix dispatch is hardcoded in `UiState::refresh_results` (`src/ui/mod.rs`), which checks the query prefix and routes to `MathPlugin`/`EmojiPlugin` directly instead of `Searcher`.

Implication: adding a new `scan`-based plugin means implementing `Plugin` + registering a name in `daemon::scan_entries`'s match arm + adding it to `config.plugins.priority`. Adding a new prefix-triggered plugin (like math/emoji) means adding a branch in `UiState::refresh_results` — there's no generic registry for that path yet.

`Entry::kind: EntryKind` tags how to launch a result; `UiState::launch_at` matches on `EntryKind` to pick which plugin's `launch()` to call, since results from different plugins are interleaved in one flat `Vec<Entry>`.

### Search (`src/search.rs`)

`Searcher` wraps a `nucleo::Nucleo<usize>` (SIMD fuzzy matcher) indexed over entry positions, not entries directly (avoids cloning the whole list into the matcher). The indexed haystack is `Entry::search_text()` — the display name plus `Entry::keywords`, which is how an app is findable by its `.desktop` `Keywords=`/`GenericName` without those terms showing in the row. `keywords` is deliberately *not* `description`: for commands the description is the `$PATH` directory, so indexing it would make every one of ~2300 command entries match "usr" or "bin".

**Matching is asynchronous.** `tick()` returns a `Status` whose `running` flag means the worker threads haven't finished, so `results()` can legitimately return a *partial* set. The `notify` callback passed to `Searcher::new` is the only signal that more has arrived; `ui::build_ui` relays it onto the glib main loop, which calls `UiState::resync_results`. Any new call path that reads `results()` and renders it must either be reachable from that relay or accept showing stale results — dropping the callback silently reintroduces truncated result lists. `results(limit)` does a priority-aware fill: it buckets matches by `Entry::priority` (set from `plugins.priority`'s index during `scan_entries`) and fills the result list from the highest-priority bucket first, so a low-priority plugin's high fuzzy score can't crowd out a higher-priority plugin's results. Nucleo's own ranking only determines order *within* a bucket.

### Config (`src/config.rs`)

TOML at `$XDG_CONFIG_HOME/slaunch/config.toml` (default `~/.config/slaunch/config.toml`); `#[serde(default)]` on every struct means partial configs are valid — unset fields fall back to `Default` impls, not an error. `config/config.toml` in the repo is the shipped example, installed by `make install` only if the user has no existing config. `reload` re-runs `config::load()` + `scan_entries()` from scratch and pushes both into `DaemonState` and the UI via `DaemonEvent::ReloadConfig`.

`Widgets::apply_config` is the single place `[window]`/`[input]` settings are applied, called both at build time and on `reload` — anything added there must go through it, or it will silently only take effect on daemon restart (which is exactly the bug `reload` used to have). `window.monitor` is honoured there: `"focused"` deliberately leaves the monitor unset so the compositor picks the active output, since a layer-shell client can't query which output has focus.

Styling is separate from `config.toml`: `style.css` (same XDG dir) is raw CSS loaded at `STYLE_PROVIDER_PRIORITY_USER`, scoped under `window.slaunch { ... }`, and reapplied live on `reload` — it overrides the system GTK theme rather than extending it.

### Keyboard handling

`EventControllerKey` is attached to the *window* with `PropagationPhase::Capture`, not to the search `Entry` widget — this is what lets Up/Down/Tab/Escape/Enter get intercepted before GTK's `Entry` consumes them for cursor movement/text editing. Shift+Tab arrives as `ISO_Left_Tab`, not `Tab` + a modifier flag; both `Down`/`Tab` and `Up`/`ISO_Left_Tab` are handled as equivalent pairs.
