# Security Model

How guft protects you, layer by layer. For what it does *not* protect, read [[Threat Model and Limits]].

## 1. Messages in transit: two encryption layers

1. **Signal protocol with post-quantum keys.** Messages use `libsignal`: the **PQXDH** handshake (X25519 plus **ML-KEM-1024**) and the **SPQR** post-quantum ratchet. This gives forward secrecy and recovery from key compromise, and resists future quantum computers.
2. **An outer layer keyed by the invite.** Every frame is wrapped in XChaCha20-Poly1305 with a hash-chain key derived from the invite secret plus your one-time code (stretched with Argon2). A flaw in the first layer alone does not expose messages.

guft contains **no custom cryptographic primitives**. It composes `libsignal` and audited RustCrypto crates.

All plaintexts are padded to fixed bucket sizes before encryption, so a short message cannot be told from another of similar length.

## 2. The network: Tor onion services

- Each profile is a Tor v3 **onion service** hosted by the app, using a built-in Arti client. There is no server, directory or account.
- **Client authorization:** only people you have connected with hold the key needed to reach your address. A stranger who learns your address cannot even connect.
- Every contact link uses **at least 5 separate Tor circuits** in parallel. Data goes in **fixed 2048-byte cells** spread across them with small random delays, plus cover traffic while a chat is active. Vanguards are enabled. Tor's path length is deliberately not customised (that would make you stand out).
- A message is acknowledged only after the other app has decrypted and stored it.

## 3. On your disk

- One **Argon2id** run (64 MiB, 3 passes) turns your passphrase into a master key; subkeys for the vault and history come from HKDF.
- Chat history is a **SQLCipher** database; the profile state (keys, sessions, contacts) is sealed with XChaCha20-Poly1305. Files are `0600` in a `0700` directory. Deleted rows are zeroed (`secure_delete`).
- On lock, idle timeout or window close, keys and decrypted state are dropped and zeroized.
- Wrong passwords trigger an increasing delay.
- Temporary chats never touch the disk at all ([[Temporary Chats]]).

## 4. Invites

Single use, time limited, and useless without the separate code. An invite is burned only after the first message from the invited person authenticates. A wrong code never burns it.

## 5. The app itself

- Written in Rust; the core crates forbid `unsafe` code.
- **Process sandbox (Linux):** no core dumps, no ptrace/attach, `no_new_privs`, a Landlock filesystem policy (writes only to the app's own directories), and a seccomp deny-list. The app refuses to start unprotected if the sandbox cannot be applied (release builds).
- **UI:** a strict content security policy (no remote content, no `eval`), no raw HTML rendering (every peer-supplied string is shown as text), and a Tauri capability set with no filesystem, shell or network access.
- **Input limits** are checked before anything is allocated or parsed: text 8 KiB, files 1 MB, 25 members per room.
- Names from peers cannot contain control, invisible or text-reordering characters, so a name cannot hide text or fake a file extension.
- The microphone is the only web permission ever granted.

## 6. How we check it

Unit and integration tests (over 110), hostile-input tests (every bit flip in a frame is refused and leaves state untouched, random-frame floods, mutated payloads, corrupted vaults), an at-rest test that scans every file on disk for plaintext, live-Tor tests, `cargo audit`, `cargo deny` and clippy in CI. guft has **not** had an independent security audit.
