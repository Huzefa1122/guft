//! Fixed-size cells. Every cell on the wire is exactly [`CELL_SIZE`] bytes,
//! whether it carries data, an acknowledgement, or nothing (cover traffic).
//!
//! ```text
//! kind(1) | frame_id(8) | seq(2) | total(2) | len(2) | payload / random padding
//! ```

use rand::RngCore;
use sha2::{Digest, Sha256};

use guft_core::limits::MAX_WIRE_BYTES;

use crate::{Error, Result};

pub const CELL_SIZE: usize = 2048;
const HEADER: usize = 15;
pub const CELL_PAYLOAD: usize = CELL_SIZE - HEADER;
/// Most cells a single frame may span.
pub const MAX_CELLS: usize = MAX_WIRE_BYTES.div_ceil(CELL_PAYLOAD);

pub type RawCell = [u8; CELL_SIZE];
pub type FrameId = [u8; 8];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Data = 1,
    Ack = 2,
    Cover = 3,
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub kind: Kind,
    pub frame_id: FrameId,
    pub seq: u16,
    pub total: u16,
    pub payload: Vec<u8>,
}

/// Identifies a frame across circuits; also checked after reassembly.
pub fn frame_id(frame: &[u8]) -> FrameId {
    let mut id = [0u8; 8];
    id.copy_from_slice(&Sha256::digest(frame)[..8]);
    id
}

fn blank(rng: &mut impl RngCore) -> RawCell {
    let mut c = [0u8; CELL_SIZE];
    rng.fill_bytes(&mut c[HEADER..]);
    c
}

fn put(c: &mut RawCell, kind: Kind, id: &FrameId, seq: u16, total: u16, len: u16) {
    c[0] = kind as u8;
    c[1..9].copy_from_slice(id);
    c[9..11].copy_from_slice(&seq.to_be_bytes());
    c[11..13].copy_from_slice(&total.to_be_bytes());
    c[13..15].copy_from_slice(&len.to_be_bytes());
}

/// Split a frame into data cells. Padding bytes are random.
pub fn fragment(frame: &[u8], rng: &mut impl RngCore) -> Result<Vec<RawCell>> {
    if frame.is_empty() || frame.len() > MAX_WIRE_BYTES {
        return Err(Error::TooLarge);
    }
    let id = frame_id(frame);
    let total = frame.len().div_ceil(CELL_PAYLOAD);
    Ok(frame
        .chunks(CELL_PAYLOAD)
        .enumerate()
        .map(|(i, chunk)| {
            let mut c = blank(rng);
            put(&mut c, Kind::Data, &id, i as u16, total as u16, chunk.len() as u16);
            c[HEADER..HEADER + chunk.len()].copy_from_slice(chunk);
            c
        })
        .collect())
}

pub fn ack_cell(id: &FrameId, rng: &mut impl RngCore) -> RawCell {
    let mut c = blank(rng);
    put(&mut c, Kind::Ack, id, 0, 0, 0);
    c
}

pub fn cover_cell(rng: &mut impl RngCore) -> RawCell {
    let mut c = blank(rng);
    put(&mut c, Kind::Cover, &[0; 8], 0, 0, 0);
    c
}

/// Strictly validate a cell from the network.
pub fn parse(raw: &RawCell) -> Result<Cell> {
    let kind = match raw[0] {
        1 => Kind::Data,
        2 => Kind::Ack,
        3 => Kind::Cover,
        _ => return Err(Error::Protocol("unknown cell kind")),
    };
    let frame_id: FrameId = raw[1..9].try_into().expect("8 bytes");
    let seq = u16::from_be_bytes([raw[9], raw[10]]);
    let total = u16::from_be_bytes([raw[11], raw[12]]);
    let len = u16::from_be_bytes([raw[13], raw[14]]) as usize;
    match kind {
        Kind::Data => {
            if len == 0 || len > CELL_PAYLOAD || total == 0 || total as usize > MAX_CELLS || seq >= total {
                return Err(Error::Protocol("bad data cell"));
            }
            // Only the last cell may be short.
            if seq + 1 < total && len != CELL_PAYLOAD {
                return Err(Error::Protocol("short non-final cell"));
            }
        }
        Kind::Ack | Kind::Cover => {
            if len != 0 || seq != 0 || total != 0 {
                return Err(Error::Protocol("bad control cell"));
            }
        }
    }
    Ok(Cell { kind, frame_id, seq, total, payload: raw[HEADER..HEADER + len].to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng() -> impl RngCore {
        rand::rng()
    }

    #[test]
    fn cells_are_all_the_same_size_and_roundtrip() {
        let frame: Vec<u8> = (0..5000u32).map(|i| i as u8).collect();
        let cells = fragment(&frame, &mut rng()).unwrap();
        assert_eq!(cells.len(), 3);
        let mut out = Vec::new();
        for (i, c) in cells.iter().enumerate() {
            let p = parse(c).unwrap();
            assert_eq!((p.kind, p.seq as usize, p.total), (Kind::Data, i, 3));
            out.extend_from_slice(&p.payload);
        }
        assert_eq!(out, frame);
        assert_eq!(parse(&ack_cell(&frame_id(&frame), &mut rng())).unwrap().kind, Kind::Ack);
        assert_eq!(parse(&cover_cell(&mut rng())).unwrap().kind, Kind::Cover);
    }

    #[test]
    fn size_limits() {
        assert!(fragment(&[], &mut rng()).is_err());
        assert!(fragment(&vec![0; MAX_WIRE_BYTES + 1], &mut rng()).is_err());
        assert_eq!(fragment(&vec![0; MAX_WIRE_BYTES], &mut rng()).unwrap().len(), MAX_CELLS);
    }

    #[test]
    fn parse_rejects_malformed() {
        let mut ok = fragment(&[1; 4000], &mut rng()).unwrap();
        let mut c = ok.remove(0);
        c[0] = 9;
        assert!(parse(&c).is_err());
        let mut c = fragment(&[1; 4000], &mut rng()).unwrap().remove(0);
        c[13..15].copy_from_slice(&0u16.to_be_bytes());
        assert!(parse(&c).is_err(), "zero length");
        let mut c = fragment(&[1; 4000], &mut rng()).unwrap().remove(0);
        c[13..15].copy_from_slice(&(CELL_PAYLOAD as u16 + 1).to_be_bytes());
        assert!(parse(&c).is_err(), "oversize length");
        let mut c = fragment(&[1; 4000], &mut rng()).unwrap().remove(0);
        c[9..11].copy_from_slice(&5u16.to_be_bytes());
        assert!(parse(&c).is_err(), "seq >= total");
        let mut c = fragment(&[1; 4000], &mut rng()).unwrap().remove(0);
        c[11..13].copy_from_slice(&u16::MAX.to_be_bytes());
        assert!(parse(&c).is_err(), "huge total");
        let mut c = fragment(&[1; 4000], &mut rng()).unwrap().remove(0);
        c[13..15].copy_from_slice(&10u16.to_be_bytes());
        assert!(parse(&c).is_err(), "short non-final cell");
        let mut c = cover_cell(&mut rng());
        c[13] = 1;
        assert!(parse(&c).is_err(), "cover with payload");
    }
}
