# Building From Source

## Requirements

- Rust stable (1.85 or newer)
- Node 20+ and `pnpm`
- Linux development packages for Tauri: `webkit2gtk-4.1`, `gtk3`, `libsoup3`, a C compiler and `pkg-config` (on Debian-based systems: `libwebkit2gtk-4.1-dev build-essential libssl-dev`)

## Build and run (development)

```sh
git clone https://github.com/Huzefa1122/guft.git
cd guft
(cd ui && pnpm install && pnpm build)
cargo run -p guft-desktop
```

For UI work in a plain browser with a mock backend (passphrase `demo-passphrase`):

```sh
cd ui && pnpm dev
```

## Release build

```sh
(cd ui && pnpm build)
cargo build --release --locked -p guft-desktop
```

The release profile uses LTO, a single codegen unit, `panic=abort`, overflow checks and stripped symbols. `.cargo/config.toml` adds full RELRO and a non-executable stack. To keep your home directory and checkout path out of the binary, add `--remap-path-prefix` flags (see [[Releasing]]).

## Tests and checks

```sh
./ci.sh
```

runs the whole workspace test suite, `clippy -D warnings`, `cargo audit`, `cargo deny check`, and the UI typecheck and build. The live Tor tests are separate because they need the internet and take minutes:

```sh
cargo test -p guft-net --test tor_live  -- --ignored --nocapture
cargo test -p guft-net --test tor_large -- --ignored --nocapture
```

## Rules for contributors

- No custom cryptography; use `libsignal` and audited crates.
- Validate every network input against `crates/guft-core/src/limits.rs` before allocating.
- Never log keys, plaintext, codes, onion keys or invites.
- Any code that stores a room message or room control frame must check whether the room is temporary.
- Use the latest dependency versions and run `cargo audit` before a release.
- Match the style of the surrounding code and add tests next to what you change.

Licence: AGPL-3.0-only (required by libsignal).
