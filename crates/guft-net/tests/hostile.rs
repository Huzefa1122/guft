//! Hostile input at the cell layer: random and adversarial cells must never panic,
//! and memory use must stay inside the documented bounds no matter what arrives.

use std::time::{Duration, Instant};

use guft_net::cell::{ack_cell, cover_cell, fragment, parse, Cell, Kind, CELL_SIZE, MAX_CELLS};
use guft_net::reassembly::{Reassembler, MAX_PARTIALS, MAX_PARTIAL_BYTES, PARTIAL_TTL};
use rand::rngs::StdRng;
use rand::{Rng, RngCore, SeedableRng};

#[test]
fn random_cells_never_panic_the_parser() {
    let mut rng = StdRng::seed_from_u64(10);
    let mut raw = [0u8; CELL_SIZE];
    let mut ok = 0u32;
    for _ in 0..300_000 {
        rng.fill_bytes(&mut raw);
        // Bias toward valid kinds so the deeper checks run.
        raw[0] = rng.random_range(0..5);
        if parse(&raw).is_ok() {
            ok += 1;
        }
    }
    // Some random cells are valid (kind 3 with zero fields is rare), none may panic.
    let _ = ok;
}

#[test]
fn adversarial_headers_are_rejected() {
    let mut rng = StdRng::seed_from_u64(11);
    let good = fragment(&[1u8; 5000], &mut rng).unwrap();
    // Every field set to its extremes.
    for (off, len) in [(9usize, 2usize), (11, 2), (13, 2)] {
        for v in [0u16, 1, 0x7fff, 0xffff] {
            let mut c = good[0];
            c[off..off + len].copy_from_slice(&v.to_be_bytes());
            let _ = parse(&c); // must not panic; most are errors
        }
    }
    let mut c = good[0];
    c[11..13].copy_from_slice(&((MAX_CELLS + 1) as u16).to_be_bytes());
    assert!(parse(&c).is_err(), "more cells than the largest frame allows");
    let mut c = good[0];
    c[13..15].copy_from_slice(&u16::MAX.to_be_bytes());
    assert!(parse(&c).is_err(), "payload length beyond the cell");
}

fn data_cell(id: [u8; 8], seq: u16, total: u16, len: usize) -> Cell {
    Cell { kind: Kind::Data, frame_id: id, seq, total, payload: vec![0xA5; len] }
}

#[test]
fn reassembly_memory_stays_bounded_under_a_flood_of_partial_frames() {
    let mut rng = StdRng::seed_from_u64(12);
    let mut r = Reassembler::new();
    let t0 = Instant::now();
    for i in 0..200_000u32 {
        let mut id = [0u8; 8];
        rng.fill_bytes(&mut id);
        let total = rng.random_range(2..=MAX_CELLS as u16);
        let seq = rng.random_range(0..total - 1);
        // Errors are fine (limits hit); panics and unbounded growth are not.
        let _ = r.push(data_cell(id, seq, total, 2033), t0 + Duration::from_millis(u64::from(i % 1000)));
        assert!(r.in_flight() <= MAX_PARTIALS);
        assert!(r.held_bytes() <= MAX_PARTIAL_BYTES);
    }
    // Old partials expire, freeing everything.
    r.expire(t0 + PARTIAL_TTL + Duration::from_secs(2000));
    assert_eq!(r.in_flight(), 0);
    assert_eq!(r.held_bytes(), 0);
}

#[test]
fn inconsistent_and_duplicate_cells_for_one_frame_are_handled() {
    let mut rng = StdRng::seed_from_u64(13);
    let frame = vec![9u8; 10_000];
    let cells: Vec<Cell> = fragment(&frame, &mut rng).unwrap().iter().map(|c| parse(c).unwrap()).collect();
    let mut r = Reassembler::new();
    let now = Instant::now();
    // A cell that claims a different total for the same frame id is refused.
    r.push(cells[0].clone(), now).unwrap();
    let mut liar = cells[1].clone();
    liar.total += 1;
    assert!(r.push(liar, now).is_err());
    // Duplicates are ignored, then the frame completes exactly once.
    for _ in 0..5 {
        assert!(r.push(cells[0].clone(), now).unwrap().is_none());
    }
    let mut done = None;
    for c in &cells[1..] {
        if let Some(f) = r.push(c.clone(), now).unwrap() {
            assert!(done.replace(f).is_none(), "delivered twice");
        }
    }
    assert_eq!(done.unwrap(), frame);
    // A frame whose cells do not hash to the claimed id is refused.
    let mut forged = cells.clone();
    forged[2].payload[0] ^= 1;
    let mut r = Reassembler::new();
    let mut last = Ok(None);
    for c in forged {
        last = r.push(c, now);
    }
    assert!(last.is_err(), "tampered cell must fail the hash check");
}

#[test]
fn control_cells_are_fixed_size_and_indistinguishable_in_length() {
    let mut rng = StdRng::seed_from_u64(14);
    assert_eq!(cover_cell(&mut rng).len(), CELL_SIZE);
    assert_eq!(ack_cell(&[1; 8], &mut rng).len(), CELL_SIZE);
    for c in fragment(&[7u8; 3], &mut rng).unwrap() {
        assert_eq!(c.len(), CELL_SIZE);
    }
}
