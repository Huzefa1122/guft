use std::collections::{BTreeMap, VecDeque};

use guft_core::rooms::{Delivery, RoomEvent};
use guft_core::{Engine, Payload};

const NOW: u64 = 1_800_000_000;
const HOUR: u64 = 3600;

fn onion(c: char) -> String {
    format!("{}.onion", c.to_string().repeat(56))
}

/// A little network of engines that behaves like the app: decrypts, applies room
/// effects, and queues the follow-up messages (hello, auth key, room invite, introductions).
struct World {
    engines: BTreeMap<&'static str, Engine>,
    inbox: Vec<(String, Delivery)>,
    events: Vec<(String, RoomEvent)>,
}

impl World {
    fn new(names: &[&'static str]) -> Self {
        let mut engines = BTreeMap::new();
        for (i, n) in names.iter().enumerate() {
            let mut e = Engine::create(n).unwrap();
            e.set_onion(&onion((b'a' + i as u8) as char)).unwrap();
            engines.insert(*n, e);
        }
        Self {
            engines,
            inbox: vec![],
            events: vec![],
        }
    }

    fn id(&self, name: &str) -> String {
        self.engines[name].my_id().to_owned()
    }

    fn name_of(&self, id: &str) -> &'static str {
        self.engines
            .iter()
            .find(|(_, e)| e.my_id() == id)
            .map(|(n, _)| *n)
            .expect("known id")
    }

    fn e(&mut self, name: &str) -> &mut Engine {
        self.engines.get_mut(name).unwrap()
    }

    /// Deliver one payload and everything it sets off.
    fn send(&mut self, from: &str, to_id: &str, payload: Payload) {
        let mut queue: VecDeque<(String, String, Payload)> = VecDeque::new();
        queue.push_back((from.to_owned(), to_id.to_owned(), payload));
        while let Some((from, to_id, payload)) = queue.pop_front() {
            let to = self.name_of(&to_id).to_owned();
            let frame = self.e(&from).encrypt(&to_id, &payload).expect("encrypt");
            let Ok(rec) = self.e(&to).decrypt(&frame, NOW) else {
                continue;
            };
            let sender_name = from.clone();
            let _ = sender_name;
            let e = self.engines.get_mut(to.as_str()).unwrap();
            if rec.new_contact {
                queue.push_back((
                    to.clone(),
                    rec.from.clone(),
                    e.auth_key_for(&rec.from).unwrap(),
                ));
                if let Some(room) = rec.joined_room {
                    let key = guft_core::rooms::room_key(&room);
                    queue.push_back((
                        to.clone(),
                        rec.from.clone(),
                        e.room_invite_for(&key, &rec.from).unwrap(),
                    ));
                }
            }
            let fx = e.on_room_payload(&rec.from, &rec.payload, NOW).unwrap();
            for (dest, p) in fx.send {
                queue.push_back((to.clone(), dest, p));
            }
            if let Some(d) = fx.deliver {
                self.inbox.push((to.clone(), d));
            }
            for ev in fx.events {
                self.events.push((to.clone(), ev));
            }
        }
    }

    /// A plain contact (no room): `joiner` uses an ordinary invite from `inviter`.
    fn befriend(&mut self, joiner: &'static str, inviter: &'static str) {
        let (invite, code) = self.e(inviter).new_invite(joiner, HOUR, NOW).unwrap();
        let cid = self.e(joiner).add_contact(&invite, &code, NOW).unwrap();
        let hello = self.e(joiner).hello_for(&cid).unwrap();
        let inviter_id = self.id(inviter);
        self.send(joiner, &inviter_id, hello);
    }

    /// `joiner` imports a room invite made by `inviter` and says hello.
    fn join(&mut self, joiner: &'static str, inviter: &'static str, room: &str) {
        let (invite, code) = self
            .e(inviter)
            .new_room_invite(room, joiner, HOUR, NOW)
            .unwrap();
        let cid = self.e(joiner).add_contact(&invite, &code, NOW).unwrap();
        let hello = self.e(joiner).hello_for(&cid).unwrap();
        let inviter_id = self.id(inviter);
        self.send(joiner, &inviter_id, hello);
    }

    fn say(&mut self, from: &'static str, room: &str, text: &str) {
        let id = guft_core::rooms::parse_room_key(room).unwrap();
        let to = self.e(from).room_recipients(room).unwrap();
        for t in to {
            self.send(
                from,
                &t,
                Payload::RoomText {
                    room: id,
                    text: text.into(),
                },
            );
        }
    }

    fn texts_for(&self, who: &str) -> Vec<(String, String)> {
        self.inbox
            .iter()
            .filter(|(w, _)| w == who)
            .map(|(_, d)| match &d.payload {
                Payload::RoomText { text, .. } => (self.name_of(&d.from).to_owned(), text.clone()),
                _ => panic!("not text"),
            })
            .collect()
    }

    fn member_names(&self, who: &str) -> Vec<String> {
        let mut v: Vec<_> = self.engines[who].rooms()[0]
            .members
            .iter()
            .map(|m| m.name.clone())
            .collect();
        v.sort();
        v
    }
}

#[test]
fn joining_through_one_invite_connects_everyone() {
    let mut w = World::new(&["Alice", "Bob", "Carol", "Dave"]);
    let room = w.e("Alice").create_room("Friends", true).unwrap();

    w.join("Bob", "Alice", &room);
    assert_eq!(w.member_names("Alice"), ["Bob"]);
    assert_eq!(w.member_names("Bob"), ["Alice"]);

    // Carol is invited by Bob (open room), and must end up connected to Alice too.
    w.join("Carol", "Bob", &room);
    assert_eq!(w.member_names("Alice"), ["Bob", "Carol"]);
    assert_eq!(w.member_names("Bob"), ["Alice", "Carol"]);
    assert_eq!(w.member_names("Carol"), ["Alice", "Bob"]);
    for who in ["Alice", "Bob", "Carol"] {
        let info = &w.engines[who].rooms()[0];
        assert_eq!(
            info.reachable, 2,
            "{who} must be able to message both others"
        );
    }

    // And Dave via Carol, who then needs introductions to two strangers.
    w.join("Dave", "Carol", &room);
    for who in ["Alice", "Bob", "Carol", "Dave"] {
        assert_eq!(w.engines[who].rooms()[0].members.len(), 3, "{who}");
        assert_eq!(
            w.engines[who].rooms()[0].reachable,
            3,
            "{who} must reach everyone"
        );
    }
    assert_eq!(w.engines["Dave"].rooms()[0].name, "Friends");
}

#[test]
fn messages_reach_every_member_with_the_right_sender() {
    let mut w = World::new(&["Alice", "Bob", "Carol"]);
    let room = w.e("Alice").create_room("Trio", true).unwrap();
    w.join("Bob", "Alice", &room);
    w.join("Carol", "Alice", &room);

    w.say("Carol", &room, "hi all");
    w.say("Bob", &room, "hello");
    assert_eq!(
        w.texts_for("Alice"),
        [
            ("Carol".to_string(), "hi all".to_string()),
            ("Bob".to_string(), "hello".to_string())
        ]
    );
    assert_eq!(
        w.texts_for("Bob"),
        [("Carol".to_string(), "hi all".to_string())]
    );
    assert_eq!(
        w.texts_for("Carol"),
        [("Bob".to_string(), "hello".to_string())]
    );

    let id = guft_core::rooms::parse_room_key(&room).unwrap();
    let file = Payload::RoomFile {
        room: id,
        name: "a.png".into(),
        data: vec![1; 200_000],
    };
    let to = w.e("Bob").room_recipients(&room).unwrap();
    for t in to {
        w.send("Bob", &t, file.clone());
    }
    assert!(w.inbox.iter().any(|(who, d)| who == "Carol"
        && matches!(&d.payload, Payload::RoomFile { data, .. } if data.len() == 200_000)));
}

#[test]
fn removal_leave_and_rename() {
    let mut w = World::new(&["Alice", "Bob", "Carol"]);
    let room = w.e("Alice").create_room("Trio", true).unwrap();
    w.join("Bob", "Alice", &room);
    w.join("Carol", "Alice", &room);
    let (bob, carol) = (w.id("Bob"), w.id("Carol"));

    // Only the creator can rename or remove.
    assert!(w.e("Bob").rename_room(&room, "Mine now").is_err());
    assert!(w.e("Bob").remove_member(&room, &carol).is_err());
    let out = w.e("Alice").rename_room(&room, "Renamed").unwrap();
    for (to, p) in out {
        w.send("Alice", &to, p);
    }
    assert_eq!(w.engines["Carol"].rooms()[0].name, "Renamed");

    // A forged removal from a non-creator changes nothing.
    let id = guft_core::rooms::parse_room_key(&room).unwrap();
    let alice = w.id("Alice");
    w.send(
        "Bob",
        &alice,
        Payload::RoomRemove {
            room: id,
            member: carol.clone(),
        },
    );
    w.send(
        "Bob",
        &carol,
        Payload::RoomRemove {
            room: id,
            member: carol.clone(),
        },
    );
    assert_eq!(
        w.engines["Carol"].rooms().len(),
        1,
        "carol must still be in the room"
    );
    assert_eq!(w.member_names("Alice"), ["Bob", "Carol"]);

    // The creator removes Carol: she drops the room, and nobody messages her again.
    let out = w.e("Alice").remove_member(&room, &carol).unwrap();
    for (to, p) in out {
        w.send("Alice", &to, p);
    }
    assert!(w.engines["Carol"].rooms().is_empty());
    assert_eq!(w.member_names("Alice"), ["Bob"]);
    assert_eq!(w.member_names("Bob"), ["Alice"]);
    w.say("Alice", &room, "carol can't read this");
    assert!(w.texts_for("Carol").is_empty());
    assert_eq!(w.texts_for("Bob").len(), 1);

    // Leaving tells the others.
    let out = w.e("Bob").leave_room(&room).unwrap();
    for (to, p) in out {
        w.send("Bob", &to, p);
    }
    assert!(w.engines["Bob"].rooms().is_empty());
    assert!(w.engines["Alice"].rooms()[0].members.is_empty());
    let _ = bob;
}

#[test]
fn closed_rooms_only_let_the_creator_invite() {
    let mut w = World::new(&["Alice", "Bob", "Carol"]);
    let room = w.e("Alice").create_room("Closed", false).unwrap();
    w.join("Bob", "Alice", &room);
    assert!(w.e("Bob").new_room_invite(&room, "x", HOUR, NOW).is_err());
    assert!(w.e("Alice").new_room_invite(&room, "x", HOUR, NOW).is_ok());

    // Adding an existing contact is creator-only too.
    let bob = w.id("Bob");
    w.join("Carol", "Alice", &room);
    let carol = w.id("Carol");
    assert!(w.e("Bob").add_contact_to_room(&room, &carol).is_err());
    let _ = bob;
}

#[test]
fn strangers_and_non_members_cannot_inject_messages_or_introductions() {
    let mut w = World::new(&["Alice", "Bob", "Carol", "Dave"]);
    let r1 = w.e("Alice").create_room("One", true).unwrap();
    let r2 = w.e("Alice").create_room("Two", true).unwrap();
    w.join("Bob", "Alice", &r1); // Bob is only in room One
    let alice = w.id("Alice");
    let two = guft_core::rooms::parse_room_key(&r2).unwrap();

    // A contact who is not in room Two cannot post to it.
    w.send(
        "Bob",
        &alice,
        Payload::RoomText {
            room: two,
            text: "sneaky".into(),
        },
    );
    assert!(w.inbox.is_empty());
    // Unknown rooms are ignored too.
    w.send(
        "Bob",
        &alice,
        Payload::RoomText {
            room: [9; 16],
            text: "ghost".into(),
        },
    );
    assert!(w.inbox.is_empty());

    // A forged introduction: Bob vouches that Carol's id is reachable with an invite that
    // actually belongs to Dave. Alice must not create any session from it.
    let carol = w.id("Carol");
    let (dave_invite, dave_code) = w.e("Dave").new_invite("x", HOUR, NOW).unwrap();
    let one = guft_core::rooms::parse_room_key(&r1).unwrap();
    let entry = guft_core::payload::RosterEntry {
        id: carol.clone(),
        name: "Carol".into(),
        onion: onion('c'),
    };
    let contacts_before = w.engines["Alice"].contacts().len();
    w.send(
        "Bob",
        &alice,
        Payload::Introduction {
            room: one,
            from: entry.clone(),
            invite: dave_invite,
            code: dave_code,
        },
    );
    assert_eq!(
        w.engines["Alice"].contacts().len(),
        contacts_before,
        "no contact from a mismatched introduction"
    );
    let room_one = w.engines["Alice"]
        .rooms()
        .into_iter()
        .find(|r| r.id == r1)
        .expect("room One");
    assert!(
        room_one.members.iter().any(|m| m.id == carol),
        "the roster entry is vouched for by Bob, but there is no session"
    );
    assert_eq!(room_one.reachable, 1, "only Bob is reachable");

    // A non-member cannot vouch at all.
    let r3 = w.e("Alice").create_room("Three", true).unwrap();
    let three = guft_core::rooms::parse_room_key(&r3).unwrap();
    let (c_invite, c_code) = w.e("Carol").new_invite("y", HOUR, NOW).unwrap();
    w.send(
        "Bob",
        &alice,
        Payload::Introduction {
            room: three,
            from: entry,
            invite: c_invite,
            code: c_code,
        },
    );
    assert!(
        w.engines["Alice"]
            .rooms()
            .iter()
            .find(|r| r.id == r3)
            .unwrap()
            .members
            .is_empty(),
        "Bob is not in room Three"
    );
}

#[test]
fn an_introduction_invite_works_only_for_the_member_it_names() {
    let mut w = World::new(&["Alice", "Bob", "Carol", "Mallory"]);
    let room = w.e("Alice").create_room("Trio", true).unwrap();
    w.join("Bob", "Alice", &room);

    // Carol joins via Alice; capture the one-time invite she makes for Bob.
    let (invite, code) = w
        .e("Alice")
        .new_room_invite(&room, "carol", HOUR, NOW)
        .unwrap();
    let cid = w.e("Carol").add_contact(&invite, &code, NOW).unwrap();
    let alice = w.id("Alice");
    let hello = w.e("Carol").hello_for(&cid).unwrap();
    let frame = w.e("Carol").encrypt(&alice, &hello).unwrap();
    let rec = w.e("Alice").decrypt(&frame, NOW).unwrap();
    let ri = w.e("Alice").room_invite_for(&room, &rec.from).unwrap();
    let fx = w
        .e("Carol")
        .on_room_payload(&alice, &ri, NOW)
        .unwrap_err_or_ok(&alice);
    let Payload::Introduce {
        invite: intro_invite,
        code: intro_code,
        to,
        ..
    } = &fx[0].1
    else {
        panic!("expected an introduction request")
    };
    assert_eq!(*to, w.id("Bob"));

    // Mallory intercepts that invite and tries to become "Bob" to Carol.
    let mid = w
        .e("Mallory")
        .add_contact(intro_invite, intro_code, NOW)
        .unwrap();
    let hello = w.e("Mallory").hello_for(&mid).unwrap();
    let frame = w.e("Mallory").encrypt(&mid, &hello).unwrap();
    assert!(
        w.e("Carol").decrypt(&frame, NOW).is_err(),
        "only Bob's identity may use it"
    );
    assert!(w.engines["Carol"]
        .contacts()
        .iter()
        .all(|c| c.name != "Mallory"));
}

/// Small helper so the test above reads in one line.
trait SendOf {
    fn unwrap_err_or_ok(self, from: &str) -> Vec<(String, Payload)>;
}
impl SendOf for Result<guft_core::rooms::RoomEffects, guft_core::Error> {
    fn unwrap_err_or_ok(self, _from: &str) -> Vec<(String, Payload)> {
        self.expect("room effects").send
    }
}

#[test]
fn temp_rooms_never_reach_the_saved_state() {
    let mut w = World::new(&["Alice", "Bob", "Carol"]);
    w.befriend("Bob", "Alice");
    let before = w.e("Alice").to_bytes().unwrap().to_vec();

    let bob = w.id("Bob");
    let group = w.e("Alice").create_temp_room("Heist", true).unwrap();
    let (direct, invite) = w.e("Alice").start_direct_temp(&bob).unwrap();
    assert!(invite.is_some());
    assert!(w.e("Alice").is_temp_room(&group) && w.e("Alice").is_temp_room(&direct));
    assert_eq!(w.e("Alice").rooms().len(), 2);

    // Not one byte of the sealed state changed, and reloading it shows no rooms.
    let after = w.e("Alice").to_bytes().unwrap().to_vec();
    assert_eq!(before, after);
    assert!(Engine::from_bytes(&after).unwrap().rooms().is_empty());

    // A normal room is still saved.
    let normal = w.e("Alice").create_room("Keep", true).unwrap();
    let reloaded = Engine::from_bytes(&w.e("Alice").to_bytes().unwrap()).unwrap();
    assert_eq!(reloaded.rooms().len(), 1);
    assert_eq!(reloaded.rooms()[0].id, normal);
}

#[test]
fn a_temp_group_room_works_like_a_room_but_marks_messages_temporary() {
    let mut w = World::new(&["Alice", "Bob", "Carol"]);
    let room = w.e("Alice").create_temp_room("Heist", true).unwrap();
    w.join("Bob", "Alice", &room);
    w.join("Carol", "Bob", &room);
    for who in ["Alice", "Bob", "Carol"] {
        let info = &w.engines[who].rooms()[0];
        assert_eq!(info.kind, guft_core::RoomKind::Temp, "{who} sees it as temporary");
        assert_eq!(info.members.len(), 2, "{who}");
        assert_eq!(info.reachable, 2, "{who} must reach everyone");
    }
    w.say("Carol", &room, "psst");
    assert_eq!(w.texts_for("Alice"), [("Carol".to_string(), "psst".to_string())]);
    assert!(w.inbox.iter().all(|(_, d)| d.temp));

    // Removal and leaving work the same way.
    let bob = w.id("Bob");
    let out = w.e("Alice").remove_member(&room, &bob).unwrap();
    assert_eq!(out.len(), 2);
    let leave = w.e("Carol").leave_room(&room).unwrap();
    assert!(!leave.is_empty());
    assert!(w.engines["Carol"].rooms().is_empty());

    // A normal room's messages are not marked temporary.
    let mut w = World::new(&["Alice", "Bob"]);
    let normal = w.e("Alice").create_room("Keep", true).unwrap();
    w.join("Bob", "Alice", &normal);
    w.say("Bob", &normal, "hi");
    assert!(w.inbox.iter().all(|(_, d)| !d.temp));
}

#[test]
fn a_direct_temp_chat_is_between_two_people_only() {
    let mut w = World::new(&["Alice", "Bob", "Carol"]);
    w.befriend("Bob", "Alice");
    w.befriend("Carol", "Alice");
    let (bob, carol) = (w.id("Bob"), w.id("Carol"));

    let (key, invite) = w.e("Alice").start_direct_temp(&bob).unwrap();
    w.send("Alice", &bob, invite.unwrap());
    // Asking again reuses the same chat and sends nothing new.
    let (again, none) = w.e("Alice").start_direct_temp(&bob).unwrap();
    assert_eq!(again, key);
    assert!(none.is_none());

    let on_bob = w.engines["Bob"].rooms();
    assert_eq!(on_bob.len(), 1);
    assert_eq!(on_bob[0].kind, guft_core::RoomKind::Direct);
    assert_eq!(on_bob[0].members.len(), 1);

    w.say("Alice", &key, "just us");
    w.say("Bob", &key, "agreed");
    assert_eq!(w.texts_for("Bob"), [("Alice".to_string(), "just us".to_string())]);
    assert_eq!(w.texts_for("Alice"), [("Bob".to_string(), "agreed".to_string())]);

    // Nobody can be added, invited, renamed or removed.
    assert!(w.e("Alice").add_contact_to_room(&key, &carol).is_err());
    assert!(w.e("Alice").new_room_invite(&key, "x", HOUR, NOW).is_err());
    assert!(w.e("Alice").rename_room(&key, "new").is_err());
    assert!(w.e("Alice").remove_member(&key, &bob).is_err());
    assert!(w.e("Bob").add_contact_to_room(&key, &carol).is_err());

    // Bob ending it ends it for Alice too.
    let leave = w.e("Bob").leave_room(&key).unwrap();
    for (to, p) in leave {
        w.send("Bob", &to, p);
    }
    assert!(w.engines["Alice"].rooms().is_empty());
    assert!(w.events.iter().any(|(who, e)| who == "Alice" && matches!(e, RoomEvent::Removed { .. })));
}

#[test]
fn a_forged_or_oversized_direct_invite_is_refused() {
    let mut w = World::new(&["Alice", "Bob", "Carol"]);
    w.befriend("Bob", "Alice");
    w.befriend("Carol", "Alice");
    w.befriend("Carol", "Bob");
    let (bob, carol) = (w.id("Bob"), w.id("Carol"));

    // Carol claims Alice created a one-to-one chat with Bob.
    let alice = w.id("Alice");
    let forged = Payload::RoomInvite {
        room: [7; 16],
        name: "x".into(),
        creator: alice.clone(),
        open_invites: false,
        kind: guft_core::RoomKind::Direct,
        members: vec![guft_core::payload::RosterEntry { id: alice.clone(), name: "Alice".into(), onion: onion('a') }],
    };
    let frame = w.e("Carol").encrypt(&bob, &forged).unwrap();
    let rec = w.e("Bob").decrypt(&frame, NOW).unwrap();
    assert!(w.e("Bob").on_room_payload(&rec.from, &rec.payload, NOW).is_err());
    assert!(w.engines["Bob"].rooms().is_empty());

    // Three people can never be a "one-to-one" chat: it does not even encode.
    let big = Payload::RoomInvite {
        room: [8; 16],
        name: "x".into(),
        creator: carol,
        open_invites: false,
        kind: guft_core::RoomKind::Direct,
        members: vec![
            guft_core::payload::RosterEntry { id: alice, name: "A".into(), onion: onion('a') },
            guft_core::payload::RosterEntry { id: bob, name: "B".into(), onion: onion('b') },
        ],
    };
    assert!(big.encode().is_err());
}
