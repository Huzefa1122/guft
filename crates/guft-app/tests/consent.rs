use std::sync::Arc;
use std::time::Duration;

use guft_app::{App, AppOptions, Body, MemBackend};
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

use guft_store::Profile;

fn files_contain(p: &Peer, needle: &[u8]) -> bool {
    let mut stack = vec![p.profile_dir()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if std::fs::read(&path).unwrap().windows(needle.len()).any(|w| w == needle) {
                return true;
            }
        }
    }
    false
}

/// Alice is in a room with Bob, and also knows Carol (who knows neither Bob nor the room).
async fn alice_bob_room_and_carol(net: &Arc<MemNetwork>) -> (Peer, Peer, Peer, String) {
    let (mut alice, mut bob, mut carol) = (peer(net, "Alice").await, peer(net, "Bob").await, peer(net, "Carol").await);
    befriend(&mut bob, &mut alice).await;
    let (invite, code) = alice.app.new_invite("carol", HOUR).unwrap();
    carol.app.add_contact(&invite, &code).await.unwrap();
    eventually("alice knows carol", || alice.app.contacts().is_ok_and(|c| c.len() == 2)).await;
    let room = alice.app.create_room("Private club", true).unwrap();
    alice.app.add_contact_to_room(&room, &contact_id(&alice, "Bob")).unwrap();
    accept(&bob, &room).await;
    eventually("bob in", || fully_connected(&bob, 1) && fully_connected(&alice, 1)).await;
    let _ = &mut carol;
    (alice, bob, carol, room)
}

#[tokio::test]
async fn a_contact_cannot_put_you_in_a_room_without_your_yes() {
    let net = MemNetwork::new();
    let (alice, bob, carol, room) = alice_bob_room_and_carol(&net).await;
    alice.app.add_contact_to_room(&room, &contact_id(&alice, "Carol")).unwrap();

    eventually("the invitation arrives", || carol.app.room_invitations().is_ok_and(|v| v.len() == 1)).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let inv = carol.app.room_invitations().unwrap().remove(0);
    assert_eq!((inv.name.as_str(), inv.from_name.as_str(), inv.temp), ("Private club", "Alice", false));
    assert!(inv.members.contains(&"Bob".to_string()) && inv.members.contains(&"Alice".to_string()));

    // Nothing has happened: Carol is in no room, knows nobody new, and nobody learned of Carol.
    assert!(carol.app.rooms().unwrap().is_empty());
    assert_eq!(carol.app.contacts().unwrap().len(), 1, "no contact was created with the room's other members");
    assert_eq!(bob.app.contacts().unwrap().len(), 1, "Bob has not been introduced to Carol");
    assert_eq!(bob.app.rooms().unwrap()[0].members.len(), 1, "Bob's roster does not include Carol");
    // Messages Bob sends reach Alice only; Carol sees nothing.
    bob.app.send_room_text(&room, "members only").await.unwrap();
    eventually("alice got it", || texts(&alice, &room).len() == 1).await;
    assert!(carol.app.rooms().unwrap().is_empty());

    // Saying yes joins and connects her to everyone.
    let chat = carol.app.accept_room_invite(&room).unwrap();
    assert_eq!(chat, room);
    assert!(carol.app.room_invitations().unwrap().is_empty());
    eventually("everyone connected", || fully_connected(&alice, 2) && fully_connected(&bob, 2) && fully_connected(&carol, 2)).await;
    carol.app.send_room_text(&room, "hello all").await.unwrap();
    eventually("both read it", || texts(&alice, &room).len() == 2 && texts(&bob, &room).len() == 2).await;
}

#[tokio::test]
async fn declining_tells_nobody_about_you_and_the_inviter_drops_you() {
    let net = MemNetwork::new();
    let (alice, bob, carol, room) = alice_bob_room_and_carol(&net).await;
    alice.app.add_contact_to_room(&room, &contact_id(&alice, "Carol")).unwrap();
    eventually("the invitation arrives", || carol.app.room_invitations().is_ok_and(|v| v.len() == 1)).await;
    assert_eq!(alice.app.rooms().unwrap()[0].members.len(), 2);

    carol.app.decline_room_invite(&room).unwrap();
    assert!(carol.app.room_invitations().unwrap().is_empty());
    assert!(carol.app.rooms().unwrap().is_empty());
    eventually("alice stops counting carol", || alice.app.rooms().unwrap()[0].members.len() == 1).await;
    assert_eq!(carol.app.contacts().unwrap().len(), 1);
    assert_eq!(bob.app.contacts().unwrap().len(), 1);
    // Declining twice, or accepting a gone invitation, is an error and changes nothing.
    assert!(carol.app.accept_room_invite(&room).is_err());
    assert!(carol.app.rooms().unwrap().is_empty());
}

#[tokio::test]
async fn a_room_invite_you_used_joins_at_once_but_only_for_that_room() {
    let net = MemNetwork::new();
    let (alice, mut carol) = (peer(&net, "Alice").await, peer(&net, "Carol").await);
    let first = alice.app.create_room("First", true).unwrap();
    let (invite, code) = alice.app.new_room_invite(&first, "carol", HOUR).unwrap();
    carol.app.add_contact(&invite, &code).await.unwrap();
    eventually("carol joined without a second yes", || fully_connected(&carol, 1) && fully_connected(&alice, 1)).await;
    assert!(carol.app.room_invitations().unwrap().is_empty());

    // Alice now tries a different room: that one waits for Carol's answer.
    let second = alice.app.create_room("Second", true).unwrap();
    alice.app.add_contact_to_room(&second, &contact_id(&alice, "Carol")).unwrap();
    eventually("second invitation waits", || carol.app.room_invitations().is_ok_and(|v| v.len() == 1)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(carol.app.rooms().unwrap().len(), 1, "still only the first room");
    let _ = &mut carol;
}

#[tokio::test]
async fn pending_invitations_are_capped_and_follow_the_contact() {
    let net = MemNetwork::new();
    let (mut alice, mut carol) = (peer(&net, "Alice").await, peer(&net, "Carol").await);
    befriend(&mut carol, &mut alice).await;
    let carol_id = contact_id(&alice, "Carol");
    for i in 0..8 {
        let r = alice.app.create_room(&format!("Spam {i}"), true).unwrap();
        alice.app.add_contact_to_room(&r, &carol_id).unwrap();
    }
    eventually("some arrive", || carol.app.room_invitations().is_ok_and(|v| v.len() >= 5)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(carol.app.room_invitations().unwrap().len(), 5, "one contact can only leave a few invitations waiting");
    assert!(carol.app.rooms().unwrap().is_empty());

    // Removing the contact clears what they left.
    carol.app.remove_contact(&contact_id(&carol, "Alice")).unwrap();
    assert!(carol.app.room_invitations().unwrap().is_empty());
    let _ = &mut alice;
}

#[tokio::test]
async fn invitations_survive_a_lock_but_temporary_ones_do_not_and_leave_no_trace() {
    let net = MemNetwork::new();
    let (alice, _bob, carol, room) = alice_bob_room_and_carol(&net).await;
    let carol_id = contact_id(&alice, "Carol");
    alice.app.add_contact_to_room(&room, &carol_id).unwrap();
    let temp = alice.app.create_temp_room("TEMPINV-MARKER-secret-room", true).unwrap();
    alice.app.add_contact_to_room(&temp, &carol_id).unwrap();
    eventually("both invitations", || carol.app.room_invitations().is_ok_and(|v| v.len() == 2)).await;
    let list = carol.app.room_invitations().unwrap();
    assert_eq!(list.iter().filter(|i| i.temp).count(), 1);

    // The temporary one is only in memory: not in the database, not in any file.
    carol.app.lock().await.unwrap();
    assert!(!files_contain(&carol, b"TEMPINV-MARKER"));
    let on_disk = Profile::unlock(&carol.profile_dir(), PASS).unwrap();
    let rooms: Vec<String> = on_disk.history.pending_rooms().unwrap().into_iter().map(|r| r.0).collect();
    assert_eq!(rooms.len(), 1, "only the ordinary room invitation is stored");
    drop(on_disk);

    carol.app.unlock(PASS).await.unwrap();
    let after = carol.app.room_invitations().unwrap();
    assert_eq!(after.len(), 1);
    assert!(!after[0].temp);
    assert!(carol.app.accept_room_invite(&temp).is_err(), "the temporary invitation did not survive the lock");
    carol.app.accept_room_invite(&after[0].room).unwrap();
    assert_eq!(carol.app.rooms().unwrap().len(), 1);
}

#[tokio::test]
async fn a_one_to_one_temporary_chat_still_opens_in_one_tap() {
    let net = MemNetwork::new();
    let (mut alice, mut bob) = (peer(&net, "Alice").await, peer(&net, "Bob").await);
    befriend(&mut bob, &mut alice).await;
    let room = alice.app.start_temp_chat(&contact_id(&alice, "Bob")).unwrap();
    eventually("bob has it with no prompt", || bob.app.rooms().is_ok_and(|r| r.len() == 1 && r[0].direct)).await;
    assert!(bob.app.room_invitations().unwrap().is_empty());
    assert_eq!(bob.app.rooms().unwrap()[0].id, room);
    let _ = &mut alice;
}
