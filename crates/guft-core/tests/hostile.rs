//! Hostile input: everything an attacker (or a corrupted disk) can hand us must be
//! refused cleanly, must never panic, and must never change our saved state.

use std::time::{Duration, Instant};

use guft_core::limits::{MAX_FILE_BYTES, MAX_WIRE_BYTES};
use guft_core::payload::{check_display_name, check_file_name, RosterEntry};
use guft_core::vault::Vault;
use guft_core::{Engine, Payload, RoomKind};
use rand::rngs::StdRng;
use rand::{Rng, RngCore, SeedableRng};

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

fn text(t: &str) -> Payload {
    Payload::Text(t.into())
}

/// Alice and Bob, connected both ways. Returns (alice, bob, bob_id_in_alice, alice_id_in_bob).
fn connected() -> (Engine, Engine, String, String) {
    let mut bob = engine("Bob", 'b');
    let (inv, code) = bob.new_invite("alice", HOUR, NOW).unwrap();
    let mut alice = engine("Alice", 'a');
    let bid = alice.add_contact(&inv, &code, NOW).unwrap();
    let hello = Payload::Hello { onion: onion('a'), name: "Alice".into(), auth: [7; 32] };
    let first = alice.encrypt(&bid, &hello).unwrap();
    let aid = bob.decrypt(&first, NOW).unwrap().from;
    // Settle the session both ways so later frames are plain Whisper messages.
    let w = bob.encrypt(&aid, &text("welcome")).unwrap();
    alice.decrypt(&w, NOW).unwrap();
    (alice, bob, bid, aid)
}

fn state(e: &Engine) -> Vec<u8> {
    e.to_bytes().unwrap().to_vec()
}

#[test]
fn every_single_bit_flip_in_a_frame_is_refused_and_changes_nothing() {
    let (mut alice, mut bob, bid, _) = connected();
    let frame = alice.encrypt(&bid, &text("attack at dawn")).unwrap();
    let before = state(&bob);
    for byte in 0..frame.len() {
        for bit in [0u8, 3, 7] {
            let mut bad = frame.clone();
            bad[byte] ^= 1 << bit;
            assert!(bob.decrypt(&bad, NOW).is_err(), "byte {byte} bit {bit} was accepted");
        }
    }
    assert_eq!(before, state(&bob), "refused frames must not touch the saved state");
    // And the genuine frame, plus the next one, still go through afterwards.
    assert_eq!(bob.decrypt(&frame, NOW).unwrap().payload, text("attack at dawn"));
    let next = alice.encrypt(&bid, &text("and again")).unwrap();
    assert_eq!(bob.decrypt(&next, NOW).unwrap().payload, text("and again"));
}

#[test]
fn tampering_with_a_large_file_frame_is_caught_everywhere() {
    let (mut alice, mut bob, bid, _) = connected();
    let file = Payload::File { name: "big.bin".into(), data: vec![0xAB; MAX_FILE_BYTES] };
    let frame = alice.encrypt(&bid, &file).unwrap();
    assert!(frame.len() <= MAX_WIRE_BYTES, "the largest legal file must fit the wire limit ({} bytes)", frame.len());
    let mut rng = StdRng::seed_from_u64(1);
    for _ in 0..200 {
        let mut bad = frame.clone();
        let i = rng.random_range(0..bad.len());
        bad[i] ^= 1 << rng.random_range(0..8);
        assert!(bob.decrypt(&bad, NOW).is_err(), "flip at {i} accepted");
    }
    // Truncation and extension.
    assert!(bob.decrypt(&frame[..frame.len() - 1], NOW).is_err());
    let mut longer = frame.clone();
    longer.push(0);
    assert!(bob.decrypt(&longer, NOW).is_err());
    assert_eq!(bob.decrypt(&frame, NOW).unwrap().payload, file);
}

#[test]
fn replays_are_refused_even_long_after() {
    let (mut alice, mut bob, bid, _) = connected();
    let first = alice.encrypt(&bid, &text("one")).unwrap();
    bob.decrypt(&first, NOW).unwrap();
    for i in 0..50 {
        let w = alice.encrypt(&bid, &text(&format!("m{i}"))).unwrap();
        bob.decrypt(&w, NOW).unwrap();
    }
    assert!(bob.decrypt(&first, NOW).is_err(), "an old frame must not be accepted a second time");
}

#[test]
fn a_frame_for_one_contact_cannot_be_passed_off_as_another() {
    let (mut alice, mut bob, bid, aid) = connected();
    // Carol is also Bob's contact.
    let mut carol = engine("Carol", 'c');
    let (inv, code) = bob.new_invite("carol", HOUR, NOW).unwrap();
    let cid_in_carol = carol.add_contact(&inv, &code, NOW).unwrap();
    let hello = Payload::Hello { onion: onion('c'), name: "Carol".into(), auth: [9; 32] };
    let first = carol.encrypt(&cid_in_carol, &hello).unwrap();
    let carol_in_bob = bob.decrypt(&first, NOW).unwrap().from;

    // Re-label Alice's frame with Carol's id in the header.
    let w = alice.encrypt(&bid, &text("from alice")).unwrap();
    let mut forged = w.clone();
    forged[1..17].copy_from_slice(&hex_to_16(&carol_in_bob));
    assert!(bob.decrypt(&forged, NOW).is_err());
    // And the reverse: the genuine frame still works, attributed to Alice.
    assert_eq!(bob.decrypt(&w, NOW).unwrap().from, aid);
}

fn hex_to_16(hex: &str) -> [u8; 16] {
    let v = data_encoding::HEXLOWER.decode(hex.as_bytes()).unwrap();
    v.try_into().unwrap()
}

#[test]
fn random_frames_with_plausible_headers_never_panic_or_change_state() {
    let (_alice, mut bob, _bid, aid) = connected();
    let (inv, _code) = bob.new_invite("open", HOUR, NOW).unwrap();
    let invite_id = guft_core::invite::InviteV1::decode(&inv).unwrap().invite_id;
    let before = state(&bob);
    let mut rng = StdRng::seed_from_u64(2);
    let started = Instant::now();
    let mut n = 0;
    while started.elapsed() < Duration::from_secs(8) {
        let len = match rng.random_range(0..4) {
            0 => rng.random_range(0..64),
            1 => rng.random_range(64..600),
            2 => rng.random_range(600..4000),
            _ => rng.random_range(4000..60_000),
        };
        let mut w = vec![0u8; len];
        rng.fill_bytes(&mut w);
        if len >= 17 {
            match rng.random_range(0..3) {
                0 => w[0] = 1,
                1 => w[0] = 2,
                _ => w[0] = rng.random(),
            }
            match w[0] {
                1 => w[1..17].copy_from_slice(&hex_to_16(&aid)),
                2 => w[1..17].copy_from_slice(&invite_id),
                _ => {}
            }
        }
        assert!(bob.decrypt(&w, NOW).is_err());
        n += 1;
    }
    assert!(n > 100, "ran {n} frames");
    // Only the pending invite bookkeeping (nothing) may differ: the state is byte-identical.
    assert_eq!(before, state(&bob));
}

#[test]
fn payload_decoding_survives_mutated_and_random_input() {
    let roster = RosterEntry { id: "a".repeat(32), name: "Eve".into(), onion: onion('e') };
    let samples = vec![
        Payload::Hello { onion: onion('a'), name: "Alice".into(), auth: [1; 32] },
        Payload::AuthKey { key: [2; 32] },
        text("hello"),
        Payload::File { name: "f.bin".into(), data: vec![5; 3000] },
        Payload::RoomInvite { room: [3; 16], name: "R".into(), creator: "b".repeat(32), open_invites: true, kind: RoomKind::Temp, members: vec![roster.clone()] },
        Payload::RoomText { room: [3; 16], text: "x".into() },
        Payload::RoomFile { room: [3; 16], name: "f".into(), data: vec![1; 700] },
        Payload::Introduce { room: [3; 16], to: "c".repeat(32), invite: "guft1:abc".into(), code: "abcd-efgh-ijkl".into() },
        Payload::Introduction { room: [3; 16], from: roster, invite: "guft1:abc".into(), code: "abcd-efgh-ijkl".into() },
        Payload::RoomLeave { room: [3; 16] },
        Payload::RoomRemove { room: [3; 16], member: "d".repeat(32) },
        Payload::RoomRename { room: [3; 16], name: "N".into() },
    ];
    let mut rng = StdRng::seed_from_u64(3);
    let mut accepted = 0;
    for s in &samples {
        let enc = s.encode().unwrap();
        assert_eq!(&Payload::decode(enc.clone()).unwrap(), s);
        for _ in 0..3000 {
            let mut m = enc.clone();
            for _ in 0..rng.random_range(1..6) {
                let i = rng.random_range(0..m.len());
                m[i] = rng.random();
            }
            // Never panics; if it decodes, it passed validation.
            if let Ok(p) = Payload::decode(m) {
                p.validate().unwrap();
                accepted += 1;
            }
        }
    }
    let _ = accepted;
    // Pure noise at every bucket size.
    for size in [512usize, 2048, 8192, 32768, 131_072, 524_288, 1_048_576] {
        for _ in 0..20 {
            let mut v = vec![0u8; size];
            rng.fill_bytes(&mut v);
            assert!(Payload::decode(v).is_err());
        }
    }
}

#[test]
fn absurd_length_prefixes_do_not_allocate_or_hang() {
    // Variant `File` (index 11), then a name length and a data length of 2^62 bytes.
    let mut raw = vec![11u8];
    raw.push(1);
    raw.push(b'x');
    raw.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x3F]);
    let mut padded = raw.clone();
    padded.push(0x80);
    padded.resize(1_048_576, 0);
    let t = Instant::now();
    assert!(Payload::decode(padded).is_err());
    assert!(t.elapsed() < Duration::from_secs(2));
    // The same for a huge member list in a room invite.
    let mut raw = vec![2u8];
    raw.extend_from_slice(&[3; 16]);
    raw.push(1);
    raw.push(b'R');
    raw.push(32);
    raw.extend_from_slice(&[b'a'; 32]);
    raw.push(1);
    raw.push(1);
    raw.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    raw.push(0x80);
    raw.resize(512, 0);
    assert!(Payload::decode(raw).is_err());
}

#[test]
fn file_names_cannot_spoof_extensions_or_escape() {
    for bad in [
        "../../etc/passwd",
        "a/b",
        "a\\b",
        ".hidden",
        "trail.",
        "trail ",
        "nul\0byte",
        "tab\tname",
        "new\nline",
        "colon:name",
        // Right-to-left override makes "invoice\u{202E}fdp.exe" display as "invoiceexe.pdf".
        "invoice\u{202E}fdp.exe",
        "isolate\u{2066}x\u{2069}.txt",
        "zero\u{200B}width.txt",
        "bom\u{FEFF}.txt",
        "lrm\u{200E}.txt",
    ] {
        assert!(check_file_name(bad).is_err(), "{bad:?} must be refused");
    }
    for ok in ["photo 1.png", "résumé.pdf", "voice-1730000000-12s.webm", "日本語.txt"] {
        assert!(check_file_name(ok).is_ok(), "{ok:?} must be allowed");
    }
}

#[test]
fn display_names_cannot_hide_text_with_invisible_or_reordering_characters() {
    for bad in ["Ali\u{202E}ce", "Bob\u{200B}", "\u{2066}Eve\u{2069}", "Mal\u{0007}lory", "\u{FEFF}Dan"] {
        assert!(check_display_name(bad).is_err(), "{bad:?} must be refused");
    }
    for ok in ["Amina Khan", "Zoë", "李雷", "Priya R."] {
        assert!(check_display_name(ok).is_ok(), "{ok:?} must be allowed");
    }
}

#[test]
fn bad_invites_never_panic_and_never_create_contacts() {
    let mut alice = engine("Alice", 'a');
    let mut bob = engine("Bob", 'b');
    let (inv, code) = bob.new_invite("x", HOUR, NOW).unwrap();
    let before = state(&alice);
    let mut rng = StdRng::seed_from_u64(4);
    for _ in 0..400 {
        let mut s = inv.clone().into_bytes();
        for _ in 0..rng.random_range(1..4) {
            let i = rng.random_range(0..s.len());
            s[i] = rng.random_range(b' '..=b'~');
        }
        let s = String::from_utf8_lossy(&s).into_owned();
        if s != inv {
            assert!(alice.add_contact(&s, &code, NOW).is_err() || alice.contacts().len() <= 1);
        }
        alice = Engine::from_bytes(&before).unwrap();
    }
    for junk in ["", "guft1:", "guft1:!!!!", "not an invite", &"x".repeat(100_000), "guft1:AAAA"] {
        assert!(alice.add_contact(junk, &code, NOW).is_err());
    }
    assert!(alice.contacts().is_empty());
}

#[test]
fn a_corrupted_vault_or_state_file_is_refused() {
    let v = Vault::create("a long enough passphrase").unwrap();
    let blob = v.seal(b"secret").unwrap();
    assert!(Vault::unlock("a long enough passphrase", &blob).is_ok());
    let mut rng = StdRng::seed_from_u64(5);
    // Flip every header byte and a sample of the rest.
    for i in (0..60).chain((0..40).map(|_| rng.random_range(60..blob.len()))) {
        let mut bad = blob.clone();
        bad[i] ^= 0x55;
        assert!(Vault::unlock("a long enough passphrase", &bad).is_err(), "byte {i}");
    }
    // A header demanding gigabytes of memory is refused before any work is done.
    let mut greedy = blob.clone();
    greedy[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
    let t = Instant::now();
    assert!(Vault::unlock("a long enough passphrase", &greedy).is_err());
    assert!(t.elapsed() < Duration::from_secs(2));
    for cut in [0, 1, 4, 20, 43, blob.len() - 1] {
        assert!(Vault::unlock("a long enough passphrase", &blob[..cut]).is_err());
    }
    // Garbage that is not a valid engine state is refused too.
    let mut junk = vec![0u8; 5000];
    rng.fill_bytes(&mut junk);
    assert!(Engine::from_bytes(&junk).is_err());
    assert!(Engine::from_bytes(&[]).is_err());
}

#[test]
fn spooled_blobs_from_strangers_are_refused() {
    let e = engine("Alice", 'a');
    let mut rng = StdRng::seed_from_u64(6);
    for len in [0usize, 1, 31, 32, 100, 1500, 70_000] {
        let mut v = vec![0u8; len];
        rng.fill_bytes(&mut v);
        assert!(e.open_spooled(&v).is_err());
    }
    // A genuinely sealed blob opens, and any change to it does not.
    let sealed = e.spool_public().seal(b"frame bytes").unwrap();
    assert_eq!(&*e.open_spooled(&sealed).unwrap(), b"frame bytes");
    for i in (0..sealed.len()).step_by(37) {
        let mut bad = sealed.clone();
        bad[i] ^= 1;
        assert!(e.open_spooled(&bad).is_err(), "byte {i}");
    }
}

#[test]
fn nothing_readable_appears_in_a_frame() {
    let (mut alice, _bob, bid, _) = connected();
    let marker = "TOP-SECRET-MARKER-0123456789-ABCDEFGHIJ";
    let frame = alice.encrypt(&bid, &text(marker)).unwrap();
    assert!(!frame.windows(marker.len()).any(|w| w == marker.as_bytes()));
    let file = Payload::File { name: "SECRET-NAME-MARKER.txt".into(), data: marker.as_bytes().repeat(40) };
    let frame = alice.encrypt(&bid, &file).unwrap();
    for needle in [marker.as_bytes(), b"SECRET-NAME-MARKER".as_slice()] {
        assert!(!frame.windows(needle.len()).any(|w| w == needle));
    }
    // Ciphertext looks random: a crude balance check over a big, highly repetitive file.
    let big = Payload::File { name: "z.bin".into(), data: vec![0u8; 500_000] };
    let frame = alice.encrypt(&bid, &big).unwrap();
    let ones: u64 = frame.iter().map(|b| u64::from(b.count_ones())).sum();
    let frac = ones as f64 / (frame.len() as f64 * 8.0);
    assert!((0.495..0.505).contains(&frac), "bit balance {frac}");
}

#[test]
fn the_same_message_never_produces_the_same_ciphertext() {
    let (mut alice, _bob, bid, _) = connected();
    let a = alice.encrypt(&bid, &text("same")).unwrap();
    let b = alice.encrypt(&bid, &text("same")).unwrap();
    assert_ne!(a, b);
    assert_ne!(a[17..], b[17..]);
}
