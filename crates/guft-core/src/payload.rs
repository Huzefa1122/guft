//! The plaintext carried inside each encrypted message.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::limits::*;
use crate::padding;
use crate::store::RoomKind;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Payload {
    /// Sent first, so the recipient learns where to reply. `auth` is the Tor client
    /// authorization key they must use to reach our onion service.
    Hello { onion: String, name: String, auth: [u8; 32] },
    /// Replaces the key an invite carried with a permanent per-contact key.
    AuthKey { key: [u8; 32] },
    /// Sent by a member who added you; you are now in the room.
    RoomInvite { room: [u8; 16], name: String, creator: String, open_invites: bool, kind: RoomKind, members: Vec<RosterEntry> },
    RoomText { room: [u8; 16], text: String },
    RoomFile { room: [u8; 16], name: String, data: Vec<u8> },
    /// A new member asks the member who added them to pass on a one-time invite to `to`.
    Introduce { room: [u8; 16], to: String, invite: String, code: String },
    /// The relayed form: `from` is the new member, and the invite lets you reach them.
    Introduction { room: [u8; 16], from: RosterEntry, invite: String, code: String },
    RoomLeave { room: [u8; 16] },
    RoomRemove { room: [u8; 16], member: String },
    RoomRename { room: [u8; 16], name: String },
    Text(String),
    File { name: String, data: Vec<u8> },
}

impl Payload {
    pub fn validate(&self) -> Result<()> {
        match self {
            Payload::Hello { onion, name, .. } => {
                check_onion(onion)?;
                check_display_name(name)
            }
            Payload::AuthKey { .. } | Payload::RoomLeave { .. } => Ok(()),
            Payload::RoomInvite { name, creator, members, kind, open_invites, .. } => {
                if *kind == RoomKind::Direct && (members.len() != 1 || *open_invites) {
                    return Err(Error::Invalid("bad one-to-one room"));
                }
                check_display_name(name)?;
                check_contact_id(creator)?;
                if members.len() >= MAX_ROOM_MEMBERS {
                    return Err(Error::Limit("room too large"));
                }
                members.iter().try_for_each(RosterEntry::validate)
            }
            Payload::RoomText { text, .. } => {
                if text.len() > MAX_TEXT_BYTES {
                    return Err(Error::Limit("text too long"));
                }
                Ok(())
            }
            Payload::RoomFile { name, data, .. } => {
                if data.len() > MAX_FILE_BYTES {
                    return Err(Error::Limit("file too large"));
                }
                check_file_name(name)
            }
            Payload::Introduce { to, invite, code, .. } => {
                check_contact_id(to)?;
                check_invite_and_code(invite, code)
            }
            Payload::Introduction { from, invite, code, .. } => {
                from.validate()?;
                check_invite_and_code(invite, code)
            }
            Payload::RoomRemove { member, .. } => check_contact_id(member),
            Payload::RoomRename { name, .. } => check_display_name(name),
            Payload::Text(t) => {
                if t.len() > MAX_TEXT_BYTES {
                    return Err(Error::Limit("text too long"));
                }
                Ok(())
            }
            Payload::File { name, data } => {
                if data.len() > MAX_FILE_BYTES {
                    return Err(Error::Limit("file too large"));
                }
                check_file_name(name)
            }
        }
    }

    /// Serialize and pad to a bucket size, ready for encryption.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        padding::pad(&postcard::to_allocvec(self)?)
    }

    /// Inverse of [`encode`]; validates everything the sender claims.
    pub fn decode(padded: Vec<u8>) -> Result<Self> {
        let raw = padding::unpad(padded)?;
        let p: Payload = postcard::from_bytes(&raw)?;
        p.validate()?;
        Ok(p)
    }
}

/// Someone in a room: their id, the name they chose, and where to reach them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub id: String,
    pub name: String,
    pub onion: String,
}

impl RosterEntry {
    pub fn validate(&self) -> Result<()> {
        check_contact_id(&self.id)?;
        check_display_name(&self.name)?;
        check_onion(&self.onion)
    }
}

/// A contact id: 32 lowercase hex characters.
pub fn check_contact_id(id: &str) -> Result<()> {
    if id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(Error::Invalid("bad contact id"))
    }
}

fn check_invite_and_code(invite: &str, code: &str) -> Result<()> {
    if invite.len() > MAX_INVITE_CHARS || code.len() > 64 {
        return Err(Error::Limit("invite too long"));
    }
    Ok(())
}

/// Characters that are invisible or reorder text. They let a name or file name look like
/// something else (`invoice<RLO>fdp.exe` displays as `invoiceexe.pdf`) or look identical to
/// another name, so peer-supplied names may not contain them. Emoji variation selector-16
/// (U+FE0F) stays allowed: many ordinary emoji need it.
fn is_spoofing_char(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}'..='\u{1160}' | '\u{17B4}'..='\u{17B5}'
        | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{3164}' | '\u{FE00}'..='\u{FE0E}' | '\u{FEFF}' | '\u{FFA0}'
        | '\u{FFF9}'..='\u{FFFB}' | '\u{E0000}'..='\u{E007F}')
}

pub fn check_display_name(name: &str) -> Result<()> {
    if name.len() > MAX_DISPLAY_NAME_BYTES || name.chars().any(|c| c.is_control() || is_spoofing_char(c)) {
        return Err(Error::Invalid("bad display name"));
    }
    Ok(())
}

/// A v3 onion address: 56 base32 chars + ".onion".
pub fn check_onion(onion: &str) -> Result<()> {
    let host = onion.strip_suffix(".onion").ok_or(Error::Invalid("bad onion address"))?;
    let ok = host.len() == 56 && host.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'));
    if ok { Ok(()) } else { Err(Error::Invalid("bad onion address")) }
}

/// File names come from the network: no paths, no control chars, no hidden or
/// reserved names, so a hostile name can never escape the download directory.
pub fn check_file_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name.len() > MAX_FILE_NAME_BYTES
        || name.starts_with('.')
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|c| c.is_control() || is_spoofing_char(c) || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'));
    if bad { Err(Error::Invalid("bad file name")) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_rules() {
        for ok in ["a.txt", "photo 1.png"] {
            assert!(check_file_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".bashrc", "../x", "a/b", "a\\b", "x\0y", "con:", "trail.", "new\nline"] {
            assert!(check_file_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn roundtrip_and_limits() {
        let p = Payload::Text("hello".into());
        assert_eq!(Payload::decode(p.encode().unwrap()).unwrap(), p);
        let big = Payload::File { name: "a".into(), data: vec![0; MAX_FILE_BYTES + 1] };
        assert!(big.encode().is_err());
        let max = Payload::File { name: "a".into(), data: vec![9; MAX_FILE_BYTES] };
        assert_eq!(Payload::decode(max.encode().unwrap()).unwrap(), max);
    }

    #[test]
    fn onion_rules() {
        let good = format!("{}.onion", "a".repeat(56));
        assert!(check_onion(&good).is_ok());
        assert!(check_onion("short.onion").is_err());
        assert!(check_onion(&format!("{}.com", "a".repeat(56))).is_err());
        assert!(check_onion(&format!("{}.onion", "A".repeat(56))).is_err());
    }
}
