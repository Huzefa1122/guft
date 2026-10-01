use std::sync::Arc;
use std::time::Duration;

use guft_app::{App, AppError, AppOptions, Body, MemBackend};
use std::os::unix::fs::PermissionsExt;
use guft_net::mem::MemNetwork;
use guft_net::NetConfig;
use tempfile::TempDir;
use tokio::time::timeout;

const PASS: &str = "correct horse battery";
const HOUR: Duration = Duration::from_secs(3600);

struct Peer {
    app: App<MemBackend>,
    _dir: TempDir,
}

impl Peer {
    fn profile_dir(&self) -> std::path::PathBuf {
        self._dir.path().join("profile")
    }
}

async fn peer(net: &Arc<MemNetwork>, name: &str) -> Peer {
    let dir = tempfile::tempdir().unwrap();
    let opts = AppOptions {
        idle_timeout: Duration::from_secs(600),
        tick: Duration::from_millis(20),
        retry_base: Duration::from_millis(20),
        retry_max: Duration::from_millis(100),
        max_attempts: 300,
    };
    let cfg = NetConfig { max_jitter: Duration::from_millis(1), ack_timeout: Duration::from_secs(2), cover_mean: None, ..NetConfig::default() };
    let app = App::new(dir.path().join("profile"), MemBackend::new(net.clone(), cfg), opts);
    app.create_profile(PASS, name).await.unwrap();
    Peer { app, _dir: dir }
}

/// Poll until `cond` holds (up to 15 s).
async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
    timeout(Duration::from_secs(15), async {
        while !cond() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for: {what}"));
}

fn texts(p: &Peer, room: &str) -> Vec<(String, Option<String>)> {
    let mut v: Vec<_> = p
        .app
        .messages(room, None, 100)
        .unwrap()
        .into_iter()
        .filter_map(|m| match m.body {
            Body::Text(t) => Some((t, m.sender)),
            _ => None,
        })
        .collect();
    v.reverse();
    v
}

fn contact_id(p: &Peer, name: &str) -> String {
    p.app.contacts().unwrap().into_iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no contact {name}")).id
}

fn fully_connected(p: &Peer, others: usize) -> bool {
    p.app.rooms().is_ok_and(|r| r.len() == 1 && r[0].members.len() == others && r[0].members.iter().all(|m| m.connected))
}


/// Wait for `room`'s invitation to reach `p` and accept it.
async fn accept(p: &Peer, room: &str) {
    eventually("the invitation arrives", || p.app.room_invitations().is_ok_and(|v| v.iter().any(|i| i.room == room))).await;
    p.app.accept_room_invite(room).unwrap();
}

/// Alice and Bob become contacts: `joiner` imports an invite from `inviter`.
async fn befriend(joiner: &mut Peer, inviter: &mut Peer) {
    let (invite, code) = inviter.app.new_invite("friend", HOUR).unwrap();
    joiner.app.add_contact(&invite, &code).await.unwrap();
    eventually("both know each other", || joiner.app.contacts().is_ok_and(|c| c.len() == 1) && inviter.app.contacts().is_ok_and(|c| c.len() == 1)).await;
}

/// Every regular file under `dir`, with its bytes.
fn all_files(dir: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut out = vec![];
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let bytes = std::fs::read(&p).unwrap();
                out.push((p, bytes));
            }
        }
    }
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Nothing about a conversation is readable on disk, and everything on disk is private.
fn assert_private_and_opaque(p: &Peer, secrets: &[&[u8]]) {
    let root = p.profile_dir();
    let files = all_files(&root);
    assert!(files.iter().any(|(path, _)| path.ends_with("history.db")), "history database exists");
    for (path, bytes) in &files {
        for s in secrets {
            assert!(!contains(bytes, s), "{} leaks {:?}", path.display(), String::from_utf8_lossy(s));
        }
        let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode & 0o077, 0, "{} is readable by others (mode {mode:o})", path.display());
        assert!(!bytes.starts_with(b"SQLite format 3"), "{} is an unencrypted SQLite file", path.display());
    }
    let dir_mode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode & 0o077, 0, "profile directory is not private");
}

#[tokio::test]
async fn no_conversation_content_is_ever_readable_on_disk() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Zebulon-Marker-Name").await, peer(&net, "Bob").await);
    befriend(&mut bob, &mut alice).await; // bob imports alice's invite
    let bob_in_alice = contact_id(&alice, "Bob");
    let alice_in_bob = contact_id(&bob, "Zebulon-Marker-Name");

    const T1: &str = "PLAINTEXT-MARKER-one-to-one-31337";
    const T2: &str = "PLAINTEXT-MARKER-from-bob-90210";
    const T3: &str = "PLAINTEXT-MARKER-in-room-27182";
    const T4: &str = "PLAINTEXT-MARKER-in-temp-room-16180";
    const FILE_NAME: &str = "FILENAME-MARKER-secret-plans.txt";
    let file_body = b"FILEBODY-MARKER-do-not-leak-".repeat(500);

    alice.app.send_text(&bob_in_alice, T1).await.unwrap();
    bob.app.send_text(&alice_in_bob, T2).await.unwrap();
    alice.app.send_file(&bob_in_alice, FILE_NAME, file_body.clone()).await.unwrap();
    // A voice-note-shaped file goes down the same path.
    alice.app.send_file(&bob_in_alice, "voice-1790000000-7s.webm", vec![0x1a, 0x45, 0xdf, 0xa3, 1, 2, 3]).await.unwrap();
    let room = alice.app.create_room("ROOMNAME-MARKER-heist", true).unwrap();
    alice.app.add_contact_to_room(&room, &bob_in_alice).unwrap();
    accept(&bob, &room).await;
    eventually("bob in room", || fully_connected(&bob, 1) && fully_connected(&alice, 1)).await;
    alice.app.send_room_text(&room, T3).await.unwrap();
    let temp = alice.app.create_temp_room("TEMPROOM-MARKER-ghost", true).unwrap();
    // Bob is a contact already: add him straight in.
    alice.app.add_contact_to_room(&temp, &bob_in_alice).unwrap();
    accept(&bob, &temp).await;
    eventually("bob in temp room", || bob.app.rooms().is_ok_and(|r| r.iter().any(|x| x.id == temp))).await;
    alice.app.send_room_text(&temp, T4).await.unwrap();
    eventually("everything arrived", || {
        bob.app.messages(&alice_in_bob, None, 20).is_ok_and(|m| m.len() >= 4)
            && texts(&bob, &room).len() == 1
            && texts(&bob, &temp).len() == 1
            && alice.app.messages(&alice_in_bob.clone(), None, 1).is_ok()
    })
    .await;

    let secrets: Vec<&[u8]> = vec![T1.as_bytes(), T2.as_bytes(), T3.as_bytes(), T4.as_bytes(), FILE_NAME.as_bytes(), b"FILEBODY-MARKER", b"ROOMNAME-MARKER", b"TEMPROOM-MARKER", b"Zebulon-Marker"];

    // While the app is unlocked and running...
    assert_private_and_opaque(&alice, &secrets);
    assert_private_and_opaque(&bob, &secrets);
    // ...and after locking.
    alice.app.lock().await.unwrap();
    bob.app.lock().await.unwrap();
    assert_private_and_opaque(&alice, &secrets);
    assert_private_and_opaque(&bob, &secrets);

    // A wrong passphrase reveals nothing and the right one still works.
    assert!(alice.app.unlock("not the passphrase").await.is_err());
    alice.app.unlock(PASS).await.unwrap();
    let got = alice.app.messages(&bob_in_alice, None, 10).unwrap();
    assert!(got.iter().any(|m| matches!(&m.body, Body::Text(t) if t == T2)));
}

#[tokio::test]
async fn file_bytes_returns_exactly_what_was_sent_and_refuses_everything_else() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    befriend(&mut alice, &mut bob).await;
    let bob_in_alice = contact_id(&alice, "Bob");
    let alice_in_bob = contact_id(&bob, "Alice");

    let data: Vec<u8> = (0..120_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    let mid = alice.app.send_file(&bob_in_alice, "voice-1790000000-9s.webm", data.clone()).await.unwrap();
    let text_id = alice.app.send_text(&bob_in_alice, "just text").await.unwrap();
    eventually("arrived", || bob.app.messages(&alice_in_bob, None, 10).is_ok_and(|m| m.len() == 2)).await;

    // The sender's copy and the receiver's copy are both byte-exact.
    let (name, bytes) = alice.app.file_bytes(mid).unwrap();
    assert_eq!((name.as_str(), bytes), ("voice-1790000000-9s.webm", data.clone()));
    let theirs = bob.app.messages(&alice_in_bob, None, 10).unwrap().into_iter().find(|m| matches!(m.body, Body::File { .. })).unwrap();
    assert_eq!(bob.app.file_bytes(theirs.id).unwrap().1, data);

    // Text messages, unknown ids and temp-range ids that do not exist are refused.
    assert!(alice.app.file_bytes(text_id).is_err());
    assert!(alice.app.file_bytes(9_999_999).is_err());
    assert!(alice.app.file_bytes(-1).is_err());
    assert!(alice.app.file_bytes(1 << 40).is_err());

    // Locked: nothing is served.
    alice.app.lock().await.unwrap();
    assert!(matches!(alice.app.file_bytes(mid), Err(AppError::Locked)));
}

#[tokio::test]
async fn files_in_temporary_chats_are_served_from_memory_and_vanish_on_lock() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    befriend(&mut alice, &mut bob).await;
    let room = alice.app.start_temp_chat(&contact_id(&alice, "Bob")).unwrap();
    eventually("bob has it", || bob.app.rooms().is_ok_and(|r| r.len() == 1)).await;
    let voice = vec![0x1au8; 4000];
    let mid = alice.app.send_room_file(&room, "voice-1790000001-4s.webm", voice.clone()).await.unwrap();
    eventually("arrived", || bob.app.messages(&room, None, 5).is_ok_and(|m| m.len() == 1)).await;
    let theirs = bob.app.messages(&room, None, 5).unwrap()[0].id;
    assert_eq!(alice.app.file_bytes(mid).unwrap().1, voice);
    assert_eq!(bob.app.file_bytes(theirs).unwrap().1, voice);
    alice.app.lock().await.unwrap();
    alice.app.unlock(PASS).await.unwrap();
    assert!(alice.app.file_bytes(mid).is_err(), "the temporary file did not survive the session");
}

#[tokio::test]
async fn a_deleted_message_is_gone_and_an_unsent_one_is_never_sent() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    befriend(&mut alice, &mut bob).await;
    let (bob_in_alice, alice_in_bob) = (contact_id(&alice, "Bob"), contact_id(&bob, "Alice"));

    // Delivered message: deleting removes only the local copy.
    let first = alice.app.send_text(&bob_in_alice, "keep on bobs side").await.unwrap();
    eventually("delivered", || bob.app.messages(&alice_in_bob, None, 5).is_ok_and(|m| m.len() == 1)).await;
    alice.app.delete_message(first).unwrap();
    assert!(alice.app.messages(&bob_in_alice, None, 5).unwrap().is_empty());
    assert_eq!(bob.app.messages(&alice_in_bob, None, 5).unwrap().len(), 1);

    // Unsent message: deleting cancels it.
    let bob_onion = bob.app.my_onion().unwrap();
    net.set_online(&bob_onion, false);
    let unsent = alice.app.send_text(&bob_in_alice, "never send this").await.unwrap();
    alice.app.send_text(&bob_in_alice, "send this one").await.unwrap();
    alice.app.delete_message(unsent).unwrap();
    net.set_online(&bob_onion, true);
    eventually("the kept one arrives", || bob.app.messages(&alice_in_bob, None, 10).is_ok_and(|m| m.len() == 2)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let got: Vec<_> = bob.app.messages(&alice_in_bob, None, 10).unwrap().into_iter().filter_map(|m| match m.body { Body::Text(t) => Some(t), _ => None }).collect();
    assert!(!got.iter().any(|t| t == "never send this"), "a deleted unsent message was sent: {got:?}");

    // Temporary chats: the same, from memory.
    let room = alice.app.start_temp_chat(&bob_in_alice).unwrap();
    eventually("bob has the temp chat", || bob.app.rooms().is_ok_and(|r| r.len() == 1)).await;
    net.set_online(&bob_onion, false);
    let t1 = alice.app.send_room_text(&room, "temp never").await.unwrap();
    alice.app.send_room_text(&room, "temp kept").await.unwrap();
    alice.app.delete_message(t1).unwrap();
    assert_eq!(alice.app.messages(&room, None, 10).unwrap().len(), 1);
    net.set_online(&bob_onion, true);
    eventually("temp kept arrives", || texts(&bob, &room).len() == 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(texts(&bob, &room)[0].0, "temp kept");
}

#[tokio::test]
async fn contacts_can_be_renamed_locally_and_removal_clears_them_from_rooms() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    befriend(&mut alice, &mut bob).await;
    let (bob_in_alice, alice_in_bob) = (contact_id(&alice, "Bob"), contact_id(&bob, "Alice"));
    let room = alice.app.create_room("Pair", true).unwrap();
    alice.app.add_contact_to_room(&room, &bob_in_alice).unwrap();
    let temp = alice.app.start_temp_chat(&bob_in_alice).unwrap();

    alice.app.rename_contact(&bob_in_alice, "  Robert (work)  ").unwrap();
    assert_eq!(contact_id(&alice, "Robert (work)"), bob_in_alice);
    let rooms = alice.app.rooms().unwrap();
    assert_eq!(rooms.iter().find(|r| r.id == room).unwrap().members[0].name, "Robert (work)");
    assert_eq!(rooms.iter().find(|r| r.id == temp).unwrap().name, "Robert (work)");
    // Bob is not told, and keeps his own name for Alice.
    assert_eq!(bob.app.contacts().unwrap().iter().find(|c| c.id == alice_in_bob).unwrap().name, "Alice");

    for bad in ["", "   ", "Ro\u{202E}bert", "bell\u{7}"] {
        assert!(alice.app.rename_contact(&bob_in_alice, bad).is_err(), "{bad:?}");
    }
    assert!(alice.app.rename_contact("0123456789abcdef0123456789abcdef", "Ghost").is_err());

    // Survives a restart of the session.
    alice.app.lock().await.unwrap();
    alice.app.unlock(PASS).await.unwrap();
    assert!(alice.app.contacts().unwrap().iter().any(|c| c.name == "Robert (work)"));

    // Removing him clears him from the persistent room, and the temp chat with him ends.
    let temp = alice.app.start_temp_chat(&bob_in_alice).unwrap();
    alice.app.remove_contact(&bob_in_alice).unwrap();
    let rooms = alice.app.rooms().unwrap();
    assert!(rooms.iter().find(|r| r.id == room).unwrap().members.is_empty());
    assert!(rooms.iter().all(|r| r.id != temp), "the temporary chat with a removed contact is gone");
}
