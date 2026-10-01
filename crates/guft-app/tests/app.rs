use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use guft_app::{App, AppError, AppOptions, Body, Event, MemBackend, Status};
use guft_core::limits::MAX_FILE_BYTES;
use guft_core::Payload;
use guft_net::access::{Access, AccessControl};
use guft_net::mem::MemNetwork;
use guft_net::{Net, Transport};
use guft_net::NetConfig;
use tempfile::TempDir;
use tokio::sync::broadcast;
use tokio::time::timeout;

const PASS: &str = "correct horse battery";
const HOUR: Duration = Duration::from_secs(3600);

fn opts() -> AppOptions {
    AppOptions {
        idle_timeout: Duration::from_secs(600),
        tick: Duration::from_millis(20),
        retry_base: Duration::from_millis(20),
        retry_max: Duration::from_millis(100),
        max_attempts: 300,
    }
}

fn cfg() -> NetConfig {
    NetConfig {
        max_jitter: Duration::from_millis(1),
        ack_timeout: Duration::from_secs(2),
        cover_mean: None,
        ..NetConfig::default()
    }
}

struct Peer {
    app: App<MemBackend>,
    events: broadcast::Receiver<Event>,
    dir: TempDir,
}

fn new_app(net: &Arc<MemNetwork>, dir: &std::path::Path, o: AppOptions) -> App<MemBackend> {
    App::new(dir.join("profile"), MemBackend::new(net.clone(), cfg()), o)
}

async fn peer(net: &Arc<MemNetwork>, name: &str) -> Peer {
    let dir = tempfile::tempdir().unwrap();
    let app = new_app(net, dir.path(), opts());
    let events = app.subscribe();
    app.create_profile(PASS, name).await.unwrap();
    Peer { app, events, dir }
}

async fn wait_for(rx: &mut broadcast::Receiver<Event>, what: &str, pred: impl Fn(&Event) -> bool) -> Event {
    timeout(Duration::from_secs(15), async {
        loop {
            let e = rx.recv().await.expect("event channel closed");
            if pred(&e) {
                return e;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for: {what}"))
}

/// `a` imports an invite from `b`. Returns (b's id inside a, a's id inside b).
async fn connect(a: &mut Peer, b: &mut Peer) -> (String, String) {
    let (invite, code) = b.app.new_invite("friend", HOUR).unwrap();
    let b_in_a = a.app.add_contact(&invite, &code).await.unwrap();
    let Event::ContactAdded { id: a_in_b, .. } =
        wait_for(&mut b.events, "contact added on b", |e| matches!(e, Event::ContactAdded { .. })).await
    else {
        unreachable!()
    };
    (b_in_a, a_in_b)
}

fn text_of(m: &guft_app::StoredMessage) -> &str {
    match &m.body {
        Body::Text(t) => t,
        _ => panic!("not text"),
    }
}

#[tokio::test]
async fn onboarding_and_conversation() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    let (bob_in_alice, alice_in_bob) = connect(&mut alice, &mut bob).await;

    assert_eq!(alice.app.contacts().unwrap()[0].name, "Bob");
    assert_eq!(bob.app.contacts().unwrap()[0].name, "Alice");

    let mid = alice.app.send_text(&bob_in_alice, "hello bob").await.unwrap();
    wait_for(&mut alice.events, "delivered", |e| matches!(e, Event::Delivered { msg_id, .. } if *msg_id == mid)).await;
    wait_for(&mut bob.events, "message at bob", |e| matches!(e, Event::Message { .. })).await;
    let got = bob.app.messages(&alice_in_bob, None, 10).unwrap();
    assert_eq!((text_of(&got[0]), got[0].outgoing), ("hello bob", false));
    assert_eq!(alice.app.messages(&bob_in_alice, None, 10).unwrap()[0].status, Status::Delivered);

    bob.app.send_text(&alice_in_bob, "hi alice").await.unwrap();
    wait_for(&mut alice.events, "reply at alice", |e| matches!(e, Event::Message { .. })).await;
    let chats = alice.app.chats().unwrap();
    assert_eq!(chats[0].unread, 1);
    alice.app.mark_read(&bob_in_alice).unwrap();
    assert_eq!(alice.app.chats().unwrap()[0].unread, 0);

    // Many messages both ways keep the ratchets in step.
    for i in 0..25 {
        alice.app.send_text(&bob_in_alice, &format!("a{i}")).await.unwrap();
        bob.app.send_text(&alice_in_bob, &format!("b{i}")).await.unwrap();
    }
    timeout(Duration::from_secs(20), async {
        loop {
            let a = alice.app.messages(&bob_in_alice, None, 200).unwrap();
            let b = bob.app.messages(&alice_in_bob, None, 200).unwrap();
            if a.len() == 2 + 50 && b.len() == 2 + 50 && a.iter().chain(&b).all(|m| m.status == Status::Delivered) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("all messages delivered");

    assert_eq!(alice.app.safety_number(&bob_in_alice).unwrap(), bob.app.safety_number(&alice_in_bob).unwrap());
    assert!(!alice.app.contacts().unwrap()[0].verified);
    alice.app.set_verified(&bob_in_alice, true).unwrap();
    assert!(alice.app.contacts().unwrap()[0].verified);
}

#[tokio::test]
async fn messages_queue_while_peer_is_offline_and_arrive_in_order() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    let (bob_in_alice, alice_in_bob) = connect(&mut alice, &mut bob).await;
    let bob_onion = bob.app.my_onion().unwrap();

    net.set_online(&bob_onion, false);
    let mut sent = Vec::new();
    for i in 0..3 {
        let t = format!("queued {i}");
        sent.push(alice.app.send_text(&bob_in_alice, &t).await.unwrap());
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let msgs = alice.app.messages(&bob_in_alice, None, 50).unwrap();
    assert!(msgs.iter().all(|m| m.status == Status::Queued), "nothing may be marked delivered while offline");
    assert!(bob.app.messages(&alice_in_bob, None, 50).unwrap().is_empty());

    net.set_online(&bob_onion, true);
    timeout(Duration::from_secs(15), async {
        loop {
            if bob.app.messages(&alice_in_bob, None, 50).unwrap().len() == 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("queued messages arrive after reconnect");
    let got: Vec<String> = bob.app.messages(&alice_in_bob, None, 50).unwrap().iter().rev().map(|m| text_of(m).to_owned()).collect();
    assert_eq!(got, ["queued 0", "queued 1", "queued 2"], "order preserved");
}

#[tokio::test]
async fn locked_app_is_offline_and_opens_only_with_password() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    let (bob_in_alice, alice_in_bob) = connect(&mut alice, &mut bob).await;

    bob.app.lock().await.unwrap();
    wait_for(&mut bob.events, "locked event", |e| *e == Event::Locked).await;
    assert!(!bob.app.is_unlocked());
    assert!(matches!(bob.app.contacts(), Err(AppError::Locked)));
    assert!(matches!(bob.app.messages(&alice_in_bob, None, 5), Err(AppError::Locked)));
    assert!(matches!(bob.app.send_text(&alice_in_bob, "x").await, Err(AppError::Locked)));

    // While Bob is locked nobody can reach him; Alice's message waits in her queue.
    alice.app.send_text(&bob_in_alice, "are you there?").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(alice.app.messages(&bob_in_alice, None, 5).unwrap()[0].status, Status::Queued);

    assert!(bob.app.unlock("wrong password!!").await.is_err());
    assert!(!bob.app.is_unlocked());
    bob.app.unlock(PASS).await.unwrap();
    wait_for(&mut bob.events, "message after unlock", |e| matches!(e, Event::Message { .. })).await;
    assert_eq!(text_of(&bob.app.messages(&alice_in_bob, None, 5).unwrap()[0]), "are you there?");
}

#[tokio::test]
async fn idle_timeout_relocks_and_drops_the_network() {
    let net = MemNetwork::new();
    let dir = tempfile::tempdir().unwrap();
    let app = new_app(&net, dir.path(), AppOptions { idle_timeout: Duration::from_millis(400), ..opts() });
    let mut events = app.subscribe();
    app.create_profile(PASS, "Solo").await.unwrap();
    let onion = app.my_onion().unwrap();
    wait_for(&mut events, "network ready", |e| matches!(e, Event::NetworkReady { .. })).await;

    // Activity keeps it open.
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        app.contacts().unwrap();
    }
    assert!(app.is_unlocked());

    wait_for(&mut events, "auto lock", |e| *e == Event::Locked).await;
    assert!(!app.is_unlocked());
    assert!(matches!(app.contacts(), Err(AppError::Locked)));
    assert!(!net.is_registered(&onion), "onion service must be gone while locked");
}

#[tokio::test]
async fn everything_survives_a_restart() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    let (bob_in_alice, alice_in_bob) = connect(&mut alice, &mut bob).await;
    alice.app.send_text(&bob_in_alice, "before restart").await.unwrap();
    wait_for(&mut bob.events, "msg", |e| matches!(e, Event::Message { .. })).await;
    bob.app.lock().await.unwrap();
    drop(bob.app);

    // A brand-new process on the same files.
    let bob2 = new_app(&net, bob.dir.path(), opts());
    let mut ev2 = bob2.subscribe();
    assert!(bob2.has_profile());
    bob2.unlock(PASS).await.unwrap();
    assert_eq!(bob2.contacts().unwrap().len(), 1);
    assert_eq!(text_of(&bob2.messages(&alice_in_bob, None, 5).unwrap()[0]), "before restart");

    alice.app.send_text(&bob_in_alice, "after restart").await.unwrap();
    wait_for(&mut ev2, "msg after restart", |e| matches!(e, Event::Message { .. })).await;
    bob2.send_text(&alice_in_bob, "reply after restart").await.unwrap();
    wait_for(&mut alice.events, "reply", |e| matches!(e, Event::Message { .. })).await;
}

#[tokio::test]
async fn files_roundtrip_and_save_safely() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    let (bob_in_alice, alice_in_bob) = connect(&mut alice, &mut bob).await;

    let data: Vec<u8> = (0..MAX_FILE_BYTES as u32).map(|i| (i % 253) as u8).collect();
    alice.app.send_file(&bob_in_alice, "report.pdf", data.clone()).await.unwrap();
    let Event::Message { id, .. } = wait_for(&mut bob.events, "file at bob", |e| matches!(e, Event::Message { .. })).await else { unreachable!() };
    let m = &bob.app.messages(&alice_in_bob, None, 5).unwrap()[0];
    assert_eq!(m.body, Body::File { name: "report.pdf".into(), size: MAX_FILE_BYTES });

    let out = tempfile::tempdir().unwrap();
    let p1 = bob.app.save_file(id, out.path()).unwrap();
    let p2 = bob.app.save_file(id, out.path()).unwrap();
    assert_eq!(std::fs::read(&p1).unwrap(), data);
    assert_ne!(p1, p2, "never overwrite an existing file");
    assert_eq!(p2.file_name().unwrap(), "report (1).pdf");

    assert!(alice.app.send_file(&bob_in_alice, "big.bin", vec![0; MAX_FILE_BYTES + 1]).await.is_err());
    for bad in ["../evil", "a/b", ".hidden", ""] {
        assert!(alice.app.send_file(&bob_in_alice, bad, vec![1]).await.is_err(), "{bad:?}");
    }
}

#[tokio::test]
async fn wrong_code_never_creates_a_contact_and_invites_are_single_use() {
    let net = MemNetwork::new();
    let (alice, bob, carol) = (peer(&net, "Alice").await, peer(&net, "Bob").await, peer(&net, "Carol").await);
    let (invite, code) = bob.app.new_invite("x", HOUR).unwrap();

    // Mallory has the invite but not the code.
    alice.app.add_contact(&invite, "aaaa-bbbb-cccc").await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(bob.app.contacts().unwrap().is_empty());
    assert_eq!(bob.app.pending_invites().unwrap().len(), 1, "a failed attempt must not burn the invite");

    carol.app.add_contact(&invite, &code).await.unwrap();
    let mut bob = bob;
    wait_for(&mut bob.events, "carol accepted", |e| matches!(e, Event::ContactAdded { name, .. } if name == "Carol")).await;
    assert!(bob.app.pending_invites().unwrap().is_empty());

    // Same invite and code again, from someone else: refused.
    let dave = peer(&net, "Dave").await;
    dave.app.add_contact(&invite, &code).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(bob.app.contacts().unwrap().len(), 1);
}

#[tokio::test]
async fn re_delivered_frames_are_acknowledged_but_stored_once() {
    let net = MemNetwork::new();
    let bob = peer(&net, "Bob").await;
    let (invite, code) = bob.app.new_invite("x", HOUR).unwrap();

    // A raw engine plays Alice so we can replay exact frames.
    let mut raw = guft_core::Engine::create("Alice").unwrap();
    let onion = "q".repeat(56) + ".onion";
    raw.set_onion(&onion).unwrap();
    let id = raw.add_contact(&invite, &code, guft_core::engine::now_secs()).unwrap();
    let hello = raw.encrypt(&id, &raw.hello_for(&id).unwrap()).unwrap();
    let text = raw.encrypt(&id, &Payload::Text("once only".into())).unwrap();

    assert!(bob.app.process_frame(&hello).await);
    assert!(bob.app.process_frame(&hello).await, "duplicate must still be acknowledged");
    assert!(bob.app.process_frame(&text).await);
    assert!(bob.app.process_frame(&text).await);
    let contacts = bob.app.contacts().unwrap();
    assert_eq!(contacts.len(), 1);
    assert_eq!(bob.app.messages(&contacts[0].id, None, 10).unwrap().len(), 1, "stored exactly once");

    // Garbage and forged frames are refused (no acknowledgement) and change nothing.
    assert!(!bob.app.process_frame(&[0u8; 5]).await);
    assert!(!bob.app.process_frame(&vec![7u8; 5000]).await);
    let mut forged = text.clone();
    *forged.last_mut().unwrap() ^= 1;
    assert!(!bob.app.process_frame(&forged).await);
    assert_eq!(bob.app.messages(&contacts[0].id, None, 10).unwrap().len(), 1);
}

#[tokio::test]
async fn bad_input_is_rejected_cleanly() {
    let net = MemNetwork::new();
    let alice = peer(&net, "Alice").await;
    assert!(alice.app.add_contact("not an invite", "aaaa-bbbb-cccc").await.is_err());
    assert!(alice.app.add_contact(&"guft1:".repeat(5000), "aaaa-bbbb-cccc").await.is_err());
    assert!(alice.app.send_text("nobody", "hi").await.is_err());
    assert!(alice.app.new_invite("x", Duration::from_secs(10)).is_err());
    assert!(alice.app.contacts().unwrap().is_empty());
    let (invite, _) = alice.app.new_invite("mine", HOUR).unwrap();
    assert!(alice.app.add_contact(&invite, "aaaa-bbbb-cccc").await.is_err(), "own invite");
}

#[tokio::test]
async fn strangers_cannot_reach_a_node_and_invite_keys_die_after_use() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    let bob_onion = bob.app.my_onion().unwrap();

    // No key at all: refused.
    let stranger = Net::new(net.connector(), cfg()).unwrap();
    assert!(stranger.send(&bob_onion, vec![1; 100]).await.is_err());
    assert!(net.denied.load(std::sync::atomic::Ordering::Relaxed) > 0);

    // A made-up key: refused.
    let (fake_ctl, fake_conn) = net.node_access("mallory");
    fake_ctl.update(Access { authorized: vec![], connect: vec![(bob_onion.clone(), zeroize::Zeroizing::new([9u8; 32]))] }).unwrap();
    assert!(guft_net::Connector::connect(&fake_conn, &bob_onion, 0).await.is_err());

    // The invite's own key works exactly until the invite is used.
    let (invite, code) = bob.app.new_invite("friend", HOUR).unwrap();
    let inv_key = guft_core::invite::InviteV1::decode(&invite).unwrap().auth;
    let (spy_ctl, spy_conn) = net.node_access("spy");
    spy_ctl.update(Access { authorized: vec![], connect: vec![(bob_onion.clone(), zeroize::Zeroizing::new(inv_key))] }).unwrap();
    assert!(guft_net::Connector::connect(&spy_conn, &bob_onion, 0).await.is_ok(), "invite key works before use");

    alice.app.add_contact(&invite, &code).await.unwrap();
    wait_for(&mut bob.events, "contact", |e| matches!(e, Event::ContactAdded { .. })).await;
    timeout(Duration::from_secs(5), async {
        while guft_net::Connector::connect(&spy_conn, &bob_onion, 0).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the invite key must stop working once it has been used");

    // And the conversation still works with the permanent keys that replaced it.
    let bob_in_alice = alice.app.contacts().unwrap()[0].id.clone();
    let mid = alice.app.send_text(&bob_in_alice, "still works").await.unwrap();
    wait_for(&mut alice.events, "delivered", |e| matches!(e, Event::Delivered { msg_id, .. } if *msg_id == mid)).await;
}

#[tokio::test]
async fn locked_but_online_spools_sealed_frames_and_reads_them_on_unlock() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    let (bob_in_alice, alice_in_bob) = connect(&mut alice, &mut bob).await;
    bob.app.set_online_when_locked(true).unwrap();
    assert!(bob.app.online_when_locked().unwrap());

    bob.app.lock().await.unwrap();
    assert!(matches!(bob.app.messages(&alice_in_bob, None, 5), Err(AppError::Locked)));

    const MARKER: &str = "SPOOL-MARKER-7d41f0";
    let mid = alice.app.send_text(&bob_in_alice, MARKER).await.unwrap();
    wait_for(&mut alice.events, "delivered while bob is locked", |e| matches!(e, Event::Delivered { msg_id, .. } if *msg_id == mid)).await;

    // Stored sealed: nothing readable on disk, not even the plaintext marker.
    let spool = bob.dir.path().join("profile/spool");
    let files: Vec<_> = std::fs::read_dir(&spool).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);
    let bytes = std::fs::read(files[0].path()).unwrap();
    assert!(!bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()));
    assert_eq!(std::fs::metadata(files[0].path()).unwrap().permissions().mode() & 0o077, 0);

    bob.app.unlock(PASS).await.unwrap();
    wait_for(&mut bob.events, "message after unlock", |e| matches!(e, Event::Message { .. })).await;
    assert_eq!(text_of(&bob.app.messages(&alice_in_bob, None, 5).unwrap()[0]), MARKER);
    assert_eq!(std::fs::read_dir(&spool).unwrap().count(), 0, "spool is emptied once read");

    // Turning it off makes a locked app unreachable again.
    bob.app.set_online_when_locked(false).unwrap();
    bob.app.lock().await.unwrap();
    let mid2 = alice.app.send_text(&bob_in_alice, "second").await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let msgs = alice.app.messages(&bob_in_alice, None, 5).unwrap();
    assert_eq!(msgs.iter().find(|m| m.id == mid2).unwrap().status, Status::Queued);
}
