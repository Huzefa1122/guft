# Architecture

```
ui (React + shadcn)  ──invoke/events──▶  src-tauri (commands, sandbox)
                                              │
                                        guft-app  (App: lock, invites, chats, rooms, temp, outbox)
                              ┌───────────────┼───────────────┐
                         guft-core        guft-store       guft-net
                  (crypto, vault, invites,  (SQLCipher,     (cells, circuits,
                   payloads, rooms)         profile, lock)   Tor via Arti)
                                              guft-harden (sandbox)
```

| Crate | Job |
|---|---|
| `guft-core` | Everything cryptographic and every wire format: engine (libsignal sessions), vault (Argon2id), outer layer, invites, padding, payloads, rooms. No network, no UI, no `unsafe`. |
| `guft-store` | The encrypted chat history, the profile files (atomic saves) and the idle/password-backoff lock. |
| `guft-net` | Moves opaque encrypted frames: fixed 2048-byte cells, reassembly with hard bounds, at least 5 isolated circuits per contact, jitter, cover traffic, end-to-end acks. Tor onion services through Arti, and an in-memory network for tests. |
| `guft-app` | The application logic the UI calls. Owns the lock model, background tasks, the outbox, locked-mode spool, rooms, and the memory-only temporary store. |
| `guft-harden` | Process hardening: no dumps, no attach, Landlock, seccomp. |
| `src-tauri` | The desktop shell: a strict CSP, the command layer, the OS sandbox set-up. |
| `ui` | The interface. Talks only to the command layer. |

## Message path

1. UI calls a command; `guft-app` encrypts with the contact's session (`guft-core`), writes the message and the encrypted frame to the database, and saves the ratchet state.
2. The outbox scheduler hands the frame to `guft-net`, which splits it into cells, spreads them over the contact's circuits and waits for the acknowledgement.
3. On the other side the cells are reassembled, decrypted, stored, the state saved, and only then acknowledged.

A crash can cause a re-delivery that a `seen` table absorbs; it can never lose a message.

## Rooms

A room is a roster. Each member holds the list; a message is encrypted once per member over the existing pairwise sessions. Joining uses a room-bound invite plus introductions relayed by the inviter. There is no group key and no host.

## Temporary rooms

Kept in the engine's `temp` map (never serialised) and the app's in-memory store; wiped on lock. The wire invite carries the room kind so the other side also keeps it in memory.

## Lock model

Unlocked means the master key and decrypted engine are in memory. `Locker::lock()` saves, closes the database and drops everything. Background work never extends the idle timer.

More detail for contributors is in `AGENTS.md` in the repository.
