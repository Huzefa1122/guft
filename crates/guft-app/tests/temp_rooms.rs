use std::sync::Arc;
use std::time::Duration;

use guft_app::{App, AppOptions, Body, Event, MemBackend, Status};
use guft_store::Profile;
use guft_net::mem::MemNetwork;
use guft_net::NetConfig;
use tempfile::TempDir;
use tokio::sync::broadcast;
use tokio::time::timeout;

const PASS: &str = "correct horse battery";
const HOUR: Duration = Duration::from_secs(3600);

struct Peer {
    app: App<MemBackend>,
    events: broadcast::Receiver<Event>,
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
    let events = app.subscribe();
    app.create_profile(PASS, name).await.unwrap();
    Peer { app, events, _dir: dir }
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

async fn wait_event(p: &mut Peer, what: &str, pred: impl Fn(&Event) -> bool) -> Event {
    timeout(Duration::from_secs(15), async {
        loop {
            let e = p.events.recv().await.expect("events");
            if pred(&e) {
                return e;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for: {what}"))
}

/// `joiner` joins `room` through an invite from `inviter`.
async fn join(joiner: &Peer, inviter: &Peer, room: &str, label: &str) {
    let (invite, code) = inviter.app.new_room_invite(room, label, HOUR).unwrap();
    joiner.app.add_contact(&invite, &code).await.unwrap();
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


/// Alice and Bob become contacts through an ordinary invite.
async fn befriend(alice: &mut Peer, bob: &mut Peer) {
    let (invite, code) = alice.app.new_invite("bob", HOUR).unwrap();
    bob.app.add_contact(&invite, &code).await.unwrap();
    eventually("both know each other", || alice.app.contacts().is_ok_and(|c| c.len() == 1) && bob.app.contacts().is_ok_and(|c| c.len() == 1)).await;
}

/// Everything durable about a profile, read straight from disk with the password:
/// stored messages, queued frames, and rooms. Temporary rooms must leave no trace in any of it.
fn nothing_temporary_on_disk(p: &Peer, room: &str) {
    let on_disk = Profile::unlock(&p.profile_dir(), PASS).unwrap();
    assert!(on_disk.history.page(room, None, 100).unwrap().is_empty(), "no temp messages in history");
    assert!(on_disk.history.summaries().unwrap().is_empty(), "no summary for temp chats");
    assert!(on_disk.history.outbox_chats().unwrap().is_empty(), "no temp frames in the outbox");
    assert!(on_disk.engine.rooms().is_empty(), "no temp room in the saved state");
}

#[tokio::test]
async fn one_tap_temp_chat_lives_only_in_memory_on_both_sides() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    befriend(&mut alice, &mut bob).await;
    let bob_in_alice = contact_id(&alice, "Bob");

    let room = alice.app.start_temp_chat(&bob_in_alice).unwrap();
    // Tapping again reuses the same chat.
    assert_eq!(alice.app.start_temp_chat(&bob_in_alice).unwrap(), room);
    eventually("bob has the temp chat", || bob.app.rooms().is_ok_and(|r| r.len() == 1)).await;
    let (a, b) = (&alice.app.rooms().unwrap()[0], &bob.app.rooms().unwrap()[0]);
    assert!(a.temp && a.direct && b.temp && b.direct);
    assert_eq!((a.name.as_str(), b.name.as_str()), ("Bob", "Alice"), "each side shows the other person");
    assert_eq!(b.id, room);

    let mid = alice.app.send_room_text(&room, "this will vanish").await.unwrap();
    eventually("bob read it", || texts(&bob, &room).len() == 1).await;
    assert_eq!(texts(&bob, &room)[0].0, "this will vanish");
    eventually("delivered tick", || alice.app.messages(&room, None, 5).unwrap().iter().any(|m| m.id == mid && m.status == Status::Delivered)).await;
    bob.app.send_room_text(&room, "ok").await.unwrap();
    eventually("alice read it", || texts(&alice, &room).len() == 2).await;
    wait_event(&mut bob, "message event", |e| matches!(e, Event::Message { chat, .. } if *chat == room)).await;
    assert_eq!(bob.app.rooms().unwrap()[0].unread, 1);
    bob.app.mark_read(&room).unwrap();
    assert_eq!(bob.app.rooms().unwrap()[0].unread, 0);

    // Files work too and can be saved out on request.
    alice.app.send_room_file(&room, "note.txt", vec![42; 5000]).await.unwrap();
    eventually("file at bob", || bob.app.messages(&room, None, 10).unwrap().iter().any(|m| matches!(&m.body, Body::File { size, .. } if *size == 5000))).await;
    let file = bob.app.messages(&room, None, 10).unwrap().into_iter().find(|m| matches!(m.body, Body::File { .. })).unwrap();
    let out = tempfile::tempdir().unwrap();
    let saved = bob.app.save_file(file.id, out.path()).unwrap();
    assert_eq!(std::fs::read(saved).unwrap(), vec![42; 5000]);

    // The ordinary one-to-one chat is untouched by all of this.
    assert!(alice.app.chats().unwrap().iter().all(|c| c.last.is_none()));

    // Lock: the session ends and everything temporary is gone, with nothing on disk.
    alice.app.lock().await.unwrap();
    nothing_temporary_on_disk(&alice, &room);
    alice.app.unlock(PASS).await.unwrap();
    assert!(alice.app.rooms().unwrap().is_empty(), "the temp chat did not survive the session");
    assert!(alice.app.messages(&room, None, 10).unwrap().is_empty());

    // Bob never wrote anything either, even though his side still shows the chat.
    bob.app.lock().await.unwrap();
    nothing_temporary_on_disk(&bob, &room);
}

#[tokio::test]
async fn ending_a_temp_chat_ends_it_for_both() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    befriend(&mut alice, &mut bob).await;
    let room = alice.app.start_temp_chat(&contact_id(&alice, "Bob")).unwrap();
    eventually("bob has it", || bob.app.rooms().is_ok_and(|r| r.len() == 1)).await;
    alice.app.send_room_text(&room, "hello").await.unwrap();
    eventually("arrived", || texts(&bob, &room).len() == 1).await;

    bob.app.leave_room(&room).unwrap();
    eventually("alice's side is gone too", || alice.app.rooms().is_ok_and(|r| r.is_empty())).await;
    wait_event(&mut alice, "removed event", |e| matches!(e, Event::RoomRemoved { room: r } if *r == room)).await;
    assert!(bob.app.messages(&room, None, 5).unwrap().is_empty());
    assert!(alice.app.messages(&room, None, 5).unwrap().is_empty());
}

#[tokio::test]
async fn temp_messages_wait_in_memory_for_an_offline_contact_and_are_ordered() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    befriend(&mut alice, &mut bob).await;
    let bob_in_alice = contact_id(&alice, "Bob");
    let alice_in_bob = contact_id(&bob, "Alice");
    let room = alice.app.start_temp_chat(&bob_in_alice).unwrap();
    eventually("bob has it", || bob.app.rooms().is_ok_and(|r| r.len() == 1)).await;

    let bob_onion = bob.app.my_onion().unwrap();
    net.set_online(&bob_onion, false);
    let mid = alice.app.send_room_text(&room, "one").await.unwrap();
    // A normal message to the same contact interleaves with it.
    alice.app.send_text(&bob_in_alice, "normal").await.unwrap();
    alice.app.send_room_text(&room, "two").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(texts(&bob, &room).is_empty());
    let status = alice.app.messages(&room, None, 5).unwrap().iter().find(|m| m.id == mid).unwrap().status;
    assert_eq!(status, Status::Queued);

    // Only the temporary frames are kept off disk; the normal one is in the outbox.
    alice.app.lock().await.unwrap();
    let on_disk = Profile::unlock(&alice.profile_dir(), PASS).unwrap();
    assert_eq!(on_disk.history.outbox_for(&bob_in_alice).unwrap().len(), 1, "only the normal message is queued on disk");
    drop(on_disk);
    alice.app.unlock(PASS).await.unwrap();
    net.set_online(&bob_onion, true);
    eventually("normal one arrives", || bob.app.messages(&alice_in_bob, None, 5).unwrap().len() == 1).await;
    // The temp ones were only in memory: after the lock they are gone, and never arrive.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(texts(&bob, &room).is_empty());

    // Without a lock in between they do arrive, in order.
    let room = alice.app.start_temp_chat(&bob_in_alice).unwrap();
    eventually("bob has the new chat", || bob.app.rooms().is_ok_and(|r| r.iter().any(|x| x.id == room))).await;
    net.set_online(&bob_onion, false);
    alice.app.send_room_text(&room, "first").await.unwrap();
    alice.app.send_text(&bob_in_alice, "plain").await.unwrap();
    alice.app.send_room_text(&room, "second").await.unwrap();
    net.set_online(&bob_onion, true);
    eventually("both temp messages", || texts(&bob, &room).len() == 2).await;
    let got: Vec<_> = texts(&bob, &room).into_iter().map(|t| t.0).collect();
    assert_eq!(got, ["first", "second"]);
    eventually("plain arrives", || bob.app.messages(&alice_in_bob, None, 5).unwrap().len() == 2).await;
}

#[tokio::test]
async fn a_temp_group_room_is_memory_only_and_takes_invites() {
    let net = MemNetwork::new();
    let (alice, bob, carol) = (peer(&net, "Alice").await, peer(&net, "Bob").await, peer(&net, "Carol").await);
    let room = alice.app.create_temp_room("Heist", true).unwrap();
    assert!(alice.app.rooms().unwrap()[0].temp);
    assert!(!alice.app.rooms().unwrap()[0].direct);
    join(&bob, &alice, &room, "bob").await;
    eventually("bob joined", || fully_connected(&bob, 1) && fully_connected(&alice, 1)).await;
    join(&carol, &bob, &room, "carol").await;
    eventually("mesh", || fully_connected(&alice, 2) && fully_connected(&bob, 2) && fully_connected(&carol, 2)).await;
    assert!(carol.app.rooms().unwrap()[0].temp, "the joiner learns it is temporary");
    assert_eq!(carol.app.rooms().unwrap()[0].name, "Heist");

    let mid = carol.app.send_room_text(&room, "act at dawn").await.unwrap();
    eventually("both read it", || texts(&alice, &room).len() == 1 && texts(&bob, &room).len() == 1).await;
    eventually("delivered once everyone has it", || carol.app.messages(&room, None, 5).unwrap().iter().any(|m| m.id == mid && m.status == Status::Delivered)).await;
    assert_eq!(texts(&alice, &room)[0].1, Some(contact_id(&alice, "Carol")));

    // Persistent rooms and temp rooms live side by side.
    let keep = alice.app.create_room("Keep", true).unwrap();
    let rooms = alice.app.rooms().unwrap();
    assert_eq!(rooms.iter().filter(|r| r.temp).count(), 1);
    assert!(rooms.iter().any(|r| r.id == keep && !r.temp));

    // Clearing the chat forgets the messages but keeps the room.
    alice.app.delete_chat(&room).unwrap();
    assert!(alice.app.messages(&room, None, 5).unwrap().is_empty());
    assert!(alice.app.rooms().unwrap().iter().any(|r| r.id == room));

    // Locking drops the room and its messages, on disk only the normal room remains.
    carol.app.lock().await.unwrap();
    nothing_temporary_on_disk(&carol, &room);
    alice.app.lock().await.unwrap();
    let on_disk = Profile::unlock(&alice.profile_dir(), PASS).unwrap();
    assert_eq!(on_disk.engine.rooms().len(), 1);
    assert_eq!(on_disk.engine.rooms()[0].id, guft_core_key(&keep));
    assert!(on_disk.history.page(&room, None, 10).unwrap().is_empty());
}

/// The engine stores rooms under the bare hex id; the app prefixes it with `r-`.
fn guft_core_key(chat: &str) -> String {
    chat.strip_prefix("r-").unwrap().to_owned()
}
