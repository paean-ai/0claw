use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Store(Arc<Mutex<Connection>>);

#[derive(serde::Serialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created_at: String,
}

#[derive(serde::Serialize, Clone)]
pub struct Message {
    pub id: i64,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<String>,
    pub created_at: String,
}

impl Store {
    pub fn new() -> Result<Self> {
        std::fs::create_dir_all("data")?;
        let conn = Connection::open("data/0claw.db")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                tool_calls TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    pub fn create_conversation(&self, id: &str, title: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT OR IGNORE INTO conversations (id, title) VALUES (?1, ?2)",
            params![id, title],
        )?;
        Ok(())
    }

    pub fn list_conversations(&self) -> Result<Vec<Conversation>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, title, created_at FROM conversations ORDER BY created_at DESC")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Conversation {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn add_message(
        &self,
        conv_id: &str,
        role: &str,
        content: &str,
        tool_calls: Option<&str>,
    ) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO messages (conversation_id, role, content, tool_calls) VALUES (?1, ?2, ?3, ?4)",
            params![conv_id, role, content, tool_calls],
        )?;
        Ok(())
    }

    pub fn get_messages(&self, conv_id: &str) -> Result<Vec<Message>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, role, content, tool_calls, created_at \
             FROM messages WHERE conversation_id = ?1 ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![conv_id], |r| {
                Ok(Message {
                    id: r.get(0)?,
                    conversation_id: r.get(1)?,
                    role: r.get(2)?,
                    content: r.get(3)?,
                    tool_calls: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
