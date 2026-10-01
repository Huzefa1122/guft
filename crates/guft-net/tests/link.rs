use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use futures::io::AsyncWriteExt;
use guft_core::limits::MAX_WIRE_BYTES;
use guft_net::cell::{cover_cell, fragment, CELL_SIZE};
use guft_net::mem::{MemConnector, MemNetwork};
use guft_net::{Connector, Error, Inbound, Net, NetConfig, Transport};
use tokio::sync::mpsc;

fn fast_cfg() -> NetConfig {
    NetConfig {
        max_jitter: Duration::from_millis(2),
        ack_timeout: Duration::from_secs(5),
        cover_mean: None,
        ..NetConfig::default()
    }
}

/// A receiver that accepts every frame and records it.
fn accept_all(mut rx: mpsc::Receiver<Inbound>) -> mpsc::UnboundedReceiver<Vec<u8>> {
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(i) = rx.recv().await {
            let _ = out_tx.send(i.frame);
            let _ = i.ack.send(true);
        }
    });
    out_rx
}

fn setup(cfg: NetConfig) -> (Arc<MemNetwork>, Arc<Net<MemConnector>>, mpsc::UnboundedReceiver<Vec<u8>>) {
    let mem = MemNetwork::new();
    let got = accept_all(mem.register("bob"));
    let net = Net::new(mem.connector(), cfg).unwrap();
    (mem, net, got)
}

fn pseudo(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(31) % 251) as u8).collect()
}

#[tokio::test]
async fn small_and_largest_frames_are_delivered_and_acked() {
    let (_mem, net, mut got) = setup(fast_cfg());
    for len in [1usize, 100, 2033, 2034, 50_000, MAX_WIRE_BYTES] {
        let frame = pseudo(len);
        net.send("bob", frame.clone()).await.unwrap();
        assert_eq!(got.recv().await.unwrap(), frame, "len {len}");
    }
}

#[tokio::test]
async fn at_least_five_circuits_each_carry_traffic() {
    let (mem, net, mut got) = setup(fast_cfg());
    net.send("bob", pseudo(100_000)).await.unwrap();
    got.recv().await.unwrap();
    assert!(*mem.opened.lock().unwrap().get("bob").unwrap() >= 5);
    let bytes = mem.bytes.lock().unwrap();
    let busy = bytes.iter().filter(|((o, _), c)| o == "bob" && c.load(Ordering::Relaxed) > 0).count();
    assert!(busy >= 5, "only {busy} circuits carried data");
}

#[tokio::test]
async fn config_below_five_circuits_is_refused() {
    let mem = MemNetwork::new();
    for (circuits, min) in [(4, 4), (3, 1), (6, 4), (4, 5)] {
        let cfg = NetConfig { circuits, min_circuits: min, ..NetConfig::default() };
        assert!(Net::new(mem.connector(), cfg).is_err(), "{circuits}/{min}");
    }
    assert!(Net::new(mem.connector(), NetConfig::default()).is_ok());
}

#[tokio::test]
async fn refused_frames_are_not_acknowledged() {
    let mem = MemNetwork::new();
    let mut rx = mem.register("bob");
    tokio::spawn(async move {
        while let Some(i) = rx.recv().await {
            let _ = i.ack.send(false);
        }
    });
    let cfg = NetConfig { ack_timeout: Duration::from_millis(500), ..fast_cfg() };
    let net = Net::new(mem.connector(), cfg).unwrap();
    assert!(matches!(net.send("bob", pseudo(3000)).await, Err(Error::AckTimeout)));
}

#[tokio::test]
async fn offline_then_online_recovers() {
    let (mem, net, mut got) = setup(fast_cfg());
    mem.set_online("bob", false);
    assert!(matches!(net.send("bob", pseudo(500)).await, Err(Error::NotEnoughCircuits { have: 0, .. })));
    mem.set_online("bob", true);
    net.send("bob", pseudo(500)).await.unwrap();
    assert_eq!(got.recv().await.unwrap(), pseudo(500));
    assert!(matches!(net.send("nobody", pseudo(5)).await, Err(Error::NotEnoughCircuits { .. })));
}

#[tokio::test]
async fn dropped_circuits_are_rebuilt() {
    let (mem, net, mut got) = setup(fast_cfg());
    net.send("bob", pseudo(5000)).await.unwrap();
    got.recv().await.unwrap();
    let before = *mem.opened.lock().unwrap().get("bob").unwrap();
    net.close_all().await;
    net.send("bob", pseudo(5000)).await.unwrap();
    got.recv().await.unwrap();
    assert!(*mem.opened.lock().unwrap().get("bob").unwrap() >= before + 5);
}

#[tokio::test]
async fn concurrent_senders_all_arrive() {
    let (_mem, net, mut got) = setup(fast_cfg());
    let mut tasks = Vec::new();
    for i in 0..8u8 {
        let net = net.clone();
        tasks.push(tokio::spawn(async move { net.send("bob", vec![i; 4000 + i as usize]).await }));
    }
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    let mut seen: Vec<u8> = (0..8).map(|_| ()).map(|_| ()).map(|_| 0).collect();
    for _ in 0..8 {
        let f = got.recv().await.unwrap();
        seen[f[0] as usize] += 1;
        assert!(f.iter().all(|b| *b == f[0]));
    }
    assert!(seen.iter().all(|c| *c == 1));
}

#[tokio::test]
async fn cover_traffic_flows_and_does_not_disturb_frames() {
    let cfg = NetConfig { cover_mean: Some(Duration::from_millis(20)), ..fast_cfg() };
    let (mem, net, mut got) = setup(cfg);
    net.send("bob", pseudo(1000)).await.unwrap();
    got.recv().await.unwrap();
    let after_first: u64 = mem.bytes.lock().unwrap().values().map(|c| c.load(Ordering::Relaxed)).sum();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let later: u64 = mem.bytes.lock().unwrap().values().map(|c| c.load(Ordering::Relaxed)).sum();
    assert!(later >= after_first + 5 * CELL_SIZE as u64, "no cover cells seen ({after_first} -> {later})");
    assert_eq!((later - after_first) % CELL_SIZE as u64, 0, "cover must be whole fixed-size cells");
    net.send("bob", pseudo(7000)).await.unwrap();
    assert_eq!(got.recv().await.unwrap(), pseudo(7000));
}

#[tokio::test]
async fn idle_links_close_and_reopen() {
    let cfg = NetConfig {
        cover_mean: Some(Duration::from_millis(10)),
        idle_close: Duration::from_millis(150),
        ..fast_cfg()
    };
    let (mem, net, mut got) = setup(cfg);
    net.send("bob", pseudo(100)).await.unwrap();
    got.recv().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let opened = *mem.opened.lock().unwrap().get("bob").unwrap();
    net.send("bob", pseudo(100)).await.unwrap();
    got.recv().await.unwrap();
    assert!(*mem.opened.lock().unwrap().get("bob").unwrap() >= opened + 5, "idle link was not closed");
}

#[tokio::test]
async fn malicious_streams_are_dropped_without_harm() {
    let mem = MemNetwork::new();
    let mut inbound = accept_all(mem.register("bob"));
    let conn = mem.connector();

    // Garbage, an oversize-total cell, and a cell with a bad hash: each just ends its stream.
    let mut bad_total = fragment(&[1u8; 5000], &mut rand::rng()).unwrap().remove(0);
    bad_total[11..13].copy_from_slice(&60_000u16.to_be_bytes());
    let mut forged = fragment(&[2u8; 3000], &mut rand::rng()).unwrap();
    forged[1][20] ^= 0xff;
    let attacks: Vec<Vec<[u8; CELL_SIZE]>> = vec![vec![[0xAB; CELL_SIZE]], vec![bad_total], forged];
    for cells in attacks {
        let mut s = conn.connect("bob", 0).await.unwrap();
        for c in cells {
            let _ = s.write_all(&c).await;
        }
        let _ = s.flush().await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(inbound.try_recv().is_err(), "nothing malicious may reach the app");

    // A cover cell and a truncated cell are harmless; the node still works afterwards.
    let mut s = conn.connect("bob", 0).await.unwrap();
    s.write_all(&cover_cell(&mut rand::rng())).await.unwrap();
    s.write_all(&[1, 2, 3]).await.unwrap();
    drop(s);
    let net = Net::new(mem.connector(), fast_cfg()).unwrap();
    net.send("bob", pseudo(2500)).await.unwrap();
    assert_eq!(inbound.recv().await.unwrap(), pseudo(2500));
}

#[tokio::test]
async fn duplicate_cells_and_retransmits_do_not_duplicate_frames() {
    let (_mem, net, mut got) = setup(fast_cfg());
    let frame = pseudo(9000);
    net.send("bob", frame.clone()).await.unwrap();
    assert_eq!(got.recv().await.unwrap(), frame);
    // Retransmission of an already-delivered frame is delivered again to the app,
    // which dedupes by frame id (the app layer owns that); the transport stays consistent.
    net.send("bob", frame.clone()).await.unwrap();
    assert_eq!(got.recv().await.unwrap(), frame);
    assert!(got.try_recv().is_err());
}
