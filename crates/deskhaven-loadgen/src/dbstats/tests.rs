use super::*;

fn writes(inserted: i64, updated: i64, deleted: i64) -> RowWrites {
    RowWrites {
        inserted,
        updated,
        deleted,
    }
}

#[test]
fn since_subtracts_per_table_and_drops_idle_tables() {
    let earlier = Snapshot {
        tables: BTreeMap::from([
            ("devices".to_owned(), writes(10, 5, 0)),
            ("users".to_owned(), writes(1, 0, 0)),
        ]),
        commits: Some(100),
    };
    let later = Snapshot {
        tables: BTreeMap::from([
            ("devices".to_owned(), writes(10, 25, 1)),
            ("users".to_owned(), writes(1, 0, 0)),
            ("audit_conn".to_owned(), writes(3, 0, 0)),
        ]),
        commits: Some(160),
    };

    let delta = later.since(&earlier);

    assert_eq!(delta.table("devices"), writes(0, 20, 1));
    assert_eq!(delta.table("audit_conn"), writes(3, 0, 0));
    assert!(!delta.tables.contains_key("users"));
    assert_eq!(delta.commits, Some(60));
}

#[test]
fn only_plain_identifiers_are_spliced_into_sql() {
    assert!(is_plain_identifier("devices"));
    assert!(is_plain_identifier("_seaql_migrations"));
    assert!(is_plain_identifier("t2"));
    assert!(!is_plain_identifier("2t"));
    assert!(!is_plain_identifier(""));
    assert!(!is_plain_identifier("a'b"));
    assert!(!is_plain_identifier("a b"));
}

#[tokio::test]
async fn sqlite_triggers_count_every_written_row() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("t.db").display());
    let probe = DbProbe::open(&url).await.unwrap();
    let conn = &probe.conn;
    conn.execute_unprepared("CREATE TABLE devices (id INTEGER PRIMARY KEY, seen INTEGER)")
        .await
        .unwrap();
    conn.execute_unprepared("CREATE TABLE \"odd name\" (id INTEGER)")
        .await
        .unwrap();
    probe.install().await.unwrap();
    probe.install().await.unwrap(); // idempotent
    let before = probe.snapshot().await.unwrap();

    // Act: 3 inserts, one statement updating 2 rows, 1 delete.
    for sql in [
        "INSERT INTO devices (id, seen) VALUES (1, 0), (2, 0), (3, 0)",
        "UPDATE devices SET seen = 1 WHERE id <= 2",
        "UPDATE devices SET seen = 2 WHERE id = 99",
        "DELETE FROM devices WHERE id = 3",
        "INSERT INTO \"odd name\" (id) VALUES (1)",
    ] {
        conn.execute_unprepared(sql).await.unwrap();
    }
    let after = probe.snapshot().await.unwrap();

    // Assert
    let delta = after.since(&before);
    assert_eq!(delta.table("devices"), writes(3, 2, 1));
    assert!(
        !delta.tables.contains_key("odd name"),
        "skipped, not quoted"
    );
    assert!(!delta.tables.contains_key(COUNTER_TABLE));
    assert_eq!(delta.commits, None);
}
