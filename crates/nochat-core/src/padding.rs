//! Fixed-size-bucket padding (ISO/IEC 7816-4 style: data, 0x80, zeros).
//!
//! Every plaintext is padded to one of a few bucket sizes before encryption so
//! that a short message cannot be told apart from another of similar size.

use crate::error::{Error, Result};
use crate::limits::MAX_PADDED_PLAINTEXT;

const BUCKETS: [usize; 6] = [512, 2 * 1024, 8 * 1024, 32 * 1024, 128 * 1024, MAX_PADDED_PLAINTEXT];

pub fn pad(data: &[u8]) -> Result<Vec<u8>> {
    let needed = data.len() + 1;
    let bucket = BUCKETS
        .iter()
        .copied()
        .find(|b| *b >= needed)
        .ok_or(Error::Limit("payload too large"))?;
    let mut out = Vec::with_capacity(bucket);
    out.extend_from_slice(data);
    out.push(0x80);
    out.resize(bucket, 0);
    Ok(out)
}

pub fn unpad(mut data: Vec<u8>) -> Result<Vec<u8>> {
    if !BUCKETS.contains(&data.len()) {
        return Err(Error::Invalid("bad padding"));
    }
    while data.last() == Some(&0) {
        data.pop();
    }
    if data.pop() != Some(0x80) {
        return Err(Error::Invalid("bad padding"));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_buckets() {
        for len in [0usize, 1, 100, 511, 512, 5000, 200_000] {
            let data = vec![7u8; len];
            let padded = pad(&data).unwrap();
            assert!(BUCKETS.contains(&padded.len()));
            assert_eq!(unpad(padded).unwrap(), data);
        }
    }

    #[test]
    fn rejects_oversize_and_garbage() {
        assert!(pad(&vec![0u8; MAX_PADDED_PLAINTEXT]).is_err());
        assert!(unpad(vec![0u8; 512]).is_err());
        assert!(unpad(vec![1u8; 100]).is_err());
    }

    #[test]
    fn data_ending_in_zeros_survives() {
        let data = vec![1, 0, 0, 0];
        assert_eq!(unpad(pad(&data).unwrap()).unwrap(), data);
    }
}
