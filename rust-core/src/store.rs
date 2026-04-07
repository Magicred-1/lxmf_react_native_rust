//! SQLite message persistence for LXMF messages and Solana transactions

use rusqlite::{Connection, params};
use std::sync::Mutex;

pub struct MessageStore {
    conn: Mutex<Connection>,
}

impl MessageStore {
    pub fn open(path: &str) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS messages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                dest_hash   BLOB NOT NULL,
                body        BLOB NOT NULL,
                outbound    INTEGER NOT NULL DEFAULT 0,
                timestamp   INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                acked       INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS solana_txs (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                signature   TEXT,
                raw_tx      BLOB NOT NULL,
                status      TEXT NOT NULL DEFAULT 'pending',
                timestamp   INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                relayed_via BLOB
            );
            CREATE INDEX IF NOT EXISTS idx_messages_dest ON messages(dest_hash);
            CREATE INDEX IF NOT EXISTS idx_txs_status ON solana_txs(status);
        ")?;

        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn insert_message(&self, dest_hash: &[u8; 16], body: &[u8], outbound: bool) -> Result<i64, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (dest_hash, body, outbound) VALUES (?1, ?2, ?3)",
            params![&dest_hash[..], body, outbound as i32],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn insert_solana_tx(&self, raw_tx: &[u8], relayed_via: Option<&[u8; 16]>) -> Result<i64, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO solana_txs (raw_tx, relayed_via) VALUES (?1, ?2)",
            params![raw_tx, relayed_via.map(|d| &d[..])],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_tx_status(&self, id: i64, status: &str, signature: Option<&str>) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE solana_txs SET status = ?1, signature = ?2 WHERE id = ?3",
            params![status, signature, id],
        )?;
        Ok(())
    }

    pub fn fetch_messages(&self, limit: u32) -> Result<String, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, hex(dest_hash), length(body), outbound, timestamp, acked FROM messages ORDER BY id DESC LIMIT ?1"
        )?;

        let rows: Vec<serde_json::Value> = stmt.query_map(params![limit], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "destHash": row.get::<_, String>(1)?,
                "bodyLen": row.get::<_, i32>(2)?,
                "outbound": row.get::<_, i32>(3)? != 0,
                "timestamp": row.get::<_, i64>(4)?,
                "acked": row.get::<_, i32>(5)? != 0,
            }))
        })?.filter_map(|r| r.ok()).collect();

        Ok(serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()))
    }

    pub fn fetch_pending_txs(&self) -> Result<String, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, hex(raw_tx), status, timestamp FROM solana_txs WHERE status = 'pending' ORDER BY id ASC"
        )?;

        let rows: Vec<serde_json::Value> = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "rawTxHex": row.get::<_, String>(1)?,
                "status": row.get::<_, String>(2)?,
                "timestamp": row.get::<_, i64>(3)?,
            }))
        })?.filter_map(|r| r.ok()).collect();

        Ok(serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()))
    }
}
