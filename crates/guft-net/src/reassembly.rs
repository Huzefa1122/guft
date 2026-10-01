//! Rebuilds frames from cells that may arrive on different circuits, in any
//! order, with duplicates. Memory use is bounded.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::cell::{frame_id, Cell, FrameId};
use crate::{Error, Result};

/// Frames we will rebuild at once.
pub const MAX_PARTIALS: usize = 32;
/// Total bytes held in unfinished frames.
pub const MAX_PARTIAL_BYTES: usize = 4 * 1024 * 1024;
/// Unfinished frames are dropped after this long.
pub const PARTIAL_TTL: Duration = Duration::from_secs(300);

struct Partial {
    total: u16,
    parts: Vec<Option<Vec<u8>>>,
    got: u16,
    bytes: usize,
    started: Instant,
}

#[derive(Default)]
pub struct Reassembler {
    partials: HashMap<FrameId, Partial>,
    bytes: usize,
}

impl Reassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a validated data cell. Returns the whole frame once complete.
    pub fn push(&mut self, cell: Cell, now: Instant) -> Result<Option<Vec<u8>>> {
        self.expire(now);
        let total = cell.total as usize;
        if !self.partials.contains_key(&cell.frame_id) {
            if self.partials.len() >= MAX_PARTIALS {
                return Err(Error::Protocol("too many frames in flight"));
            }
            self.partials.insert(
                cell.frame_id,
                Partial { total: cell.total, parts: vec![None; total], got: 0, bytes: 0, started: now },
            );
        }
        let p = self.partials.get_mut(&cell.frame_id).expect("just inserted");
        if p.total != cell.total {
            return Err(Error::Protocol("inconsistent cell count"));
        }
        let slot = &mut p.parts[cell.seq as usize];
        if slot.is_some() {
            return Ok(None); // duplicate cell
        }
        if self.bytes + cell.payload.len() > MAX_PARTIAL_BYTES {
            return Err(Error::Protocol("reassembly memory limit"));
        }
        self.bytes += cell.payload.len();
        p.bytes += cell.payload.len();
        *slot = Some(cell.payload);
        p.got += 1;
        if p.got < p.total {
            return Ok(None);
        }

        let done = self.partials.remove(&cell.frame_id).expect("present");
        self.bytes -= done.bytes;
        let frame: Vec<u8> = done.parts.into_iter().flatten().flatten().collect();
        if frame_id(&frame) != cell.frame_id {
            return Err(Error::Protocol("frame hash mismatch"));
        }
        Ok(Some(frame))
    }

    pub fn expire(&mut self, now: Instant) {
        let before = self.bytes;
        let mut freed = 0;
        self.partials.retain(|_, p| {
            let keep = now.duration_since(p.started) < PARTIAL_TTL;
            if !keep {
                freed += p.bytes;
            }
            keep
        });
        self.bytes = before - freed;
    }

    pub fn in_flight(&self) -> usize {
        self.partials.len()
    }

    pub fn held_bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{fragment, parse};
    use rand::seq::SliceRandom;

    fn cells(frame: &[u8]) -> Vec<Cell> {
        fragment(frame, &mut rand::rng()).unwrap().iter().map(|c| parse(c).unwrap()).collect()
    }

    #[test]
    fn any_order_with_duplicates() {
        let frame: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let mut cs = cells(&frame);
        cs.extend(cs.clone());
        cs.shuffle(&mut rand::rng());
        let mut r = Reassembler::new();
        let now = Instant::now();
        let mut got = None;
        for c in cs {
            if let Some(f) = r.push(c, now).unwrap() {
                assert!(got.is_none(), "completed twice");
                got = Some(f);
            }
        }
        // Duplicates arriving after completion start a fresh partial; it must expire.
        assert_eq!(got.unwrap(), frame);
        r.expire(now + PARTIAL_TTL + Duration::from_secs(1));
        assert_eq!((r.in_flight(), r.held_bytes()), (0, 0));
    }

    #[test]
    fn interleaved_frames() {
        let (a, b) = (vec![1u8; 9000], vec![2u8; 7000]);
        let mut all = cells(&a);
        all.extend(cells(&b));
        all.shuffle(&mut rand::rng());
        let mut r = Reassembler::new();
        let done: Vec<_> = all.into_iter().filter_map(|c| r.push(c, Instant::now()).unwrap()).collect();
        assert_eq!(done.len(), 2);
        assert!(done.contains(&a) && done.contains(&b));
    }

    #[test]
    fn limits_hold() {
        let mut r = Reassembler::new();
        let now = Instant::now();
        // One cell of many distinct, never-completed frames.
        for i in 0..MAX_PARTIALS {
            let mut c = cells(&vec![i as u8; 5000]).remove(0);
            c.frame_id = [i as u8; 8];
            r.push(c, now).unwrap();
        }
        let mut extra = cells(&[9u8; 5000]).remove(0);
        extra.frame_id = [200; 8];
        assert!(r.push(extra, now).is_err());
        assert!(r.held_bytes() <= MAX_PARTIAL_BYTES);
    }

    #[test]
    fn tampered_content_rejected() {
        let frame = vec![5u8; 6000];
        let mut cs = cells(&frame);
        cs[1].payload[0] ^= 1;
        let mut r = Reassembler::new();
        let results: Vec<_> = cs.into_iter().map(|c| r.push(c, Instant::now())).collect();
        assert!(results.last().unwrap().is_err());
    }

    #[test]
    fn inconsistent_total_rejected() {
        let mut r = Reassembler::new();
        let mut cs = cells(&vec![3u8; 6000]);
        r.push(cs.remove(0), Instant::now()).unwrap();
        let mut bad = cs.remove(0);
        bad.total += 1;
        assert!(r.push(bad, Instant::now()).is_err());
    }
}
