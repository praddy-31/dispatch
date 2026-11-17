# Contributing

Thanks for helping. Dispatch is a Rust workspace: `crates/core` (the library `dispatch`, plus
the `dispatch-cli` tool) holds everything that can be tested without a window, and `app/` is
a Tauri 2 shell around it whose window is plain HTML, CSS and JavaScript with no build step.

## Build and test

You need an Apple silicon Mac with Rust (rustup), the Xcode Command Line Tools
(`xcode-select --install`) and, for the app, the Tauri CLI:
`cargo install tauri-cli --version '^2'`. No Node.

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                      # saved feed files, no network
cd app && cargo tauri dev                   # the app (uses your real data folder)
cd app && cargo tauri build                 # Dispatch.app and the .dmg
```

Run fmt, clippy and the tests before you commit; CI runs the same on every pull request.

## Use a throwaway data folder

Never test against your real data. Point Dispatch at a temporary folder:

```sh
DISPATCH_HOME=/tmp/dispatch-test cargo run -p dispatch-core --bin dispatch-cli -- refresh
DISPATCH_HOME=/tmp/dispatch-test cargo run -p dispatch-core --bin dispatch-cli -- status
DISPATCH_HOME=/tmp/dispatch-test ./target/debug/dispatch --autostart   # the app, no window
```

Only one copy of the app runs at a time (single instance), so quit an installed Dispatch
before running a development build.

## Ground rules

- **Privacy:** no telemetry, no analytics. A new network request needs a reason, a line in
  the README's Privacy section and in `docs/ARCHITECTURE.md` → Network, and must go through
  `fetch::Http` so the network log counts it.
- **Be polite to arXiv:** at most one API request every 3 seconds, a User-Agent, RSS once per
  daily slot, and an hour's pause after HTTP 429. Don't run `dispatch-cli backfill` in a loop.
- **Feeds are data, never code:** feed text reaches the page only through `textContent`;
  KaTeX runs with `trust: false`, and only on research titles and abstracts.
- **Never lose the user's work:** writes are atomic (temp file + rename) or one SQLite
  transaction; a failed or empty fetch never removes items; files outside the data folder
  are written only where the user chose, and never over a file Dispatch didn't write.
- **Secrets** (API keys) go in the macOS Keychain, never in a file or the database.
- **Tests use made-up data:** feed fixtures in `crates/core/tests/fixtures/` are invented
  (names, titles, ids, hosts on `example.org`). Don't commit copies of real feeds.
- Every keyboard shortcut is one entry in `keys.js` `ACTIONS`.
- Keep it light. Justify any new dependency in your pull request.

## Commits

Small commits with a clear imperative message, for example `Keep a feed's discussion link`.
Explain decisions (a library choice, a format change) in the pull request description.

## Guides

- [Architecture](docs/ARCHITECTURE.md): layers, modules, data on disk, the refresh flow and
  every host Dispatch may contact.

## Licence

By contributing you agree that your contributions are licensed under the [MIT licence](LICENSE).
