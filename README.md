# guft

Serverless, Tor-based, post-quantum end-to-end encrypted chat: text and files (up to 1 MB), one-to-one or private rooms. Tauri 2 desktop app (Rust core + React/shadcn UI).

Work in progress; not independently audited. Do not rely on it for safety yet.

Licensed under AGPL-3.0-only. Built on Signal's `libsignal` (not affiliated with or endorsed by Signal).

## Why guft exists

guft is not meant to replace WhatsApp, Signal or anything else. Most people should keep using the chat apps they already use.

It is for the people who want something more private than that: two people (or a small group) who trust each other, but who do not want a company, a server, a phone number or an account anywhere in the middle. Nobody can be handed your messages, because nobody holds them, and each room can have its own identity so your conversations cannot be tied together.

It is deliberately small. Text, small files and voice notes between people who have exchanged an invite, over Tor, with the privacy settings already on.

## What it does

- **No servers and no accounts.** Each profile is a Tor v3 onion service hosted by the app itself. Contacts reach each other directly over Tor.
- **Post-quantum end-to-end encryption.** Messages use libsignal's PQXDH handshake (ML-KEM-1024 + X25519) and the SPQR post-quantum ratchet, wrapped in a second layer keyed by the invite secret and the separate one-time code.
- **Temporary chats.** One button in any 1:1 chat opens a chat that lives in memory only (on both devices); groups use a room created as "Temporary". Nothing is written to the history database, the outbox or the saved state, and it is wiped when guft locks, idles out or closes.
- **A different identity in every room.** Make or join a room as a separate identity: a new name, new keys and a new Tor address with nothing linking it to you elsewhere, erased when you leave.
- **Private rooms up to 25 people.** A full mesh over the pairwise sessions: no group key and no host. A room-bound invite makes the newcomer a member and the inviter introduces them to everyone else automatically.
- **Traffic hygiene.** Fixed 2048-byte cells over at least 5 isolated Tor circuits per contact, size-bucket padding, cover traffic and vanguards.
- **Encrypted at rest.** Chat history lives in a SQLCipher database keyed from your passphrase (Argon2id). The vault relocks on idle, on the lock button, and on window close.
- **Voice notes.** Hold a conversation with up to a minute of audio (Opus, about 24 kbit/s, roughly 180 kB). Recording stays local until you send it, and it travels as an ordinary encrypted file. Needs a microphone; a received note is only decoded when you press play.
- **Everyday chat.** Delivery ticks, unread counts, copy or delete a message (an unsent one is cancelled), rename a contact (only you see it), clear a chat, verify safety numbers.
- **Limits**: text ≤ 8 KiB, files ≤ 1 MB.

## Documentation

Install guides, how-tos, the security model and troubleshooting are in the [wiki](https://github.com/Huzefa1122/guft/wiki) (sources in `docs/wiki`). Prebuilt Linux binaries are on the [Releases](https://github.com/Huzefa1122/guft/releases) page.

## Invites

There is no directory. To talk to someone you need two things, shared over different channels:

1. an **invite** (safe to paste anywhere), and
2. a **one-time code** (give it by voice or in person).

The invite is single-use and expires. Using both boots the encrypted session; the invite is burned only after the first message authenticates. The same flow adds someone straight into a room.

## Build and run

Prerequisites: Rust stable (≥ 1.85), Node 20+ with pnpm, and the usual Tauri/WebKitGTK development packages on Linux.

```sh
cargo build -p guft-desktop     # binary: target/debug/guft
cd ui && pnpm install && pnpm build
```

For UI work in a plain browser, `cd ui && pnpm dev` serves a mock backend (passphrase `demo-passphrase`); it is never part of release builds.

## Tests

```sh
cargo test --workspace                      # offline, includes the in-memory network
cargo clippy --workspace --all-targets -- -D warnings
cargo audit                                 # RustSec advisories
cargo test -p guft-net --test tor_live -- --ignored --nocapture   # real Tor, slow
cd ui && pnpm typecheck && pnpm build
```

## Security

See [SECURITY.md](SECURITY.md) for the threat model and how to report a problem. Short version: message contents are protected end to end, metadata and endpoints are not, and the code has not been audited.
