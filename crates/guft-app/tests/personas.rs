//! Separate identities: a room can be made or joined as a brand-new identity (new keys, new
//! Tor address, its own name) that nothing links to your main one.

use std::sync::Arc;
use std::time::Duration;

use guft_app::{AppError, AppOptions, Body, Hub, MemBackend, MAX_PERSONAS};
use guft_net::mem::MemNetwork;
use guft_net::NetConfig;
use tempfile::TempDir;
use tokio::time::timeout;

const PASS: &str = "correct horse battery";
const HOUR: Duration = Duration::from_secs(3600);

struct User {
    hub: Hub<MemBackend>,
    dir: TempDir,
}

impl User {
    fn profile_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("profile")
    }
}

async fn user(net: &Arc<MemNetwork>, name: &str) -> User {
    let dir = tempfile::tempdir().unwrap();
    let opts = AppOptions {
        idle_timeout: Duration::from_secs(600),
        tick: Duration::from_millis(20),
        retry_base: Duration::from_millis(20),
        retry_max: Duration::from_millis(100),
        max_attempts: 300,
    };
    let cfg = NetConfig { max_jitter: Duration::from_millis(1), ack_timeout: Duration::from_secs(2), cover_mean: None, ..NetConfig::default() };
    let net = net.clone();
    let hub = Hub::new(dir.path().join("profile"), opts, move |_| MemBackend::new(net.clone(), cfg.clone()));
    hub.create_profile(PASS, name).await.unwrap();
    User { hub, dir }
}

async fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
    timeout(Duration::from_secs(15), async {
        while !cond() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for: {what}"));
}

fn texts(u: &User, room: &str) -> Vec<String> {
    let mut v: Vec<_> = u
        .hub
        .messages(room, None, 100)
        .unwrap()
        .into_iter()
        .filter_map(|m| match m.body {
            Body::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    v.reverse();
    v
}

fn contact_named(u: &User, name: &str) -> guft_app::ContactView {
    u.hub.contacts().unwrap().into_iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no contact {name}"))
}

/// `joiner` uses a room invite from `inviter` (optionally as a new identity) and waits until the
/// room shows up with `members` other people connected.
async fn join(joiner: &User, inviter: &User, room: &str, identity: Option<&str>, members: usize) {
    let (invite, code) = inviter.hub.new_room_invite(room, "friend", HOUR).unwrap();
    joiner.hub.add_contact(&invite, &code, identity).await.unwrap();
    eventually("joined and connected", || joiner.hub.rooms().is_ok_and(|r| r.iter().any(|x| x.members.len() == members && x.members.iter().all(|m| m.connected)))).await;
}

#[tokio::test]
async fn a_room_made_as_a_new_identity_shares_nothing_with_your_main_one() {
    let net = MemNetwork::new();
    let (alice, bob) = (user(&net, "Alice").await, user(&net, "Bob").await);

    // Alice also knows Bob as herself, in an ordinary chat.
    let (invite, code) = alice.hub.new_invite("bob", HOUR).unwrap();
    bob.hub.add_contact(&invite, &code, None).await.unwrap();
    eventually("main contacts", || bob.hub.contacts().is_ok_and(|c| c.len() == 1)).await;

    let room = alice.hub.create_room("Secret", true, false, Some("Ghost")).await.unwrap();
    assert!(room.starts_with("p1~"), "{room}");
    let mine = alice.hub.rooms().unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].identity.as_deref(), Some("Ghost"));
    assert!(mine[0].mine);

    join(&bob, &alice, &room, None, 1).await;

    // Bob sees two different people: Alice, and Ghost, with different keys and addresses.
    let (a, g) = (contact_named(&bob, "Alice"), contact_named(&bob, "Ghost"));
    assert_ne!(a.id, g.id, "different identity keys");
    assert_ne!(a.onion, g.onion, "different Tor addresses");
    assert_eq!(a.onion, alice.hub.my_onion().unwrap());
    assert_eq!(g.onion, alice.hub.persona_onion(1).unwrap());
    // Nothing in the room says Alice.
    let room_on_bob = &bob.hub.rooms().unwrap()[0];
    assert_eq!(room_on_bob.members.len(), 1);
    assert_eq!(room_on_bob.members[0].name, "Ghost");
    assert_eq!(room_on_bob.creator, g.id);

    // Messages work both ways, and carry the persona's identity.
    let sent = alice.hub.send_room_text(&room, "hello from the other me").await.unwrap();
    assert!(sent < (1 << 53), "ids must stay safe for JavaScript numbers");
    eventually("bob reads it", || texts(&bob, &room_on_bob.id).len() == 1).await;
    assert_eq!(bob.hub.messages(&room_on_bob.id, None, 5).unwrap()[0].sender.as_deref(), Some(g.id.as_str()));
    bob.hub.send_room_text(&room_on_bob.id, "hi ghost").await.unwrap();
    eventually("ghost reads it", || texts(&alice, &room).len() == 2).await;
    assert_eq!(alice.hub.messages(&room, None, 5).unwrap()[0].chat, room);
}

#[tokio::test]
async fn joining_as_a_new_identity_hides_you_from_the_room_and_the_room_from_your_contacts() {
    let net = MemNetwork::new();
    let (alice, carol) = (user(&net, "Alice").await, user(&net, "Carol").await);
    let room = alice.hub.create_room("Club", true, false, None).await.unwrap();

    join(&carol, &alice, &room, Some("Mask"), 1).await;
    eventually("alice sees the member", || alice.hub.rooms().unwrap()[0].members.len() == 1).await;

    // To the room she is Mask, and Alice's roster never mentions Carol.
    assert_eq!(alice.hub.rooms().unwrap()[0].members[0].name, "Mask");
    let carols = carol.hub.rooms().unwrap();
    assert_eq!(carols.len(), 1);
    assert_eq!(carols[0].identity.as_deref(), Some("Mask"));
    assert!(carols[0].id.starts_with("p1~"));

    // The room's people are not in Carol's own contacts or chats.
    assert!(carol.hub.contacts().unwrap().is_empty());
    assert!(carol.hub.chats().unwrap().is_empty());
    assert_eq!(carol.hub.my_name().unwrap(), "Carol");
    assert_ne!(carol.hub.persona_onion(1).unwrap(), carol.hub.my_onion().unwrap());

    // The same Carol can also be a plain member of another room, as herself.
    let other = alice.hub.create_room("Other", true, false, None).await.unwrap();
    let (invite, code) = alice.hub.new_room_invite(&other, "carol", HOUR).unwrap();
    carol.hub.add_contact(&invite, &code, None).await.unwrap();
    eventually("two rooms", || carol.hub.rooms().is_ok_and(|r| r.len() == 2 && r.iter().filter(|x| x.identity.is_none()).count() == 1)).await;
    assert_eq!(carol.hub.contacts().unwrap().len(), 1, "only the room where she is Carol made a contact");
}

#[tokio::test]
async fn only_room_invites_can_be_used_with_a_new_identity_and_failures_leave_nothing_behind() {
    let net = MemNetwork::new();
    let (alice, carol) = (user(&net, "Alice").await, user(&net, "Carol").await);

    // A plain (non-room) invite cannot become a persona.
    let (invite, code) = alice.hub.new_invite("x", HOUR).unwrap();
    let err = carol.hub.add_contact(&invite, &code, Some("Mask")).await.unwrap_err();
    assert!(err.to_string().contains("room invite"), "{err}");

    // Names must be real names, and nothing is created for a refused one.
    let room = alice.hub.create_room("Club", true, false, None).await.unwrap();
    let (invite, _code) = alice.hub.new_room_invite(&room, "c", HOUR).unwrap();
    assert!(carol.hub.add_contact(&invite, "aaaa-bbbb-cccc", Some("   ")).await.is_err());
    assert!(carol.hub.add_contact(&invite, "aaaa-bbbb-cccc", Some("Ma\u{202E}sk")).await.is_err());
    assert!(carol.hub.add_contact(&invite, "aaaa-bbbb-cccc", Some(&"x".repeat(100))).await.is_err());

    assert!(carol.hub.persona_ids().is_empty());
    let personas = carol.profile_dir().join("personas");
    assert!(!personas.exists() || std::fs::read_dir(personas).unwrap().count() == 0, "no files of a failed identity remain");
    assert!(carol.hub.rooms().unwrap().is_empty());
}

#[tokio::test]
async fn files_unread_counts_and_deleting_work_in_a_room_with_its_own_identity() {
    let net = MemNetwork::new();
    let (alice, bob) = (user(&net, "Alice").await, user(&net, "Bob").await);
    let room = alice.hub.create_room("Secret", true, false, Some("Ghost")).await.unwrap();
    join(&bob, &alice, &room, None, 1).await;
    let bob_room = bob.hub.rooms().unwrap()[0].id.clone();

    let data: Vec<u8> = (0..30_000u32).map(|i| (i * 31 % 251) as u8).collect();
    let sent = alice.hub.send_room_file(&room, "plan.bin", data.clone()).await.unwrap();
    eventually("bob has the file", || bob.hub.messages(&bob_room, None, 5).is_ok_and(|m| m.len() == 1)).await;
    let theirs = bob.hub.messages(&bob_room, None, 5).unwrap().remove(0);
    assert_eq!(bob.hub.file_bytes(theirs.id).unwrap().1, data);
    assert_eq!(alice.hub.file_bytes(sent).unwrap().1, data);
    let out = tempfile::tempdir().unwrap();
    assert_eq!(std::fs::read(bob.hub.save_file(theirs.id, out.path()).unwrap()).unwrap(), data);

    assert_eq!(bob.hub.rooms().unwrap()[0].unread, 1);
    bob.hub.mark_read(&bob_room).unwrap();
    assert_eq!(bob.hub.rooms().unwrap()[0].unread, 0);

    // Alice deletes her own copy by the id the UI knows.
    alice.hub.delete_message(sent).unwrap();
    assert!(alice.hub.messages(&room, None, 5).unwrap().is_empty());
    assert!(alice.hub.file_bytes(sent).is_err());
    // A message id that belongs to no identity is refused.
    assert!(alice.hub.delete_message((5 << 42) + 1).is_err());

    // Your main contacts cannot be dragged into a room that has its own identity.
    let (invite, code) = bob.hub.new_invite("alice", HOUR).unwrap();
    alice.hub.add_contact(&invite, &code, None).await.unwrap();
    let bob_in_alice = contact_named(&alice, "Bob").id;
    assert!(alice.hub.add_contact_to_room(&room, &bob_in_alice).is_err());
}

#[tokio::test]
async fn identities_come_back_after_a_lock_with_their_rooms_and_history() {
    let net = MemNetwork::new();
    let (alice, bob) = (user(&net, "Alice").await, user(&net, "Bob").await);
    let room = alice.hub.create_room("Secret", true, false, Some("Ghost")).await.unwrap();
    join(&bob, &alice, &room, None, 1).await;
    alice.hub.send_room_text(&room, "before the lock").await.unwrap();
    let onion = alice.hub.persona_onion(1).unwrap();

    alice.hub.lock().await.unwrap();
    assert!(matches!(alice.hub.rooms(), Err(AppError::Locked)));
    assert!(alice.hub.persona_onion(1).is_none(), "the separate identity is locked too");
    assert!(!net.is_registered(&onion), "and offline");

    alice.hub.unlock(PASS).await.unwrap();
    eventually("the identity is back", || alice.hub.rooms().is_ok_and(|r| r.len() == 1 && r[0].identity.as_deref() == Some("Ghost"))).await;
    assert_eq!(alice.hub.persona_onion(1).unwrap(), onion, "same address as before");
    assert_eq!(texts(&alice, &room), ["before the lock"]);
    // Still works end to end.
    alice.hub.send_room_text(&room, "after the lock").await.unwrap();
    let bob_room = bob.hub.rooms().unwrap()[0].id.clone();
    eventually("bob gets both", || texts(&bob, &bob_room).len() == 2).await;
}

#[tokio::test]
async fn leaving_erases_the_identity_completely() {
    let net = MemNetwork::new();
    let (alice, bob) = (user(&net, "Alice").await, user(&net, "Bob").await);
    let room = alice.hub.create_room("Secret", true, false, Some("Ghost")).await.unwrap();
    join(&bob, &alice, &room, None, 1).await;
    let onion = alice.hub.persona_onion(1).unwrap();
    assert!(net.is_registered(&onion));
    let dir = alice.profile_dir().join("personas/1");
    assert!(dir.exists());

    alice.hub.leave_room(&room).unwrap();
    eventually("identity erased", || alice.hub.persona_ids().is_empty() && !dir.exists()).await;
    assert!(!net.is_registered(&onion), "and its Tor address is offline for good");
    assert!(alice.hub.rooms().unwrap().is_empty());
    // The other side sees the member leave.
    eventually("bob's roster drops it", || bob.hub.rooms().unwrap()[0].members.is_empty()).await;

    // Gone for good: a later unlock does not bring it back.
    alice.hub.lock().await.unwrap();
    alice.hub.unlock(PASS).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(alice.hub.persona_ids().is_empty());
}

#[tokio::test]
async fn being_removed_from_a_room_erases_the_identity_you_joined_it_with() {
    let net = MemNetwork::new();
    let (alice, carol) = (user(&net, "Alice").await, user(&net, "Carol").await);
    let room = alice.hub.create_room("Club", true, false, None).await.unwrap();
    join(&carol, &alice, &room, Some("Mask"), 1).await;
    eventually("alice sees mask", || alice.hub.rooms().unwrap()[0].members.len() == 1).await;
    let mask = alice.hub.rooms().unwrap()[0].members[0].id.clone();

    alice.hub.remove_room_member(&room, &mask).unwrap();
    eventually("carol's identity is erased", || carol.hub.persona_ids().is_empty()).await;
    assert!(carol.hub.rooms().unwrap().is_empty());
    assert!(!carol.profile_dir().join("personas/1").exists());
}

#[tokio::test]
async fn two_rooms_two_identities_cannot_be_linked() {
    let net = MemNetwork::new();
    let (alice, bob) = (user(&net, "Alice").await, user(&net, "Bob").await);
    let a = alice.hub.create_room("Room A", true, false, Some("Ann")).await.unwrap();
    let b = alice.hub.create_room("Room B", true, false, Some("Bea")).await.unwrap();
    join(&bob, &alice, &a, None, 1).await;
    eventually("second", || true).await;
    let (invite, code) = alice.hub.new_room_invite(&b, "bob", HOUR).unwrap();
    bob.hub.add_contact(&invite, &code, None).await.unwrap();
    eventually("bob is in both", || bob.hub.rooms().is_ok_and(|r| r.len() == 2)).await;

    let (ann, bea) = (contact_named(&bob, "Ann"), contact_named(&bob, "Bea"));
    assert_ne!(ann.id, bea.id);
    assert_ne!(ann.onion, bea.onion);
    let main = alice.hub.my_onion().unwrap();
    assert!(![&ann.onion, &bea.onion].contains(&&main));
    assert_eq!(alice.hub.persona_ids(), vec![1, 2]);
    // Each room's creator is its own identity.
    let rooms = bob.hub.rooms().unwrap();
    for r in &rooms {
        let want = if r.name == "Room A" { &ann.id } else { &bea.id };
        assert_eq!(&r.creator, want);
    }
}

#[tokio::test]
async fn temporary_rooms_can_have_their_own_identity_too() {
    let net = MemNetwork::new();
    let (alice, bob) = (user(&net, "Alice").await, user(&net, "Bob").await);
    let room = alice.hub.create_room("Ephemeral", true, true, Some("Smoke")).await.unwrap();
    let r = &alice.hub.rooms().unwrap()[0];
    assert!(r.temp && r.identity.as_deref() == Some("Smoke"));
    join(&bob, &alice, &room, None, 1).await;
    alice.hub.send_room_text(&room, "this will not be saved").await.unwrap();
    let bob_room = bob.hub.rooms().unwrap()[0].id.clone();
    eventually("bob reads it", || texts(&bob, &bob_room).len() == 1).await;

    alice.hub.lock().await.unwrap();
    alice.hub.unlock(PASS).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(alice.hub.rooms().unwrap().is_empty(), "the temporary room did not survive the lock");
}

#[tokio::test]
async fn there_is_a_limit_on_separate_identities() {
    let net = MemNetwork::new();
    let alice = user(&net, "Alice").await;
    for i in 0..MAX_PERSONAS {
        alice.hub.create_room(&format!("Room {i}"), true, false, Some(&format!("Alias {i}"))).await.unwrap();
    }
    let err = alice.hub.create_room("One more", true, false, Some("Too many")).await.unwrap_err();
    assert!(err.to_string().contains("too many"), "{err}");
    assert_eq!(alice.hub.persona_ids().len(), MAX_PERSONAS);
    // Leaving one frees a place.
    let first = alice.hub.rooms().unwrap().remove(0).id;
    alice.hub.leave_room(&first).unwrap();
    eventually("one erased", || alice.hub.persona_ids().len() == MAX_PERSONAS - 1).await;
    alice.hub.create_room("Fits now", true, false, Some("Alias new")).await.unwrap();
}
