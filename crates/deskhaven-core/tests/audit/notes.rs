//! The controller's session note (`{id, session_id, note}`, no uuid, no connection id): it
//! names a session, so it can only land on rows that carry that session id. One arriving
//! before them waits in a buffer bounded per source, per device and overall. The first
//! non-empty note of a session stays (06 §7).

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use deskhaven_core::audit::{
    MAX_NOTES_PER_DEVICE, MAX_NOTES_PER_SOURCE, NOTE_TTL, NoteOutcome, PAIRING_WINDOW,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

use super::support::{ID, SESSION, authorized, client, closed, ctx, new_conn, note};

#[tokio::test]
async fn the_first_note_of_a_session_stays_and_an_empty_one_clears_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.send("a", new_conn(7)).await;
        c.send("b", authorized(7, SESSION)).await;

        // Act
        let first = c
            .audit
            .record_note(note(ID, SESSION, "fixing\r\nprinter"), client(1));
        let first = first.await.unwrap();
        let second = c.audit.record_note(note(ID, SESSION, "other"), client(1));
        let second = second.await.unwrap();
        let empty = c
            .audit
            .record_note(note(ID, SESSION, "\u{200B}"), client(1));
        let empty = empty.await.unwrap();

        // Assert
        let b = db.backend;
        assert_eq!(first, NoteOutcome::Stored, "{b}");
        assert_eq!(
            (second, empty),
            (NoteOutcome::Ignored, NoteOutcome::Ignored),
            "{b}"
        );
        let rows = c.sessions().await;
        let kept = rows[0].controller_note.as_deref();
        assert_eq!(kept, Some("fixing\nprinter"), "{b}");
        assert_eq!(rows[0].note, None, "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_note_before_its_session_waits_for_it() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.send("a", new_conn(7)).await;

        // Act
        let early = c.audit.record_note(note(ID, SESSION, "early"), client(1));
        let early = early.await.unwrap();
        c.send("b", closed(7, SESSION)).await;

        // Assert
        let rows = c.sessions().await;
        assert_eq!(early, NoteOutcome::Pending, "{}", db.backend);
        assert_eq!(rows.len(), 1, "{}", db.backend);
        let kept = rows[0].controller_note.as_deref();
        assert_eq!(kept, Some("early"), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn a_waiting_note_expires_but_outlives_the_pairing_window() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        assert!(NOTE_TTL >= PAIRING_WINDOW);
        c.audit
            .record_note(note(ID, SESSION, "late"), client(1))
            .await
            .unwrap();
        c.audit
            .record_note(note(ID, SESSION + 1, "kept"), client(1))
            .await
            .unwrap();

        // Act
        c.clock.advance(PAIRING_WINDOW - Duration::from_millis(1));
        c.send("a", authorized(8, SESSION + 1)).await;
        c.clock.advance(NOTE_TTL);
        c.send("b", authorized(7, SESSION)).await;

        // Assert
        let rows = c.sessions().await;
        assert_eq!(
            rows[0].controller_note.as_deref(),
            Some("kept"),
            "{}",
            db.backend
        );
        assert_eq!(rows[1].controller_note, None, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn a_note_for_an_unknown_device_is_not_kept() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;

        // Act
        let outcome = c.audit.record_note(note("555555555", 1, "x"), client(1));

        // Assert
        assert_eq!(
            outcome.await.unwrap(),
            NoteOutcome::NotFound,
            "{}",
            db.backend
        );
    })
    .await;
}

/// One source (one IPv4 address or IPv6 /64) cannot fill the buffer for everyone, and
/// waiting notes are never evicted for newer ones.
#[tokio::test]
async fn waiting_notes_are_capped_per_source_and_per_device() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let devices: Vec<String> = (0..MAX_NOTES_PER_SOURCE)
            .map(|i| format!("{}", 200_000_000 + i))
            .collect();
        for device in &devices {
            c.register(device).await;
        }
        for (i, device) in devices.iter().enumerate() {
            let held = c
                .audit
                .record_note(note(device, i as u64 + 1, "n"), client(1));
            assert_eq!(held.await.unwrap(), NoteOutcome::Pending, "{}", db.backend);
        }

        // Act
        let over_source = c
            .audit
            .record_note(note(ID, 1, "n"), client(1))
            .await
            .unwrap();
        let mut per_device = Vec::new();
        for i in 0..=MAX_NOTES_PER_DEVICE {
            let held = c
                .audit
                .record_note(note(ID, 100 + i as u64, "n"), client(2));
            per_device.push(held.await.unwrap());
        }

        // Assert
        let b = db.backend;
        assert_eq!(over_source, NoteOutcome::Full, "{b}");
        let (last, kept) = per_device.split_last().unwrap();
        assert!(
            kept.iter().all(|o| *o == NoteOutcome::Pending),
            "{b}: {kept:?}"
        );
        assert_eq!(*last, NoteOutcome::Full, "{b}");
        c.send_from(&devices[0], "a", authorized(1, 1)).await;
        let rows = c.sessions().await;
        assert_eq!(
            rows[0].controller_note.as_deref(),
            Some("n"),
            "{b}: not evicted"
        );
    })
    .await;
}
