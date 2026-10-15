use rusqlite::Connection;
use std::path::PathBuf;

pub(super) struct Fixture {
    root: PathBuf,
    pub path: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-event-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("events.sqlite3");
        let connection = crate::db::open(&path).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        // Pump/signal tests use the real event table and their own synthetic
        // emitter below. Stop before production emitters: those triggers need
        // the complete Agent schema, which is outside this queue fixture.
        let (event_table, _) = super::COLLABORATION_EVENT_SCHEMA
            .split_once("CREATE TRIGGER")
            .unwrap();
        connection.execute_batch(event_table).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE test_messages (id INTEGER PRIMARY KEY, body TEXT NOT NULL);
             CREATE TRIGGER test_message_event AFTER INSERT ON test_messages BEGIN
               INSERT INTO agent_collaboration_events
                 (scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
               VALUES ('task-a',2,'message',CAST(NEW.id AS TEXT),'inserted',
                 json_object('body',NEW.body));
             END;",
            )
            .unwrap();
        Self { root, path }
    }

    pub fn writer(&self) -> Connection {
        crate::db::open(&self.path).unwrap()
    }

    pub fn insert(connection: &Connection, body: &str) {
        connection
            .execute("INSERT INTO test_messages(body) VALUES (?1)", [body])
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Only this fixture's freshly created UUID directory, never user data.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
