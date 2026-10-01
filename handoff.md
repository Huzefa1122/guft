# guft — handoff

State of the project and the work that remains, for the next agent. Read `AGENTS.md` first (layout, security rules, commands, lock model). This file covers what changed after it was written and what is left.

## Product

Serverless, Tor-based, post-quantum end-to-end encrypted chat: Tauri 2 desktop app (Rust core + React/shadcn UI), AGPL-3.0-only.

Hard requirements from the user:
- Text and files only. **No calls or video.** Files ≤ 1 MB, text ≤ 8 KiB.
- Private chat rooms usable "without any hassle after invites", very private and secure.
- Tor onion services, ≥ 5 isolated circuits per contact.
- All messages post-quantum (libsignal PQXDH ML-KEM-1024 + SPQR), plus an outer layer keyed by invite secret + one-time code.
- Password unlocks the local DB; it relocks on idle, manual lock and window close.
- Good UI with shadcn.
- Latest dependency versions. Run `cargo update` and `cargo audit` before releases.
- No custom crypto primitives. Use libsignal and audited crates.

The product was renamed from `nochat` to `guft`: crates are `guft-*`, the binary is `target/debug/guft`, the invite prefix is `guft1:`, the event channel is `guft://event`, env vars are `GUFT_*`, and the Tauri identifier is `dev.guft.app`. Because the identifier changed, an old `nochat` profile is not picked up; create a fresh profile. The repository directory is still named `nochat`.

## Standing constraints (from the user, do not break)

1. **Do not commit or push** unless asked. A remote `origin` (git@khub.local:admin/nochat/nochat.git) exists; one signed commit (cd6b0b8) was pushed earlier at the user's request.
2. **Do not launch the GUI without asking.** A launch needs a fresh request from the user each time.
3. **Never capture the full screen.** Verify the app through process checks, logs and flags only.
4. Keep the Landlock/seccomp sandbox on by default. Opt-out `GUFT_NO_SANDBOX` is debug-only.
5. Security claims: never promise "0 vulns". The approach is audited libraries, `cargo audit`, tests, fuzzing and CI.

## What is done

| Area | State |
|---|---|
| `guft-core` | Vault, invites (+OTP code), outer layer, padding (1 MiB top bucket), payloads, spool, engine, **rooms**. 38 tests. |
| `guft-store` | SQLCipher history (with `sender` column + migration), profile, locker. 8 tests. |
| `guft-net` | Cells, reassembly, link (≥5 circuits, cover traffic, acks), `mem` network, Tor via Arti with restricted discovery (client auth). Live Tor passed before the rename. |
| `guft-app` | `App` incl. locked-mode spool, rooms API, events (`RoomChanged`, `RoomRemoved`). 16 tests. |
| `guft-harden` | dumpable off, RLIMIT_CORE 0, no_new_privs, Landlock, seccomp deny list. Child-process test. |
| `src-tauri` | Shell, strict CSP, all commands (1:1 and rooms). Binary `guft`. |
| `ui/` | Lock screen, welcome, sidebar (contacts **and rooms**), chat, bubbles with per-sender names, composer, add contact, **create room, room invites, members dialog**, safety numbers, settings, toasts. Black-and-white theme (user plans to make it customisable later). Settings overflow fixed. |
| Release hardening | `.cargo/config.toml` (PIE, full RELRO, noexecstack), release profile (`lto`, `panic=abort`, `overflow-checks`, `strip`), `deny.toml`, `ci.sh`. Stack protector needs nightly and is still missing. |
| Docs | `README.md`, `SECURITY.md` (threat model), `AGENTS.md` updated. |

### Rooms design (implemented in core and app, surfaced in the UI)
- Full mesh over existing pairwise PQ sessions. No group key, no host.
- Room-bound invites. A joiner sends `Introduce` through the inviter; the inviter forwards `Introduction` to the others. Intro invites are bound to one identity via `expect`.
- Creator-only remove and rename. `open_invites` lets members invite. The UI hides invite actions when neither applies.
- Removal works because nobody encrypts to the removed member.
- A room message shows Delivered only when every member acked (`outbox_remaining == 0`), else Sent.
- Room chat key is `"r-<hex>"`. Limits: 25 members per room, 50 rooms.
- Engine API: `crates/guft-core/src/rooms.rs`. App API in `crates/guft-app/src/app.rs`: `rooms()`, `create_room`, `new_room_invite`, `add_contact_to_room`, `leave_room`, `remove_room_member`, `rename_room`, `send_room_text`, `send_room_file`.
- Commands in `src-tauri/src/commands.rs`: `rooms`, `create_room`, `room_invite`, `add_to_room`, `leave_room`, `remove_member`, `rename_room`, `send_room_text`, `send_room_file`. `messages`/`mark_read`/`delete_chat` accept room keys.
- UI: `lib/api.ts` types and wrappers, mock rooms in `lib/mock.ts`, merged conversation list in `lib/store.tsx` (`Conv`), `CreateRoom.tsx`, `RoomInvite.tsx`, `RoomMembers.tsx`, sender names in `Bubble.tsx` with a per-sender hue (`--sender-hue`).

## Verified on the final tree
- `./ci.sh` is green: `cargo test --workspace` (82 tests), clippy `-D warnings`, `cargo audit`, `cargo deny check`, `pnpm typecheck && pnpm build`.
- `cargo build -p guft-desktop` produces `target/debug/guft`; `readelf` confirms PIE, full RELRO (`BIND_NOW`) and a non-executable stack.
- Live Tor: `tor_baseline` and `tor_live` (`--ignored --nocapture`) both passed; authorized delivery acked, strangers refused with `NotEnoughCircuits`.
- The desktop UI was exercised in a headless browser against the dev mock (unlock, rooms list, create room, room invite + code, members dialog, add member, sender names, settings). The real Tauri window was not launched.
- A flaky core test (`strangers_and_non_members_cannot_inject_messages_or_introductions`) indexed rooms by insertion order; room ids are random, so it now looks the room up by id. Ran 20× green.

## Remaining tasks (in priority order)

### 1. Missed-message relay between room members (known limitation)
A member only receives a room message if the sender eventually comes online again while they are (the sender's outbox retries). If the sender never returns, the message is stuck. Plan: members relay missed messages for each other. It must keep `seen` dedupe, bound memory and bandwidth, and never let a non-member inject. This needs a message id inside room payloads and dedupe by that id (frame hashes differ on relay). Add tests in `crates/guft-app/tests/rooms.rs`.

### 2. CI and hardened releases
- `ci.sh` exists (tests, clippy, audit, deny, pnpm). Wire it to the repository host's CI (khub runners) or a GitHub Actions workflow if the project moves.
- Finish release hardening: stack protector is unstable in stable Rust; revisit when it stabilises. Consider reproducible builds.

### 3. UI polish (user is iterating on this)
- The user wants a customisable colour scheme later; the current palette is deliberately black and white.
- Consider a progress indicator for large (1 MB) file sends over Tor and rate/queue visibility.

## Gotchas
- `WEBKIT_DISABLE_DMABUF_RENDERER=1` is set by default in `main.rs`; without it WebKitGTK crashes on Wayland with "Error 71".
- `src-tauri` needs the default `custom-protocol` feature, otherwise the window tries to reach the dev server at localhost:5173 and shows "Connection refused".
- In `crates/guft-app`, background work must use `with_bg` and only user actions use `with`, otherwise the idle auto-lock never fires.
- Receive order: decrypt → history + `seen` → save state → ack. Don't reorder.
- `libsignal` is pinned to git tag `v0.103.1`; `SparsePostQuantumRatchet` to `v1.6.0`. Review every bump.
- `rand` 0.9 and `rusqlite` 0.37 are held back by upstream (libsignal API; Arti shares libsqlite3-sys).
- `.cargo/audit.toml` ignores RUSTSEC-2023-0071 (rsa via Arti; no RSA private-key operations) with the reason written there. `deny.toml` repeats the ignore for `cargo deny`.
- This sandbox resets TLS to some Tor relays, so a hosted onion service is slow to become "fully reachable". Sending still works; tests send with retries.
- Tor path length is deliberately not customised (it fingerprints users).
- Demo mode for UI development: the mock API accepts passphrase `demo-passphrase`.
- Debug log of the running app: `serving the bundled UI` confirms the embedded UI is used.
- Settings overflow was caused by the dialog grid track sizing to a long unbreakable onion address; `DialogContent` now uses `grid-cols-[minmax(0,1fr)]`. Keep that when touching dialogs.

## Commands
```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
cargo deny check
cargo build -p guft-desktop        # binary: target/debug/guft
cd ui && pnpm typecheck && pnpm build
./ci.sh
cargo test -p guft-net --test tor_live -- --ignored --nocapture
```

## Update: logo and temporary rooms (done)
- Logo: shield with "guft" (see `AGENTS.md`, section Logo). Brand, favicon and Tauri icons use it.
- Temporary rooms: one-tap 1:1 temp chat (button in a contact's chat header) and group temp rooms (switch in "New room"). Memory only on both devices, wiped on lock/idle/exit. Design and rules in `AGENTS.md` (section Temporary rooms). `./ci.sh` is green with 90 tests. The real Tauri window was not launched for this change; the UI was checked in a headless browser against the dev mock.
- Ideas not done: a "burn after reading" timer per message, a visible countdown for idle lock inside temp chats, and relaying missed messages between members (task 1 above).

## Pre-release review (2026-10-01)

### Fixed in this pass
- **Spoofable names (real bug, found by test).** File names and display names accepted bidi overrides and invisible characters (`invoice<RLO>fdp.exe` displays as `invoiceexe.pdf`; `Bob<ZWSP>` looks identical to `Bob`). `payload.rs::is_spoofing_char` now rejects them (U+FE0F stays allowed for emoji).
- IPC text size is bounded in the Tauri layer (16 KiB) before it reaches the core.
- `remove_contact` left the contact in every room roster; it now clears them (and ends a temp chat with them).

### Added
- Select dropdowns restyled (shadcn `Select`), voice notes (see `AGENTS.md`), contact rename (local only), copy / delete a single message (an unsent message is cancelled), `file_bytes` command.
- Tests: `guft-core/tests/hostile.rs` (14: every bit flip refused and state unchanged, large-file tamper, replay, cross-contact relabel, 8 s of random frames with plausible headers, mutated payload fuzz, absurd length prefixes, spoof names, bad invites, corrupted vault, spool blobs, ciphertext balance/uniqueness), `guft-net/tests/hostile.rs` (5: cell fuzz, reassembly bounds under flood), `guft-app/tests/at_rest.rs` (5: marker scan of every file on disk while unlocked and locked for 1:1, files, voice-named files, rooms and temp rooms; file modes; delete/rename), `guft-net/tests/tor_large.rs` (live Tor, ignored).
- Whole suite: `./ci.sh` green, 114 tests, clippy `-D warnings`, `cargo audit` (0 vulnerabilities, 5 warnings), `cargo deny`.

### Verified
- Live Tor: a 190 kB frame and a 1,064,896-byte frame (the largest legal file) arrived byte-identical. The big one took ~30 s once circuits were warm; with cold circuits and one ack timeout a send took 2 to 3 minutes. The UI shows no progress for this.
- Nothing readable on disk (marker scan), no unencrypted SQLite, all files `0600`, directory `0700`; `secure_delete` is on; temporary chats leave nothing in the database, outbox or saved state.
- UI: text containing `<img onerror>`, `<script>`, `javascript:` renders as plain text; no `innerHTML`/`eval`/links anywhere; capability file only has `core:event:default`.
- Not run: ASAN (no sanitizer runtime in this Rust install, no rustup), Valgrind (needs libc debug symbols), cargo-fuzz (not installed). The hostile tests are the substitute; add real fuzzing to CI (targets: `Payload::decode`, `cell::parse`, `Reassembler::push`, `Engine::decrypt`, `InviteV1::decode`, `Vault::unlock`).

### Open findings, most important first
1. **A contact can add you to a room and you join automatically** (`RoomInvite` from an existing contact). Joining makes your client introduce itself to every member (they become contacts and learn your onion address and identity). A contact can therefore expose you to strangers without consent. Fix: hold `RoomInvite` from contacts as a pending invitation the user accepts or declines; auto-accept only when the sender is someone whose invite you just used (a room-bound invite you imported). Needs engine, app, UI and tests.
2. **No schema version in the saved state.** `State` is postcard (not self-describing), so any change to a persisted struct breaks every existing profile. Add a version header and migration before the first public release.
3. **No passphrase change, no backup/restore, minimum passphrase is 8 characters.** Argon2id is 64 MiB x 3 (RFC 9106 second choice), so the passphrase is the weak point against an offline attacker. Plan: random history key stored inside the vault (so changing the passphrase only re-seals `state.nc`), strength meter, 10+ characters, documented export of the profile directory.
4. **Packaging and updates.** `bundle.active` is false; there are no installers, signing, update channel or reproducible-build story. The sandbox is Linux-only (`guft-harden` returns an empty report elsewhere and `sandbox.rs` ignores it, so other OSes would run unsandboxed without saying so).
5. **Upstream experimental dependencies:** libsignal is a git pin (no crates.io release), Arti's `launch_onion_service_with_hsid` and `restricted-discovery` are experimental APIs. Re-read their changelogs at every bump.
6. **Landlock grants read access to the whole filesystem** (only writes are restricted), so an exploit in the UI process could read `~/.ssh`, browser profiles, and so on. Narrow the read rules to system directories plus the app's own directories.
7. **One shared reassembler for all inbound streams** (32 partial frames, 4 MiB): a malicious authorized contact can keep it full and stall other contacts' multi-cell frames for up to 5 minutes. Per-contact accounting needs stream-to-contact attribution, which Tor does not give us today.
8. **Voice playback in real WebKitGTK is unverified.** Opus/WebM recording works there (checked off-screen with a synthetic stream); audio element playback could not be exercised headless. The real microphone path (permission handler, sandbox, pipewire/pulse socket via `XDG_RUNTIME_DIR`) needs a test by a person. If playback fails the UI says so and offers Save.
9. Keys are zeroized but libsignal internals are not, and nothing is `mlock`ed (a locked app can still be swapped out while unlocked). Recommend encrypted swap in the docs; do not use `mlockall` (it breaks WebKit under normal RLIMIT_MEMLOCK).
10. Creator-only room administration: if the creator leaves or is removed nobody can rename or remove members.
11. `cargo audit` warnings: `glib 0.18.5` unsound (`VariantStrIter`, via Tauri's gtk-rs), `paste`, `proc-macro-error`, `proc-macro-error2`, `bincode` unmaintained. All upstream.
12. CSP keeps `style-src 'unsafe-inline'` (the component library sets inline styles).

### Missing for everyday chat (not done)
Search inside a chat; reply/quote; reactions; edit; inline image preview (decode on tap only); desktop notifications (opt-in, content-free); drag and drop or paste of files; retry button for a failed message; send progress and cancel for large files; mute; lock when the window is minimized or loses focus; change own display name; passphrase change (see above); export/import. Multi-device is not supported by design (one ratchet per contact). Typing indicators, read receipts and link previews are deliberately absent for privacy.

