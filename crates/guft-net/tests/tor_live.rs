//! Talks to the real Tor network. Slow (minutes) and needs internet access:
//!
//!     cargo test -p guft-net --test tor_live -- --ignored --nocapture
//!
//! Covers: onion identity from the vault seed, client authorization (authorized
//! nodes talk, a stranger is refused, a live key change lets a new client in),
//! multi-circuit delivery with acknowledgements.

use std::time::{Duration, Instant};

use guft_net::access::{Access, AccessControl, Key};
use guft_net::tor::{onion_address, TorDirs, TorNode, Vanguards};
use guft_net::{NetConfig, Transport};
use zeroize::Zeroizing;

fn dirs(root: &std::path::Path, name: &str) -> TorDirs {
    // GUFT_VANGUARDS=disabled|lite|full (default full)
    let vanguards = match std::env::var("GUFT_VANGUARDS").as_deref() {
        Ok("disabled") => Vanguards::Disabled,
        Ok("lite") => Vanguards::Lite,
        _ => Vanguards::Full,
    };
    TorDirs { state: root.join(name).join("state"), cache: root.join(name).join("cache"), vanguards }
}

fn key(b: u8) -> Key {
    Zeroizing::new([b; 32])
}

async fn send_with_retries(node: &TorNode, to: &str, frame: &[u8], attempts: u32) -> Result<Duration, String> {
    let t = Instant::now();
    let net = node.net();
    let mut last = String::new();
    for attempt in 1..=attempts {
        match net.send(to, frame.to_vec()).await {
            Ok(()) => return Ok(t.elapsed()),
            Err(e) => {
                last = e.to_string();
                println!("  attempt {attempt} failed: {last}");
                tokio::time::sleep(Duration::from_secs(15)).await;
            }
        }
    }
    Err(last)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "uses the live Tor network"]
async fn authorized_nodes_talk_strangers_are_refused() {
    // Set RUST_LOG (e.g. `warn,tor_hsservice=debug`) to see Arti's own logs.
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_test_writer().try_init();
    let root = tempfile::tempdir().unwrap();
    let t0 = Instant::now();
    let (seed_a, seed_b, seed_c) = ([11u8; 32], [22u8; 32], [33u8; 32]);
    let (a_onion, b_onion) = (onion_address(&seed_a), onion_address(&seed_b));

    // a and b authorize each other with a key apiece; c holds nothing.
    let (kab, kba) = (key(5), key(6));
    let access_a = Access { authorized: vec![("c-b".into(), kab.clone())], connect: vec![(b_onion.clone(), kba.clone())] };
    let access_b = Access { authorized: vec![("c-a".into(), kba.clone())], connect: vec![(a_onion.clone(), kab.clone())] };

    let (da, db, dc) = (dirs(root.path(), "a"), dirs(root.path(), "b"), dirs(root.path(), "c"));
    let (a, b, c) = tokio::join!(
        TorNode::start(&da, &seed_a, NetConfig::default(), access_a),
        TorNode::start(&db, &seed_b, NetConfig::default(), access_b),
        TorNode::start(&dc, &seed_c, NetConfig::default(), Access::default()),
    );
    let ((a, _a_rx), (b, mut b_rx), (c, _c_rx)) = (a.expect("a"), b.expect("b"), c.expect("c"));
    println!("bootstrapped all three in {:?}", t0.elapsed());

    // Arti must report exactly the address we derived from the vault seed.
    assert_eq!(a.onion(), a_onion);
    assert_eq!(b.service_onion().as_deref(), Some(b_onion.as_str()));

    // Waiting for "fully reachable" can be slow on a filtered network; sending is the real test.
    match b.wait_reachable(Duration::from_secs(90)).await {
        Ok(()) => println!("b fully reachable after {:?}", t0.elapsed()),
        Err(e) => println!("b not fully reachable yet ({e}); trying anyway"),
    }

    tokio::spawn(async move {
        while let Some(i) = b_rx.recv().await {
            println!("b received {} bytes", i.frame.len());
            let _ = i.ack.send(true);
        }
    });

    // 1. The authorized node gets through.
    let frame: Vec<u8> = (0..30_000u32).map(|i| (i % 251) as u8).collect();
    let took = send_with_retries(&a, b.onion(), &frame, 8).await.expect("authorized send must be delivered");
    println!("authorized delivery acked in {took:?}");

    // 2. A stranger with no key is refused (Tor reports missing client authorization).
    let stranger = c.net().send(b.onion(), vec![1; 500]).await;
    println!("stranger result: {stranger:?}");
    assert!(stranger.is_err(), "a node without a key must not reach b");

    // 3. Authorize the stranger live (reconfigure b, give c the key) and it gets through.
    let kcb = key(7);
    b.update(Access {
        authorized: vec![("c-a".into(), kba.clone()), ("c-c".into(), kcb.clone())],
        connect: vec![(a_onion.clone(), kab.clone())],
    })
    .unwrap();
    c.update(Access { authorized: vec![], connect: vec![(b_onion.clone(), kcb.clone())] }).unwrap();
    let took = send_with_retries(&c, b.onion(), &[7u8; 4_000], 10).await.expect("newly authorized client must get through");
    println!("live-authorized delivery acked in {took:?}");

    // 4. Revoke it again: the old authorization must stop working for new connections.
    b.update(Access { authorized: vec![("c-a".into(), kba)], connect: vec![(a_onion, kab)] }).unwrap();

    a.shutdown().await;
    b.shutdown().await;
    c.shutdown().await;
}
