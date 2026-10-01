use std::sync::Arc;
use std::time::Duration;

use guft_app::{App, AppOptions, Body, Event, MemBackend, Status};
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

#[tokio::test]
async fn people_join_through_invites_and_everyone_gets_connected() {
    let net = MemNetwork::new();
    let (alice, bob, carol) = (peer(&net, "Alice").await, peer(&net, "Bob").await, peer(&net, "Carol").await);
    let room = alice.app.create_room("Friends", true).unwrap();

    join(&bob, &alice, &room, "bob").await;
    eventually("bob joined", || fully_connected(&bob, 1) && fully_connected(&alice, 1)).await;
    assert_eq!(bob.app.rooms().unwrap()[0].name, "Friends");
    assert_eq!(bob.app.rooms().unwrap()[0].id, room, "everyone uses the same room id");

    // Carol uses an invite from Bob, not Alice; the introductions still connect her to Alice.
    join(&carol, &bob, &room, "carol").await;
    eventually("mesh complete", || fully_connected(&alice, 2) && fully_connected(&bob, 2) && fully_connected(&carol, 2)).await;

    let names = |p: &Peer| {
        let mut n: Vec<_> = p.app.rooms().unwrap()[0].members.iter().map(|m| m.name.clone()).collect();
        n.sort();
        n
    };
    assert_eq!(names(&alice), ["Bob", "Carol"]);
    assert_eq!(names(&carol), ["Alice", "Bob"]);
}

#[tokio::test]
async fn room_messages_arrive_with_sender_and_ticks_wait_for_everyone() {
    let net = MemNetwork::new();
    let (mut alice, bob, carol) = (peer(&net, "Alice").await, peer(&net, "Bob").await, peer(&net, "Carol").await);
    let room = alice.app.create_room("Trio", true).unwrap();
    join(&bob, &alice, &room, "bob").await;
    join(&carol, &alice, &room, "carol").await;
    eventually("mesh", || fully_connected(&alice, 2) && fully_connected(&bob, 2) && fully_connected(&carol, 2)).await;

    let mid = carol.app.send_room_text(&room, "hi room").await.unwrap();
    eventually("both got it", || texts(&alice, &room).len() == 1 && texts(&bob, &room).len() == 1).await;
    let carol_in_alice = contact_id(&alice, "Carol");
    let carol_in_bob = contact_id(&bob, "Carol");
    assert_eq!(texts(&alice, &room), [("hi room".to_string(), Some(carol_in_alice))]);
    assert_eq!(texts(&bob, &room), [("hi room".to_string(), Some(carol_in_bob))]);
    eventually("sender sees delivered", || carol.app.messages(&room, None, 5).unwrap().iter().any(|m| m.id == mid && m.status == Status::Delivered)).await;

    // Unread counts and the room appear in the room list.
    assert_eq!(alice.app.rooms().unwrap()[0].unread, 1);
    alice.app.mark_read(&room).unwrap();
    assert_eq!(alice.app.rooms().unwrap()[0].unread, 0);
    wait_event(&mut alice, "message event", |e| matches!(e, Event::Message { chat, .. } if *chat == room)).await;

    // Files go to everyone too.
    carol.app.send_room_file(&room, "pic.png", vec![7; 150_000]).await.unwrap();
    eventually("file at bob", || bob.app.messages(&room, None, 10).unwrap().iter().any(|m| matches!(&m.body, Body::File { size, .. } if *size == 150_000))).await;
}

#[tokio::test]
async fn an_offline_member_catches_up_and_ticks_reflect_it() {
    let net = MemNetwork::new();
    let (alice, bob, carol) = (peer(&net, "Alice").await, peer(&net, "Bob").await, peer(&net, "Carol").await);
    let room = alice.app.create_room("Trio", true).unwrap();
    join(&bob, &alice, &room, "bob").await;
    join(&carol, &alice, &room, "carol").await;
    eventually("mesh", || fully_connected(&alice, 2) && fully_connected(&bob, 2) && fully_connected(&carol, 2)).await;

    let bob_onion = bob.app.my_onion().unwrap();
    net.set_online(&bob_onion, false);
    let mid = alice.app.send_room_text(&room, "while bob is away").await.unwrap();
    eventually("carol got it", || texts(&carol, &room).len() == 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let status = |p: &Peer| p.app.messages(&room, None, 5).unwrap().iter().find(|m| m.id == mid).unwrap().status;
    assert_eq!(status(&alice), Status::Sent, "one member has it, one does not: not delivered yet");
    assert!(texts(&bob, &room).is_empty());

    net.set_online(&bob_onion, true);
    eventually("bob caught up", || texts(&bob, &room).len() == 1).await;
    eventually("now delivered", || status(&alice) == Status::Delivered).await;
}

#[tokio::test]
async fn removed_members_stop_receiving_and_lose_the_room() {
    let net = MemNetwork::new();
    let (alice, bob, carol) = (peer(&net, "Alice").await, peer(&net, "Bob").await, peer(&net, "Carol").await);
    let room = alice.app.create_room("Trio", true).unwrap();
    join(&bob, &alice, &room, "bob").await;
    join(&carol, &alice, &room, "carol").await;
    eventually("mesh", || fully_connected(&alice, 2) && fully_connected(&bob, 2) && fully_connected(&carol, 2)).await;

    // Non-creators cannot remove or rename.
    let carol_in_bob = contact_id(&bob, "Carol");
    assert!(bob.app.remove_room_member(&room, &carol_in_bob).is_err());
    assert!(bob.app.rename_room(&room, "Mine").is_err());

    alice.app.rename_room(&room, "Renamed").unwrap();
    eventually("renamed everywhere", || bob.app.rooms().unwrap()[0].name == "Renamed" && carol.app.rooms().unwrap()[0].name == "Renamed").await;

    let carol_in_alice = contact_id(&alice, "Carol");
    alice.app.remove_room_member(&room, &carol_in_alice).unwrap();
    eventually("carol dropped the room", || carol.app.rooms().unwrap().is_empty()).await;
    eventually("bob updated", || bob.app.rooms().unwrap()[0].members.len() == 1).await;

    alice.app.send_room_text(&room, "carol cannot read this").await.unwrap();
    eventually("bob got it", || texts(&bob, &room).len() == 1).await;
    assert!(carol.app.messages(&room, None, 5).unwrap().is_empty());

    // Bob leaves; Alice's roster empties.
    bob.app.leave_room(&room).unwrap();
    eventually("alice sees bob leave", || alice.app.rooms().unwrap()[0].members.is_empty()).await;
    assert!(bob.app.rooms().unwrap().is_empty());
}

#[tokio::test]
async fn existing_contacts_can_be_added_and_closed_rooms_restrict_invites() {
    let net = MemNetwork::new();
    let (alice, bob, carol) = (peer(&net, "Alice").await, peer(&net, "Bob").await, peer(&net, "Carol").await);

    // Alice and Carol know each other from before; Bob joins a closed room.
    let (invite, code) = alice.app.new_invite("carol", HOUR).unwrap();
    carol.app.add_contact(&invite, &code).await.unwrap();
    eventually("contacts", || alice.app.contacts().unwrap().len() == 1).await;

    let room = alice.app.create_room("Closed", false).unwrap();
    join(&bob, &alice, &room, "bob").await;
    eventually("bob in", || fully_connected(&bob, 1)).await;
    assert!(bob.app.new_room_invite(&room, "x", HOUR).is_err(), "members of a closed room cannot invite");

    let carol_in_alice = contact_id(&alice, "Carol");
    alice.app.add_contact_to_room(&room, &carol_in_alice).unwrap();
    eventually("everyone connected", || fully_connected(&alice, 2) && fully_connected(&bob, 2) && fully_connected(&carol, 2)).await;
    assert!(alice.app.add_contact_to_room(&room, &carol_in_alice).is_err(), "already a member");
}
