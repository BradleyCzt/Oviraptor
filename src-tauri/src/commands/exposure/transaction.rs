// §Q3 — short IMMEDIATE transactions with automatic rollback. The connection is only
// borrowed here, and the network never runs inside a transaction: a claim closes, the fetch
// happens, then a second short transaction records what was seen.

use rusqlite::Error as SqliteError;

pub struct ImmediateTx<'c> {
    connection: &'c rusqlite::Connection,
    finished: bool,
}

impl<'c> ImmediateTx<'c> {
    /// `BEGIN IMMEDIATE` takes the write lock at once, so two starters serialize instead of
    /// both passing the "is there already an active run?" check. A busy database is retried a
    /// bounded number of times and then reported, never swallowed.
    pub fn begin(connection: &'c rusqlite::Connection) -> Result<Self, String> {
        let mut attempts = 0u32;
        loop {
            match connection.execute_batch("BEGIN IMMEDIATE") {
                Ok(()) => {
                    return Ok(Self {
                        connection,
                        finished: false,
                    })
                }
                Err(error) if busy(&error) && attempts < 4 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(40 * u64::from(attempts)));
                }
                Err(error) => return Err(format!("数据库繁忙，未开始事务：{error}")),
            }
        }
    }

    /// Statements use the normal `rusqlite::params!` macro through this borrow.
    pub fn connection(&self) -> &rusqlite::Connection {
        self.connection
    }

    pub fn insert_row_id(&self) -> i64 {
        self.connection.last_insert_rowid()
    }

    pub fn commit(mut self) -> Result<(), String> {
        self.finished = true;
        self.connection
            .execute_batch("COMMIT")
            .map_err(|error| format!("提交失败：{error}"))
    }
}

impl Drop for ImmediateTx<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.connection.execute_batch("ROLLBACK");
        }
    }
}

fn busy(error: &SqliteError) -> bool {
    let text = error.to_string().to_ascii_lowercase();
    text.contains("database is locked") || text.contains("database is busy")
}
