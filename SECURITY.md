# Security

guft is a work in progress and has **not** been independently audited. Do not rely on it to protect against a determined, well-resourced adversary yet.

## Reporting a problem

Report vulnerabilities privately to the maintainer (via the repository host), not in a public issue. Include the version, what you expected and what happened. Never include real passphrases, invite codes, onion keys or message contents.

## What is protected

- **Message and file contents in transit.** Every 1:1 and room message is encrypted with libsignal (PQXDH ML-KEM-1024 + X25519 handshake, SPQR ratchet) and then wrapped in a second layer keyed by the invite secret plus the separate one-time code. Relays, exit nodes and anyone without the invite cannot read them.
- **Contents at rest.** Chat history is stored in a SQLCipher database keyed from your passphrase (Argon2id → HKDF subkeys). No keys or plaintext are written unencrypted; the database and vault files are `0600` inside a `0700` directory.
- **Locked state.** On lock, idle timeout or window close, the vault is saved and the master key, engine and all sessions are dropped and zeroized. While locked, the onion service is down and nothing can be decrypted.
- **Reachability.** Your onion address is only shared through invites. Client authorization keys are per contact; a stranger who learns the address cannot connect.
- **Temporary chats and rooms.** Their messages, unsent frames and roster exist only in process memory (zeroized on lock) and are never written to the history database, the outbox or the sealed state. They end when guft locks, idles out or exits. This does not stop a peer from keeping what they received, and the underlying pairwise session state (ratchets) is saved as usual.
- **Voice notes.** Recording starts only when you press the microphone, the browser microphone stream is stopped as soon as you stop, cancel, switch chat or lock, and a clip is never sent without a second tap on Send. The desktop shell grants the microphone permission and refuses every other web permission (camera, screen capture, location, ...). Received voice notes are ordinary files: they are fetched and handed to the system media decoder only when you press play, never automatically. A malformed audio file is therefore a (small) risk to the system's media stack, which is outside guft's control.
- **Names.** Display names and file names from peers may not contain control, invisible or bidirectional-override characters, so a name cannot hide text or fake a file extension.
- **Removal from a room.** A removed member stops receiving messages because nobody encrypts to them any more. Any history they already received cannot be taken back.

## What is not protected

- **Metadata.** Who talks to whom, when, how often and roughly how much. Tor hides the network path, not the fact that two onion services exchange fixed-size cells. Your contact knows your onion address.
- **Endpoint compromise.** Malware, a keylogger, screen capture, an attacker with your unlocked device or a memory dump while unlocked can read everything. The vault passphrase and the invite code are also exposed to anything that can read your keyboard or clipboard.
- **Members.** Everyone in a room can copy, screenshot or leak anything they can read. A room is only as private as its least careful member.
- **Invites shared carelessly.** If the invite and the one-time code travel over the same compromised channel, the session can be established by whoever holds both. Share them separately.
- **Denial of service and traffic analysis.** The app is resilient to garbage and oversized frames, but it does not hide the existence of traffic or guarantee delivery. "Stay reachable while locked" keeps the Tor identity key in memory and still leaks connection metadata; only frames are sealed.
- **Dependencies and builds.** The app pins libsignal to a reviewed tag and runs `cargo audit`, but upstream bugs and supply-chain attacks are not eliminated.
- **Removed/edited docs.** Anything on disk that a peer sent is rendered as untrusted text, but a member of a room sees the name the sender chose; names are not authenticated beyond the session.

## Design rules we follow

- No custom cryptographic primitives or protocols; all crypto comes from `libsignal-protocol` and audited RustCrypto crates.
- All network input is validated against `crates/guft-core/src/limits.rs` before allocating or parsing.
- Peer-supplied file names are sanitised before touching the filesystem.
- Fixed-size cells, size-bucket padding, cover traffic and vanguards; Tor path length is not customised.
- The UI runs under a strict CSP with no remote assets and no `eval`.

We do not claim "zero vulnerabilities": the approach is audited libraries, tests, fuzzing and CI, and we will publish fixes when issues are found.
