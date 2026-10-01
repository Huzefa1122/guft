# nochat — agent guide

Serverless, Tor-based, post-quantum end-to-end encrypted chat. Tauri 2 desktop app (Rust core + React/shadcn UI). License: AGPL-3.0-only (required by libsignal).

## Layout
- `crates/nochat-core` — crypto, vault, invites, padding, payloads. No networking, no UI. `#![forbid(unsafe_code)]`.
- `crates/nochat-store` — SQLCipher chat history, `Profile` (create/unlock/save/lock), `Locker` (idle auto-lock, wrong-password backoff). One Argon2id run -> master key -> HKDF subkeys (`state`, `history`). Files are 0600, dir 0700; nothing readable on disk.
- `crates/nochat-net` — (planned) Arti onion service + client, ≥5 isolated parallel circuits per contact, cover traffic.
- `src-tauri` — (planned) Tauri shell, OS sandbox, IPC commands.
- `ui` — (planned) Vite + React + Tailwind + shadcn/ui, strict CSP, no remote assets.

## Security rules (do not weaken)
- Never write custom cryptographic primitives or protocols. Message crypto = `libsignal-protocol` (PQXDH ML-KEM-1024 + SPQR). The extra outer layer (`outer.rs`) only composes audited RustCrypto primitives.
- Pin libsignal to an exact git tag; review every bump (no stable API from upstream).
- Validate all network input against `limits.rs` BEFORE allocating or parsing. File ≤ 200,000 bytes, text ≤ 8 KiB.
- File names from peers go through `payload::check_file_name`; never join them to a path unchecked.
- Secrets use `Zeroizing`; never log keys, plaintext, codes, onion keys or invites.
- Tor path length is not customised (it fingerprints users). Fixed guard, random middles, isolated circuits, fixed-size padding, cover traffic, vanguards.
- UI: strict CSP, no `eval`, no remote scripts/fonts, no `dangerouslySetInnerHTML`. Treat every peer-supplied string as untrusted text.
- Invites are single-use, time-limited, and need the separate one-time code. Burn an invite only after a first message authenticates.

## Commands
- `cargo test --workspace` — all tests
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo audit` / `cargo deny check` — dependencies and licenses (must stay AGPL-compatible)
- ASAN build (nightly): `RUSTFLAGS=-Zsanitizer=address cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu`

## Conventions
- Match surrounding style; small modules; tests beside the code. Errors never contain secrets or plaintext.
- Git is local only for now; do not push or create remotes without being asked.

## Lock model
Unlocked = master key + decrypted engine in memory. `Locker::lock()` saves, closes the DB and drops (zeroizes) everything: manual button, idle timeout, window close. While locked the onion service is down (nothing can be decrypted).
