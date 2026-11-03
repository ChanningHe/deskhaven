//! The controller's session note (`{id, session_id, note}`, no uuid, no connection id): it
//! names a session, so it can only land on rows that carry that session id. One arriving
//! before them waits in a bounded buffer.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use deskhaven_core::audit::{ControllerNote, NOTE_TTL, NoteOutcome};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

use super::support::{ID, SESSION, authorized, closed, ctx, new_conn, session};

fn note(rustdesk_id: &str, text: &str) -> ControllerNote {
    ControllerNote {
        rustdesk_id: rustdesk_id.to_owned(),
        session_id: session(SESSION),
        note: text.to_owned(),
    }
}

#[tokio::test]
async fn a_note_lands_on_the_session_and_the_latest_one_wins() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.send("a", new_conn(7)).await;
        c.send("b", authorized(7, SESSION)).await;

        // Act
        let first = c.audit.record_note(note(ID, "first")).await.unwrap();
        let second = c
            .audit
            .record_note(note(ID, "fixing\r\nprinter"))
            .await
            .unwrap();

        // Assert
        let b = db.backend;
        assert_eq!(
            (first, second),
            (NoteOutcome::Stored, NoteOutcome::Stored),
            "{b}"
        );
        let rows = c.sessions().await;
        assert_eq!(
            rows[0].controller_note.as_deref(),
            Some("fixing\nprinter"),
            "{b}"
        );
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
        let early = c.audit.record_note(note(ID, "early")).await.unwrap();
        c.send("b", closed(7, SESSION)).await;

        // Assert
        let rows = c.sessions().await;
        assert_eq!(early, NoteOutcome::Pending, "{}", db.backend);
        assert_eq!(rows.len(), 1, "{}", db.backend);
        assert_eq!(
            rows[0].controller_note.as_deref(),
            Some("early"),
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn a_waiting_note_expires() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.audit.record_note(note(ID, "late")).await.unwrap();

        // Act
        c.clock.advance(NOTE_TTL + Duration::from_millis(1));
        c.send("a", authorized(7, SESSION)).await;

        // Assert
        assert_eq!(
            c.sessions().await[0].controller_note,
            None,
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn a_note_for_an_unknown_device_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;

        // Act
        let outcome = c.audit.record_note(note("555555555", "x")).await.unwrap();

        // Assert
        assert_eq!(outcome, NoteOutcome::NotFound, "{}", db.backend);
    })
    .await;
}
