//! Large frames over the real Tor network: a voice-note sized file (~180 kB) and the
//! largest file guft allows (1 MB + protocol overhead). Slow and needs internet:
//!
//!     cargo test -p guft-net --test tor_large -- --ignored --nocapture

use std::time::{Duration, Instant};

use guft_core::limits::MAX_WIRE_BYTES;
use guft_net::access::{Access, Key};
use guft_net::tor::{onion_address, TorDirs, TorNode, Vanguards};
use guft_net::{NetConfig, Transport};
use zeroize::Zeroizing;

fn key(b: u8) -> Key {
    Zeroizing::new([b; 32])
}

fn dirs(root: &std::path::Path, name: &str) -> TorDirs {
    TorDirs { state: root.join(name).join("state"), cache: root.join(name).join("cache"), vanguards: Vanguards::Full }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "uses the live Tor network"]
async fn voice_note_and_largest_file_cross_tor_intact() {
    let root = tempfile::tempdir().unwrap();
    let (seed_a, seed_b) = ([41u8; 32], [42u8; 32]);
    let (a_onion, b_onion) = (onion_address(&seed_a), onion_address(&seed_b));
    let (kab, kba) = (key(5), key(6));
    let access_a = Access { authorized: vec![("c-b".into(), kab.clone())], connect: vec![(b_onion.clone(), kba.clone())] };
    let access_b = Access { authorized: vec![("c-a".into(), kba)], connect: vec![(a_onion, kab)] };
    let (da, db) = (dirs(root.path(), "a"), dirs(root.path(), "b"));
    let (a, b) = tokio::join!(
        TorNode::start(&da, &seed_a, NetConfig::default(), access_a),
        TorNode::start(&db, &seed_b, NetConfig::default(), access_b),
    );
    let ((a, _a_rx), (b, mut b_rx)) = (a.expect("a"), b.expect("b"));
    let _ = b.wait_reachable(Duration::from_secs(90)).await;

    let (tx, mut got) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(i) = b_rx.recv().await {
            let _ = tx.send(i.frame.clone());
            let _ = i.ack.send(true);
        }
    });

    // Pseudo-random bytes so nothing compresses or repeats.
    let make = |n: usize, seed: u64| -> Vec<u8> {
        let mut x = seed;
        (0..n).map(|_| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; (x >> 24) as u8 }).collect()
    };
    for (label, size) in [("voice-note-sized", 190_000usize), ("largest-file", MAX_WIRE_BYTES - 64)] {
        let frame = make(size, size as u64);
        let t = Instant::now();
        let mut result = Err(String::new());
        for attempt in 1..=6 {
            match a.net().send(b.onion(), frame.clone()).await {
                Ok(()) => { result = Ok(t.elapsed()); break; }
                Err(e) => {
                    println!("  {label}: attempt {attempt} failed: {e}");
                    result = Err(e.to_string());
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        }
        let took = result.unwrap_or_else(|e| panic!("{label} was not delivered: {e}"));
        let received = tokio::time::timeout(Duration::from_secs(60), got.recv()).await.expect("b got the frame").unwrap();
        assert_eq!(received, frame, "{label}: bytes must arrive identical");
        println!("{label}: {} bytes acked in {took:?} ({:.1} kB/s)", size, size as f64 / 1000.0 / took.as_secs_f64());
    }
    a.shutdown().await;
    b.shutdown().await;
}
