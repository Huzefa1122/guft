use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::thread::sleep;
use std::time::Duration;

use nochat_core::Payload;
use nochat_store::{Body, Error, History, Locker, Profile, Status};
use tempfile::tempdir;

const PASS: &str = "correct horse battery";
const MARKER: &str = "SECRET-MARKER-9f3a1c";

fn text(t: &str) -> Payload {
    Payload::Text(t.into())
}

#[test]
fn history_roundtrip_pages_unread_and_files() {
    let dir = tempdir().unwrap();
    let h = History::open(&dir.path().join("h.db"), &[7u8; 32]).unwrap();
    for i in 0..5 {
        h.add("alice", 100 + i, i % 2 == 0, &text(&format!("m{i}"))).unwrap();
    }
    let file = Payload::File { name: "a.png".into(), data: vec![1, 2, 3] };
    let fid = h.add("bob", 200, false, &file).unwrap();

    let page = h.page("alice", None, 3).unwrap();
    assert_eq!(page.iter().map(|m| m.ts).collect::<Vec<_>>(), [104, 103, 102]);
    let older = h.page("alice", Some(page.last().unwrap().id), 10).unwrap();
    assert_eq!(older.len(), 2);

    assert_eq!(h.file_bytes(fid).unwrap().unwrap(), [1, 2, 3]);
    assert_eq!(h.page("bob", None, 5).unwrap()[0].body, Body::File { name: "a.png".into(), size: 3 });

    let s = h.summaries().unwrap();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].chat, "bob");
    assert_eq!(s[0].unread, 1);
    assert_eq!(s.iter().find(|c| c.chat == "alice").unwrap().unread, 2);
    h.mark_read("alice").unwrap();
    assert_eq!(h.summaries().unwrap().iter().find(|c| c.chat == "alice").unwrap().unread, 0);

    h.set_status(page[0].id, Status::Delivered).unwrap();
    assert_eq!(h.page("alice", None, 1).unwrap()[0].status, Status::Delivered);

    assert!(h.add("x", 1, true, &Payload::Hello { onion: "a".into(), name: "n".into() }).is_err());
    h.delete_chat("alice").unwrap();
    assert!(h.page("alice", None, 5).unwrap().is_empty());
}

#[test]
fn history_wrong_key_and_hostile_chat_names() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("h.db");
    let h = History::open(&path, &[1u8; 32]).unwrap();
    // SQL injection attempts are just data.
    let evil = "x'; DROP TABLE messages; --";
    h.add(evil, 1, true, &text("hi")).unwrap();
    assert_eq!(h.page(evil, None, 5).unwrap().len(), 1);
    h.close().unwrap();
    assert!(History::open(&path, &[2u8; 32]).is_err());
    assert!(History::open(&path, &[1u8; 32]).is_ok());
}

#[test]
fn outbox_keeps_order() {
    let dir = tempdir().unwrap();
    let h = History::open(&dir.path().join("h.db"), &[3u8; 32]).unwrap();
    let a = h.outbox_push("c", 1, b"first").unwrap();
    h.outbox_push("c", 2, b"second").unwrap();
    h.outbox_bump(a).unwrap();
    let q = h.outbox_for("c").unwrap();
    assert_eq!((q[0].frame.as_slice(), q[0].attempts), (b"first".as_slice(), 1));
    h.outbox_done(a).unwrap();
    assert_eq!(h.outbox_for("c").unwrap().len(), 1);
}

#[test]
fn profile_lifecycle_and_nothing_readable_on_disk() {
    let dir = tempdir().unwrap();
    let p = dir.path().join("me");
    assert!(!Profile::exists(&p));
    {
        let prof = Profile::create(&p, PASS, "Huzefa").unwrap();
        prof.history.add("someone", 1, true, &text(MARKER)).unwrap();
        prof.lock().unwrap();
    }
    assert!(Profile::exists(&p));
    assert!(matches!(Profile::create(&p, PASS, "x"), Err(Error::Exists)));
    assert!(Profile::unlock(&p, "wrong passphrase").is_err());

    for f in ["state.nc", "history.db"] {
        let bytes = fs::read(p.join(f)).unwrap();
        assert!(!bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()), "{f} leaks plaintext");
        assert!(!bytes.starts_with(b"SQLite format 3"), "{f} is an unencrypted SQLite file");
        assert_eq!(fs::metadata(p.join(f)).unwrap().permissions().mode() & 0o077, 0, "{f} permissions");
    }
    assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o077, 0);

    let prof = Profile::unlock(&p, PASS).unwrap();
    assert_eq!(prof.engine.display_name(), "Huzefa");
    assert_eq!(prof.history.page("someone", None, 5).unwrap().len(), 1);
}

#[test]
fn locker_relocks_on_idle_and_on_demand() {
    let dir = tempdir().unwrap();
    let p = dir.path().join("me");
    Profile::create(&p, PASS, "me").unwrap().lock().unwrap();

    let mut l = Locker::new(p, Duration::from_millis(150));
    assert!(matches!(l.with(|_| Ok(())), Err(Error::Locked)));
    l.unlock(PASS).unwrap();
    assert!(l.is_unlocked());

    // Activity keeps the session alive.
    for _ in 0..3 {
        sleep(Duration::from_millis(60));
        l.with(|p| p.history.add("c", 1, true, &text("hi")).map(|_| ())).unwrap();
    }
    assert!(l.is_unlocked());

    sleep(Duration::from_millis(200));
    assert!(l.tick().unwrap());
    assert!(!l.is_unlocked());
    assert!(matches!(l.with(|_| Ok(())), Err(Error::Locked)));

    l.unlock(PASS).unwrap();
    l.with(|p| {
        assert_eq!(p.history.page("c", None, 10).unwrap().len(), 3, "history survived the relock");
        Ok(())
    })
    .unwrap();
    l.lock().unwrap();
    l.lock().unwrap();
    assert!(!l.is_unlocked());
}

#[test]
fn locker_backs_off_after_wrong_passwords() {
    let dir = tempdir().unwrap();
    let p = dir.path().join("me");
    Profile::create(&p, PASS, "me").unwrap().lock().unwrap();
    let mut l = Locker::new(p, Duration::from_secs(60));
    for _ in 0..4 {
        assert!(matches!(l.unlock("nope nope nope"), Err(Error::Core(_))));
    }
    // Even the right password is refused during the wait.
    assert!(matches!(l.unlock(PASS), Err(Error::Backoff(_))));
}
