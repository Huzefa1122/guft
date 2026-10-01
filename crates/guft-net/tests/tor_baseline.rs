//! Does this machine's network let Tor build ordinary circuits at all?
//! Isolates "our code is wrong" from "the environment blocks Tor".
//!
//!     cargo test -p guft-net --test tor_baseline -- --ignored --nocapture

use std::time::{Duration, Instant};

use arti_client::config::TorClientConfigBuilder;
use arti_client::TorClient;
use futures::io::{AsyncReadExt, AsyncWriteExt};

async fn try_connect(client: &TorClient<tor_rtcompat::PreferredRuntime>, host: &str, port: u16, req: &str) -> Result<usize, String> {
    let t = Instant::now();
    let res = tokio::time::timeout(Duration::from_secs(90), async {
        let mut s = client.connect((host, port)).await.map_err(|e| e.to_string())?;
        s.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
        s.flush().await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 256];
        s.read(&mut buf).await.map_err(|e| e.to_string())
    })
    .await;
    println!("  {host}:{port} -> {:?} in {:?}", res.as_ref().map(|r| r.as_ref().map_err(|e| e.clone())), t.elapsed());
    res.map_err(|_| "timeout".to_string())?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "uses the live Tor network"]
async fn ordinary_circuits_work() {
    let root = tempfile::tempdir().unwrap();
    let mut b = TorClientConfigBuilder::from_directories(root.path().join("s"), root.path().join("c"));
    b.address_filter().allow_onion_addrs(true);
    let t = Instant::now();
    let client = TorClient::create_bootstrapped(b.build().unwrap()).await.expect("bootstrap");
    println!("bootstrapped in {:?}", t.elapsed());

    let exit = try_connect(&client, "check.torproject.org", 80, "GET / HTTP/1.0\r\nHost: check.torproject.org\r\n\r\n").await;
    let onion = try_connect(
        &client,
        "duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion",
        80,
        "GET / HTTP/1.0\r\nHost: duckduckgogg42xjoc72x3sjasowoarfbgcmvfimaftt6twagswzczad.onion\r\n\r\n",
    )
    .await;
    println!("exit circuit: {exit:?}\nonion client circuit: {onion:?}");
}
