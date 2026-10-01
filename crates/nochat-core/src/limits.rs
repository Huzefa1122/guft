//! Hard limits, enforced before any allocation or parsing of untrusted input.

/// Largest file we accept or send, in bytes (the spec says "less than 200 kB").
pub const MAX_FILE_BYTES: usize = 200_000;
/// Largest text message, in bytes.
pub const MAX_TEXT_BYTES: usize = 8 * 1024;
/// Longest accepted file name, in bytes.
pub const MAX_FILE_NAME_BYTES: usize = 128;
/// Longest accepted display name, in bytes.
pub const MAX_DISPLAY_NAME_BYTES: usize = 48;
/// Longest accepted invite string, in characters.
pub const MAX_INVITE_CHARS: usize = 8 * 1024;
/// Largest padded plaintext bucket (must hold the largest payload plus framing).
pub const MAX_PADDED_PLAINTEXT: usize = 256 * 1024;
/// Largest ciphertext frame accepted from the network, in bytes.
/// Plaintext bucket + Signal/SPQR overhead (PQ ratchet chunks, Kyber ciphertext).
pub const MAX_WIRE_BYTES: usize = MAX_PADDED_PLAINTEXT + 16 * 1024;
/// Most contacts we keep, bounding state size.
pub const MAX_CONTACTS: usize = 500;
/// Most outstanding (unused) invites we keep.
pub const MAX_PENDING_INVITES: usize = 32;
