# guft — agent guide

Serverless, Tor-based, post-quantum end-to-end encrypted chat. Tauri 2 desktop app (Rust core + React/shadcn UI). License: AGPL-3.0-only (required by libsignal).

## Layout
- `crates/guft-core` — crypto, vault, invites, padding, payloads. No networking, no UI. `#![forbid(unsafe_code)]`.
- `crates/guft-store` — SQLCipher chat history, `Profile` (create/unlock/save/lock), `Locker` (idle auto-lock, wrong-password backoff). One Argon2id run -> master key -> HKDF subkeys (`state`, `history`). Files are 0600, dir 0700; nothing readable on disk.
- `crates/guft-net` — moves opaque frames. `cell` (fixed 2048-byte cells), `reassembly` (bounded), `link` (>=5 isolated circuits per contact, jittered cells, cover traffic, e2e acks, rate limits; generic over any stream), `mem` (in-memory network for tests), `tor` (Arti: onion service from a vault-held seed, in-memory keystore, vanguards, per-(contact,slot) isolation).
- `crates/guft-app` — the application logic the UI calls: `App` (create/unlock/lock, invites, contacts, rooms, send text/files, retry outbox, events, idle auto-lock). Backends: `TorBackend` (production) and `MemBackend` (tests).
- `crates/guft-harden` — process hardening: dumpable off, RLIMIT_CORE 0, no_new_privs, Landlock, seccomp deny list.
- `src-tauri` — Tauri shell, strict CSP, OS sandbox (`sandbox.rs`), IPC commands for 1:1 chat and rooms. Binary: `guft`.
- `ui` — Vite + React + Tailwind + shadcn/ui, strict CSP, no remote assets. Lock screen, welcome, sidebar (contacts and rooms), chat, composer, room dialogs. A dev mock (`lib/mock.ts`, passphrase `demo-passphrase`) runs under `pnpm dev` in a plain browser.

## Security rules (do not weaken)
- Never write custom cryptographic primitives or protocols. Message crypto = `libsignal-protocol` (PQXDH ML-KEM-1024 + SPQR). The extra outer layer (`outer.rs`) only composes audited RustCrypto primitives.
- Pin libsignal to an exact git tag; review every bump (no stable API from upstream).
- Validate all network input against `limits.rs` BEFORE allocating or parsing. File ≤ 1 MB, text ≤ 8 KiB.
- File names from peers go through `payload::check_file_name`; never join them to a path unchecked.
- Secrets use `Zeroizing`; never log keys, plaintext, codes, onion keys or invites.
- Tor path length is not customised (it fingerprints users). Fixed guard, random middles, isolated circuits, fixed-size padding, cover traffic, vanguards.
- UI: strict CSP, no `eval`, no remote scripts/fonts, no `dangerouslySetInnerHTML`. Treat every peer-supplied string as untrusted text.
- Invites are single-use, time-limited, and need the separate one-time code. Burn an invite only after a first message authenticates.

## Commands
- `cargo test --workspace` — all tests (offline)
- `cargo test -p guft-net --test tor_live -- --ignored --nocapture` — real Tor end-to-end (slow; first delivery ~1-2 min). Set `GUFT_VANGUARDS=lite|disabled` to compare. `--test tor_baseline` checks plain Tor connectivity.
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo audit` — RustSec advisories (policy in `.cargo/audit.toml`; every ignore needs a written reason). `cargo deny check` for licenses (must stay AGPL-compatible)
- `cargo build -p guft-desktop` — binary: `target/debug/guft`
- `cd ui && pnpm typecheck && pnpm build`
- ASAN build (nightly): `RUSTFLAGS=-Zsanitizer=address cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu`

## Conventions
- Match surrounding style; small modules; tests beside the code. Errors never contain secrets or plaintext.
- Git is local only for now; do not push or create remotes without being asked.

## Lock model
Unlocked = master key + decrypted engine in memory. `Locker::lock()` saves, closes the DB and drops (zeroizes) everything: manual button, idle timeout, window close. While locked the onion service is down (nothing can be decrypted).

## Rooms
Full mesh over the existing pairwise PQ sessions: no group key, no host. A room-bound invite makes the joiner a member and the inviter introduces them to everyone else. Only the creator may rename or remove; `open_invites` lets members invite. Removal works because nobody encrypts to the removed member. Room history key is `r-<hex>` (`messages`/`mark_read`/`delete_chat` accept it). A room message reads Delivered only when every member acked. Limits: 25 members, 50 rooms.

## Operational notes (UI dev)
- `WEBKIT_DISABLE_DMABUF_RENDERER=1` is set by default in `main.rs`; without it WebKitGTK can crash on Wayland.
- `src-tauri` needs the default `custom-protocol` feature, otherwise the window tries the dev server at localhost:5173.
- Demo mode: `pnpm dev` in a plain browser uses `lib/mock.ts` (passphrase `demo-passphrase`); it is never bundled into release builds.

## Dependencies
Always use the latest releases. Held back only by upstream: `rand` 0.9 (libsignal API), `rusqlite` 0.37 (Arti links the same libsqlite3-sys). Known upstream-only advisories: `paste`, `proc-macro-error2` (build-time macros, unmaintained), `rsa` (ignored with reason in `.cargo/audit.toml`). Re-run `cargo update` + `cargo audit` before every release.

## Operational notes
- Background work must use `with_bg` (does not extend the idle timer); only user-initiated calls use `with`.
- Order on receive: decrypt -> history + `seen` -> save state -> ack. A crash can cause a re-delivery that `seen` absorbs, never a lost message.
- Networks that reset TLS to some relays (seen in this sandbox) make a hosted service slow to become "fully reachable"; sending still works, so tests send with retries instead of waiting on that state.
- rustls needs an explicit crypto provider: `guft-net` enables `rustls/ring`.

## Temporary rooms (memory only)
`RoomKind` is `Normal` | `Temp` (group) | `Direct` (one-to-one, created by `App::start_temp_chat`, reuses the open one, no invites/rename/remove, ends for both when either leaves). Only `Normal` rooms live in `State.rooms`; `Temp`/`Direct` rooms live in `Engine.temp`, which is never serialized (a test asserts the saved bytes do not change). The wire `RoomInvite` carries `kind` so the other side also keeps it in memory. In `guft-app`, temp messages go to `TempStore` and unsent frames to `TempOutbox` (`temp.rs`), never to the database or outbox table; temp ids start at `1<<62`. Both are wiped in `go_quiet` (lock, idle lock) and at unlock. Any new code that writes room messages or room control frames must check `is_temp_room` / `Delivery.temp` first (see `queue_ctl`, `is_temp_chat`). Chat key is still `r-<hex>`; `messages`/`mark_read`/`delete_chat`/`save_file` dispatch on whether the room is temp. Tests: `crates/guft-core/tests/rooms.rs`, `crates/guft-app/tests/temp_rooms.rs` (they unlock the profile from disk to prove nothing temporary was persisted).

## Room consent
`RoomInvite` from a contact is not applied automatically. `process_frame_blocking` applies it only if the user asked for it (`History::take_expected_room`: set in `add_contact` when the imported invite carries a room, `InviteV1.room`) or it is a `Direct` temp chat; otherwise `hold_invitation` keeps it (encrypted DB table `room_pending` for normal rooms, `TempStore.invites` in memory for temp rooms; caps 5 per contact, 30 total, 14 days). `accept_room_invite` runs the original join logic (introductions are sent only now); `decline_room_invite` sends `RoomLeave` to the inviter. A new `Direct` invite from a contact replaces a stale one. Tests: `crates/guft-app/tests/consent.rs`.

## Separate identities (`Hub`)
`crates/guft-app/src/hub.rs`. A `Hub` is the main profile plus personas; each persona is a complete `Profile` in `<profile>/personas/<n>/` (own keys, onion service, contacts, history) with its own `App` and backend (its own Tor client). Its random secret protects it and is kept in the main profile's sealed `personas.nc` (`Profile::read_aux/write_aux`; light Argon2 via `Vault::create_for_random_secret`). A persona exists for one room: created by `create_room(.., Some(alias))` or `add_contact(room invite, code, Some(alias))` (room invites only), erased by `erase_when_sent` after a leave/removal (waits for the leave notice to go out). Ids the UI sees: `p<n>~<id>` for chats, `n*2^42 + id` for messages (kept < 2^53). The hub locks personas with the main identity and restores them on `Event::Unlocked`. Personas never appear in `contacts()`/`chats()`, and `add_contact_to_room` refuses persona rooms. Temp message ids start at 2^40 (they were 2^62, which JavaScript cannot represent). Tests: `crates/guft-app/tests/personas.rs`, live Tor: `tor_personas.rs`.

## Voice notes
A voice note is a normal `File` named `voice-<unix>-<seconds>s.<webm|ogg|m4a>`; nothing special is on the wire. `ui/src/lib/voice.ts` records with `MediaRecorder` (Opus, 24 kbit/s, 60 s max), `VoicePlayer.tsx` decodes only on tap, `Bubble.tsx` recognises the name. Rust side: `App::file_bytes`, command `file_bytes`, CSP `media-src blob:`, and `main.rs` `on_permission_request` allows only the microphone. WebKitGTK on this machine supports Opus/WebM recording; playback could not be verified in a headless WebKit (see handoff).

## Logo
`ui/public/logo.svg` (favicon, standalone, light edge), `ui/src/components/Logo.tsx` (themed, same paths), `src-tauri/icons/*.png` (rendered from the SVG with `rsvg-convert -h N`, padded square). The letters are strokes, not font glyphs.

## Intro video
`docs/guft.mp4` (+ `docs/guft.jpg` poster) is linked from the README. The same file ships in the app as `ui/public/intro.mp4`/`intro.jpg`; `ui/src/components/IntroVideo.tsx` shows the poster and, only when the user presses play, fetches the bundled file into a blob URL (this is why the CSP has `connect-src 'self'`; `media-src` is still `blob:` only). It is shown on the welcome screen and in Settings. Source composition (Hyperframes) lives outside the repo in `brag-output/` (git-excluded); keep the video free of audit/security claims and of copyrighted music.
