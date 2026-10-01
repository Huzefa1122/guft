//! A room with its own identity over the real Tor network. Slow (minutes), needs internet:
//!
//!     cargo test -p guft-app --test tor_personas -- --ignored --nocapture

use std::time::{Duration, Instant};

use guft_app::{AppOptions, Body, Hub, TorBackend};
use guft_net::NetConfig;

const PASS: &str = "correct horse battery";

async fn until(what: &str, max: Duration, mut cond: impl FnMut() -> bool) {
    let t = Instant::now();
    while !cond() {
        assert!(t.elapsed() < max, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    println!("{what}: {:?}", t.elapsed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "uses the live Tor network"]
async fn a_separate_identity_room_works_over_tor() {
    let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let make = |dir: &std::path::Path| Hub::new(dir.join("profile"), AppOptions::default(), |d| TorBackend::new(d, NetConfig::default()));
    let (alice, bob) = (make(da.path()), make(db.path()));
    tokio::join!(alice.create_profile(PASS, "Alice"), bob.create_profile(PASS, "Bob")).0.unwrap();

    let room = alice.create_room("Live", true, false, Some("Ghost")).await.unwrap();
    let (invite, code) = alice.new_room_invite(&room, "bob", Duration::from_secs(3600)).unwrap();
    let main_onion = alice.my_onion().unwrap();
    let persona_onion = alice.persona_onion(1).unwrap();
    assert_ne!(main_onion, persona_onion);
    bob.add_contact(&invite, &code, None).await.unwrap();

    // Alice's separate identity runs its own Tor client and onion service; Bob reaches it there.
    until("bob is in the room with Ghost", Duration::from_secs(900), || bob.rooms().is_ok_and(|r| r.len() == 1 && r[0].members.len() == 1)).await;
    let bob_room = bob.rooms().unwrap()[0].id.clone();
    assert_eq!(bob.contacts().unwrap()[0].onion, persona_onion);

    alice.send_room_text(&room, "over tor, as someone else").await.unwrap();
    until("bob reads it", Duration::from_secs(900), || {
        bob.messages(&bob_room, None, 5).is_ok_and(|m| m.iter().any(|x| matches!(&x.body, Body::Text(t) if t == "over tor, as someone else")))
    })
    .await;
    bob.send_room_text(&bob_room, "and back").await.unwrap();
    until("ghost reads the reply", Duration::from_secs(900), || alice.messages(&room, None, 5).is_ok_and(|m| m.len() == 2)).await;
    let _ = tokio::join!(alice.lock(), bob.lock());
}
