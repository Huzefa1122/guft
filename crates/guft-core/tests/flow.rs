use guft_core::limits::MAX_FILE_BYTES;
use guft_core::vault::Vault;
use guft_core::{Engine, Payload};

const NOW: u64 = 1_800_000_000;
const HOUR: u64 = 3600;

fn onion(c: char) -> String {
    format!("{}.onion", c.to_string().repeat(56))
}

fn engine(name: &str, c: char) -> Engine {
    let mut e = Engine::create(name).unwrap();
    e.set_onion(&onion(c)).unwrap();
    e
}

fn hello(name: &str, c: char) -> Payload {
    Payload::Hello { onion: onion(c), name: name.into(), auth: [7; 32] }
}

fn text(t: &str) -> Payload {
    Payload::Text(t.into())
}

/// Alice imports Bob's invite and Bob accepts her hello. Returns (alice, bob, bob_id_in_alice, alice_id_in_bob).
fn connected() -> (Engine, Engine, String, String) {
    let mut bob = engine("Bob", 'b');
    let (inv, code) = bob.new_invite("alice", HOUR, NOW).unwrap();
    let mut alice = engine("Alice", 'a');
    let bid = alice.add_contact(&inv, &code, NOW).unwrap();
    let first = alice.encrypt(&bid, &hello("Alice", 'a')).unwrap();
    let r = bob.decrypt(&first, NOW).unwrap();
    assert!(r.new_contact);
    (alice, bob, bid, r.from)
}

#[test]
fn full_conversation() {
    let (mut alice, mut bob, bid, aid) = connected();
    assert_eq!(bob.contacts()[0].name, "Alice");
    assert_eq!(bob.contacts()[0].onion, onion('a'));

    let w = bob.encrypt(&aid, &text("hi alice")).unwrap();
    assert_eq!(alice.decrypt(&w, NOW).unwrap().payload, text("hi alice"));
    for i in 0..30 {
        let w = alice.encrypt(&bid, &text(&format!("a{i}"))).unwrap();
        assert_eq!(bob.decrypt(&w, NOW).unwrap().payload, text(&format!("a{i}")));
        let w = bob.encrypt(&aid, &text(&format!("b{i}"))).unwrap();
        assert_eq!(alice.decrypt(&w, NOW).unwrap().payload, text(&format!("b{i}")));
    }
    let file = Payload::File { name: "pic.png".into(), data: vec![42; MAX_FILE_BYTES] };
    let w = alice.encrypt(&bid, &file).unwrap();
    assert_eq!(bob.decrypt(&w, NOW).unwrap().payload, file);
    assert_eq!(alice.safety_number(&bid).unwrap(), bob.safety_number(&aid).unwrap());
}

#[test]
fn oversize_file_rejected_on_send() {
    let (mut alice, _bob, bid, _) = connected();
    let big = Payload::File { name: "x".into(), data: vec![0; MAX_FILE_BYTES + 1] };
    assert!(alice.encrypt(&bid, &big).is_err());
}

#[test]
fn wrong_code_fails_and_does_not_burn_invite() {
    let mut bob = engine("Bob", 'b');
    let (inv, code) = bob.new_invite("x", HOUR, NOW).unwrap();

    let mut mallory = engine("Mallory", 'm');
    let mid = mallory.add_contact(&inv, "aaaa-bbbb-cccc", NOW).unwrap();
    let w = mallory.encrypt(&mid, &hello("Mallory", 'm')).unwrap();
    assert!(bob.decrypt(&w, NOW).is_err());
    assert!(bob.contacts().is_empty());

    let mut alice = engine("Alice", 'a');
    let bid = alice.add_contact(&inv, &code, NOW).unwrap();
    let w = alice.encrypt(&bid, &hello("Alice", 'a')).unwrap();
    assert!(bob.decrypt(&w, NOW).unwrap().new_contact);
}

#[test]
fn invite_is_single_use() {
    let mut bob = engine("Bob", 'b');
    let (inv, code) = bob.new_invite("x", HOUR, NOW).unwrap();
    let mut alice = engine("Alice", 'a');
    let mut carol = engine("Carol", 'c');
    let bid = alice.add_contact(&inv, &code, NOW).unwrap();
    let cid = carol.add_contact(&inv, &code, NOW).unwrap();
    let wa = alice.encrypt(&bid, &hello("Alice", 'a')).unwrap();
    let wc = carol.encrypt(&cid, &hello("Carol", 'c')).unwrap();
    assert!(bob.decrypt(&wa, NOW).unwrap().new_contact);
    assert!(bob.decrypt(&wc, NOW).is_err());
    assert_eq!(bob.contacts().len(), 1);
}

#[test]
fn invite_expires() {
    let mut bob = engine("Bob", 'b');
    let (inv, code) = bob.new_invite("x", HOUR, NOW).unwrap();
    let mut alice = engine("Alice", 'a');
    assert!(alice.add_contact(&inv, &code, NOW + HOUR).is_err());
    let bid = alice.add_contact(&inv, &code, NOW + HOUR - 1).unwrap();
    let w = alice.encrypt(&bid, &hello("Alice", 'a')).unwrap();
    assert!(bob.decrypt(&w, NOW + HOUR).is_err());
    assert!(bob.contacts().is_empty());
    bob.purge_expired(NOW + HOUR);
    assert!(bob.pending_invites().is_empty());
}

#[test]
fn invite_lifetime_bounds_and_own_invite() {
    let mut bob = engine("Bob", 'b');
    assert!(bob.new_invite("x", 60, NOW).is_err());
    assert!(bob.new_invite("x", 8 * 24 * HOUR, NOW).is_err());
    let (inv, code) = bob.new_invite("x", HOUR, NOW).unwrap();
    assert!(bob.add_contact(&inv, &code, NOW).is_err());
}

#[test]
fn replay_and_duplicate_first_message_rejected() {
    let mut bob = engine("Bob", 'b');
    let (inv, code) = bob.new_invite("x", HOUR, NOW).unwrap();
    let mut alice = engine("Alice", 'a');
    let bid = alice.add_contact(&inv, &code, NOW).unwrap();
    let first = alice.encrypt(&bid, &hello("Alice", 'a')).unwrap();
    // Sent before Alice hears back: still delivered via the consumed invite.
    let second = alice.encrypt(&bid, &text("more")).unwrap();
    let r = bob.decrypt(&first, NOW).unwrap();
    assert!(bob.decrypt(&first, NOW).is_err(), "duplicate first frame");
    assert_eq!(bob.decrypt(&second, NOW).unwrap().payload, text("more"));
    assert!(bob.decrypt(&second, NOW).is_err(), "replay");
    let w = bob.encrypt(&r.from, &text("ok")).unwrap();
    assert_eq!(alice.decrypt(&w, NOW).unwrap().payload, text("ok"));
    assert!(alice.decrypt(&w, NOW).is_err(), "replay to alice");
}

#[test]
fn out_of_order_delivery() {
    let (mut alice, mut bob, bid, _) = connected();
    let ws: Vec<_> = (0..5).map(|i| alice.encrypt(&bid, &text(&format!("m{i}"))).unwrap()).collect();
    for i in [3, 1, 0, 4, 2] {
        assert_eq!(bob.decrypt(&ws[i], NOW).unwrap().payload, text(&format!("m{i}")));
    }
}

#[test]
fn persistence_roundtrip() {
    let (mut alice, bob, bid, aid) = connected();
    let vault = Vault::create("pass phrase").unwrap();
    let blob = vault.seal(&bob.to_bytes().unwrap()).unwrap();
    assert!(Vault::unlock("wrong phrase", &blob).is_err());
    let (_, plain) = Vault::unlock("pass phrase", &blob).unwrap();
    let mut bob = Engine::from_bytes(&plain).unwrap();
    let w = alice.encrypt(&bid, &text("after restore")).unwrap();
    assert_eq!(bob.decrypt(&w, NOW).unwrap().payload, text("after restore"));
    let w = bob.encrypt(&aid, &text("reply")).unwrap();
    assert_eq!(alice.decrypt(&w, NOW).unwrap().payload, text("reply"));
}

#[test]
fn garbage_never_panics() {
    let (_alice, mut bob, _, _) = connected();
    for len in [0usize, 1, 16, 17, 40, 41, 100, 1000, 300_000] {
        for fill in [0u8, 1, 2, 0xff] {
            let mut w = vec![fill; len];
            if len > 0 {
                w[0] = fill % 3;
            }
            assert!(bob.decrypt(&w, NOW).is_err());
        }
    }
}

#[test]
fn tampered_frames_rejected() {
    let (mut alice, mut bob, bid, _) = connected();
    let w = alice.encrypt(&bid, &text("hello")).unwrap();
    for i in [0, 1, 17, 20, 30, w.len() - 1] {
        let mut bad = w.clone();
        bad[i] ^= 1;
        assert!(bob.decrypt(&bad, NOW).is_err(), "byte {i}");
    }
    assert_eq!(bob.decrypt(&w, NOW).unwrap().payload, text("hello"));
}

#[test]
fn similar_sizes_look_identical_on_the_wire() {
    let (mut alice, _bob, bid, _) = connected();
    let a = alice.encrypt(&bid, &text("hi")).unwrap();
    let b = alice.encrypt(&bid, &text(&"x".repeat(300))).unwrap();
    assert_eq!(a.len(), b.len());
}

#[test]
fn client_auth_keys_follow_invite_then_contact() {
    let mut bob = engine("Bob", 'b');
    let (inv, code) = bob.new_invite("alice", HOUR, NOW).unwrap();
    let pending = bob.authorized_clients(NOW);
    assert_eq!(pending.len(), 1, "a pending invite authorizes exactly its own key");
    assert!(pending[0].0.starts_with("i-"));

    let mut alice = engine("Alice", 'a');
    let bid = alice.add_contact(&inv, &code, NOW).unwrap();
    let ck = alice.connect_keys();
    assert_eq!((ck[0].0.as_str(), *ck[0].1), (onion('b').as_str(), *pending[0].1), "alice uses the invite's key");

    let first = alice.encrypt(&bid, &alice.hello_for(&bid).unwrap()).unwrap();
    let r = bob.decrypt(&first, NOW).unwrap();
    let aid = r.from;

    // The invite's key is revoked at once; alice is now authorized under her own label.
    let now_auth = bob.authorized_clients(NOW);
    assert_eq!(now_auth.len(), 1);
    assert_eq!(now_auth[0].0, format!("c-{aid}"));
    // Bob reaches alice with the key alice derived for him, taken from her hello.
    let alice_side = alice.authorized_clients(NOW);
    let for_bob = alice_side.iter().find(|(l, _)| *l == format!("c-{bid}")).expect("alice authorizes bob");
    assert_eq!(*bob.connect_keys()[0].1, *for_bob.1);

    // Bob's permanent key reaches alice and replaces the invite key.
    let ak = bob.auth_key_for(&aid).unwrap();
    let w = bob.encrypt(&aid, &ak).unwrap();
    alice.decrypt(&w, NOW).unwrap();
    assert_eq!(*alice.connect_keys()[0].1, *now_auth[0].1, "invite key replaced");
    assert_ne!(*alice.connect_keys()[0].1, *pending[0].1);

    // Keys are per contact and never repeat.
    let (inv2, _) = bob.new_invite("another", HOUR, NOW).unwrap();
    let all = bob.authorized_clients(NOW);
    assert_eq!(all.len(), 2);
    assert_ne!(*all[0].1, *all[1].1);
    let _ = inv2;
}

#[test]
fn expired_and_revoked_invites_stop_authorizing() {
    let mut bob = engine("Bob", 'b');
    bob.new_invite("a", HOUR, NOW).unwrap();
    bob.new_invite("b", 2 * HOUR, NOW).unwrap();
    assert_eq!(bob.authorized_clients(NOW).len(), 2);
    assert_eq!(bob.authorized_clients(NOW + HOUR).len(), 1, "expired invite drops out");
    bob.revoke_invite("b");
    assert_eq!(bob.authorized_clients(NOW).len(), 1);
}

#[test]
fn only_contacts_can_be_given_keys() {
    let bob = engine("Bob", 'b');
    assert!(bob.hello_for("nobody").is_err());
    assert!(bob.auth_key_for("nobody").is_err());
}
