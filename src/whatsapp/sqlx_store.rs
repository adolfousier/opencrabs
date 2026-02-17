//! SQLx-backed WhatsApp session store
//!
//! Implements `wacore::store::Backend` using the project's existing `sqlx` SQLite
//! driver, avoiding the `libsqlite3-sys` version conflict that `whatsapp-rust-sqlite-storage`
//! (Diesel-based) would introduce.

use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::error::{db_err, Result, StoreError};
use wacore::store::traits::{
    AppStateSyncKey, AppSyncStore, DeviceListRecord, DeviceStore, LidPnMappingEntry, ProtocolStore,
    SignalStore,
};
use wacore::store::Device;

/// SQLx-backed storage for `whatsapp-rust`.
///
/// Uses a dedicated SQLite file at `~/.opencrabs/whatsapp/session.db`,
/// completely separate from the main OpenCrabs database.
#[derive(Clone)]
pub struct SqlxStore {
    pool: SqlitePool,
    device_id: i32,
}

impl SqlxStore {
    /// Open (or create) the store at the given path.
    pub async fn new(path: &str) -> Result<Self> {
        let opts = SqliteConnectOptions::from_str(path)
            .map_err(|e| StoreError::Connection(e.to_string()))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await
            .map_err(|e| StoreError::Connection(e.to_string()))?;

        let store = Self { pool, device_id: 1 };
        store.run_migrations().await?;
        Ok(store)
    }

    async fn run_migrations(&self) -> Result<()> {
        let sql = r#"
            CREATE TABLE IF NOT EXISTS wa_device (
                id          INTEGER PRIMARY KEY,
                data        BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS wa_identities (
                address     TEXT NOT NULL,
                device_id   INTEGER NOT NULL,
                key         BLOB NOT NULL,
                PRIMARY KEY (address, device_id)
            );
            CREATE TABLE IF NOT EXISTS wa_sessions (
                address     TEXT NOT NULL,
                device_id   INTEGER NOT NULL,
                record      BLOB NOT NULL,
                PRIMARY KEY (address, device_id)
            );
            CREATE TABLE IF NOT EXISTS wa_prekeys (
                id          INTEGER NOT NULL,
                device_id   INTEGER NOT NULL,
                record      BLOB NOT NULL,
                uploaded    INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (id, device_id)
            );
            CREATE TABLE IF NOT EXISTS wa_signed_prekeys (
                id          INTEGER NOT NULL,
                device_id   INTEGER NOT NULL,
                record      BLOB NOT NULL,
                PRIMARY KEY (id, device_id)
            );
            CREATE TABLE IF NOT EXISTS wa_sender_keys (
                address     TEXT NOT NULL,
                device_id   INTEGER NOT NULL,
                record      BLOB NOT NULL,
                PRIMARY KEY (address, device_id)
            );
            CREATE TABLE IF NOT EXISTS wa_app_state_keys (
                key_id      BLOB NOT NULL,
                device_id   INTEGER NOT NULL,
                data        TEXT NOT NULL,
                PRIMARY KEY (key_id, device_id)
            );
            CREATE TABLE IF NOT EXISTS wa_app_state_versions (
                name        TEXT NOT NULL,
                device_id   INTEGER NOT NULL,
                data        TEXT NOT NULL,
                PRIMARY KEY (name, device_id)
            );
            CREATE TABLE IF NOT EXISTS wa_app_state_mutation_macs (
                name        TEXT NOT NULL,
                version     INTEGER NOT NULL,
                index_mac   BLOB NOT NULL,
                value_mac   BLOB NOT NULL,
                device_id   INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_mutation_macs_lookup
                ON wa_app_state_mutation_macs (name, index_mac, device_id);
            CREATE TABLE IF NOT EXISTS wa_skdm_recipients (
                group_jid   TEXT NOT NULL,
                device_jid  TEXT NOT NULL,
                device_id   INTEGER NOT NULL,
                PRIMARY KEY (group_jid, device_jid, device_id)
            );
            CREATE TABLE IF NOT EXISTS wa_lid_pn_mapping (
                lid             TEXT NOT NULL,
                phone_number    TEXT NOT NULL,
                created_at      INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL,
                learning_source TEXT NOT NULL DEFAULT '',
                device_id       INTEGER NOT NULL,
                PRIMARY KEY (lid, device_id)
            );
            CREATE INDEX IF NOT EXISTS idx_lid_pn_phone
                ON wa_lid_pn_mapping (phone_number, device_id);
            CREATE TABLE IF NOT EXISTS wa_base_keys (
                address     TEXT NOT NULL,
                message_id  TEXT NOT NULL,
                base_key    BLOB NOT NULL,
                device_id   INTEGER NOT NULL,
                PRIMARY KEY (address, message_id, device_id)
            );
            CREATE TABLE IF NOT EXISTS wa_device_registry (
                user        TEXT NOT NULL,
                device_id   INTEGER NOT NULL,
                data        TEXT NOT NULL,
                PRIMARY KEY (user, device_id)
            );
            CREATE TABLE IF NOT EXISTS wa_sender_key_forget (
                group_jid   TEXT NOT NULL,
                participant TEXT NOT NULL,
                device_id   INTEGER NOT NULL,
                PRIMARY KEY (group_jid, participant, device_id)
            );
        "#;

        // Execute each statement separately (sqlx doesn't support multi-statement in one call)
        for stmt in sql.split(';') {
            let trimmed = stmt.trim();
            if trimmed.is_empty() {
                continue;
            }
            sqlx::query(trimmed)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        Ok(())
    }

    /// Check if a paired device record exists (valid MessagePack data).
    pub async fn device_exists(&self) -> Result<bool> {
        let row = sqlx::query("SELECT data FROM wa_device WHERE id = ?")
            .bind(self.device_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        match row {
            Some(r) => {
                let data: Vec<u8> = r.get("data");
                // Verify it's valid MessagePack (not legacy JSON)
                Ok(rmp_serde::from_slice::<Device>(&data).is_ok())
            }
            None => Ok(false),
        }
    }
}

// ─── SignalStore ───────────────────────────────────────────────────────────────

#[async_trait]
impl SignalStore for SqlxStore {
    async fn put_identity(&self, address: &str, key: [u8; 32]) -> Result<()> {
        sqlx::query(
            "INSERT INTO wa_identities (address, device_id, key) VALUES (?, ?, ?)
             ON CONFLICT(address, device_id) DO UPDATE SET key = excluded.key",
        )
        .bind(address)
        .bind(self.device_id)
        .bind(key.as_slice())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn load_identity(&self, address: &str) -> Result<Option<Vec<u8>>> {
        let row = sqlx::query(
            "SELECT key FROM wa_identities WHERE address = ? AND device_id = ?",
        )
        .bind(address)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.map(|r| r.get("key")))
    }

    async fn delete_identity(&self, address: &str) -> Result<()> {
        sqlx::query("DELETE FROM wa_identities WHERE address = ? AND device_id = ?")
            .bind(address)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_session(&self, address: &str) -> Result<Option<Vec<u8>>> {
        let row = sqlx::query(
            "SELECT record FROM wa_sessions WHERE address = ? AND device_id = ?",
        )
        .bind(address)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.map(|r| r.get("record")))
    }

    async fn put_session(&self, address: &str, session: &[u8]) -> Result<()> {
        sqlx::query(
            "INSERT INTO wa_sessions (address, device_id, record) VALUES (?, ?, ?)
             ON CONFLICT(address, device_id) DO UPDATE SET record = excluded.record",
        )
        .bind(address)
        .bind(self.device_id)
        .bind(session)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete_session(&self, address: &str) -> Result<()> {
        sqlx::query("DELETE FROM wa_sessions WHERE address = ? AND device_id = ?")
            .bind(address)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn store_prekey(&self, id: u32, record: &[u8], uploaded: bool) -> Result<()> {
        sqlx::query(
            "INSERT INTO wa_prekeys (id, device_id, record, uploaded) VALUES (?, ?, ?, ?)
             ON CONFLICT(id, device_id) DO UPDATE SET record = excluded.record, uploaded = excluded.uploaded",
        )
        .bind(id)
        .bind(self.device_id)
        .bind(record)
        .bind(uploaded)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn load_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        let row = sqlx::query(
            "SELECT record FROM wa_prekeys WHERE id = ? AND device_id = ?",
        )
        .bind(id)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.map(|r| r.get("record")))
    }

    async fn remove_prekey(&self, id: u32) -> Result<()> {
        sqlx::query("DELETE FROM wa_prekeys WHERE id = ? AND device_id = ?")
            .bind(id)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn store_signed_prekey(&self, id: u32, record: &[u8]) -> Result<()> {
        sqlx::query(
            "INSERT INTO wa_signed_prekeys (id, device_id, record) VALUES (?, ?, ?)
             ON CONFLICT(id, device_id) DO UPDATE SET record = excluded.record",
        )
        .bind(id)
        .bind(self.device_id)
        .bind(record)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn load_signed_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        let row = sqlx::query(
            "SELECT record FROM wa_signed_prekeys WHERE id = ? AND device_id = ?",
        )
        .bind(id)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.map(|r| r.get("record")))
    }

    async fn load_all_signed_prekeys(&self) -> Result<Vec<(u32, Vec<u8>)>> {
        let rows = sqlx::query(
            "SELECT id, record FROM wa_signed_prekeys WHERE device_id = ?",
        )
        .bind(self.device_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let id: u32 = r.get::<i64, _>("id") as u32;
                let record: Vec<u8> = r.get("record");
                (id, record)
            })
            .collect())
    }

    async fn remove_signed_prekey(&self, id: u32) -> Result<()> {
        sqlx::query("DELETE FROM wa_signed_prekeys WHERE id = ? AND device_id = ?")
            .bind(id)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn put_sender_key(&self, address: &str, record: &[u8]) -> Result<()> {
        sqlx::query(
            "INSERT INTO wa_sender_keys (address, device_id, record) VALUES (?, ?, ?)
             ON CONFLICT(address, device_id) DO UPDATE SET record = excluded.record",
        )
        .bind(address)
        .bind(self.device_id)
        .bind(record)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_sender_key(&self, address: &str) -> Result<Option<Vec<u8>>> {
        let row = sqlx::query(
            "SELECT record FROM wa_sender_keys WHERE address = ? AND device_id = ?",
        )
        .bind(address)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.map(|r| r.get("record")))
    }

    async fn delete_sender_key(&self, address: &str) -> Result<()> {
        sqlx::query("DELETE FROM wa_sender_keys WHERE address = ? AND device_id = ?")
            .bind(address)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

// ─── AppSyncStore ─────────────────────────────────────────────────────────────

#[async_trait]
impl AppSyncStore for SqlxStore {
    async fn get_sync_key(&self, key_id: &[u8]) -> Result<Option<AppStateSyncKey>> {
        let row = sqlx::query(
            "SELECT data FROM wa_app_state_keys WHERE key_id = ? AND device_id = ?",
        )
        .bind(key_id)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        match row {
            Some(r) => {
                let json: String = r.get("data");
                let key: AppStateSyncKey =
                    serde_json::from_str(&json).map_err(|e| StoreError::Serialization(e.to_string()))?;
                Ok(Some(key))
            }
            None => Ok(None),
        }
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        let json =
            serde_json::to_string(&key).map_err(|e| StoreError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO wa_app_state_keys (key_id, device_id, data) VALUES (?, ?, ?)
             ON CONFLICT(key_id, device_id) DO UPDATE SET data = excluded.data",
        )
        .bind(key_id)
        .bind(self.device_id)
        .bind(&json)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_version(&self, name: &str) -> Result<HashState> {
        let row = sqlx::query(
            "SELECT data FROM wa_app_state_versions WHERE name = ? AND device_id = ?",
        )
        .bind(name)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        match row {
            Some(r) => {
                let json: String = r.get("data");
                let state: HashState =
                    serde_json::from_str(&json).map_err(|e| StoreError::Serialization(e.to_string()))?;
                Ok(state)
            }
            None => Ok(HashState::default()),
        }
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        let json =
            serde_json::to_string(&state).map_err(|e| StoreError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO wa_app_state_versions (name, device_id, data) VALUES (?, ?, ?)
             ON CONFLICT(name, device_id) DO UPDATE SET data = excluded.data",
        )
        .bind(name)
        .bind(self.device_id)
        .bind(&json)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn put_mutation_macs(
        &self,
        name: &str,
        version: u64,
        mutations: &[AppStateMutationMAC],
    ) -> Result<()> {
        for m in mutations {
            sqlx::query(
                "INSERT INTO wa_app_state_mutation_macs (name, version, index_mac, value_mac, device_id)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(name)
            .bind(version as i64)
            .bind(&m.index_mac)
            .bind(&m.value_mac)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    async fn get_mutation_mac(&self, name: &str, index_mac: &[u8]) -> Result<Option<Vec<u8>>> {
        let row = sqlx::query(
            "SELECT value_mac FROM wa_app_state_mutation_macs
             WHERE name = ? AND index_mac = ? AND device_id = ?",
        )
        .bind(name)
        .bind(index_mac)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.map(|r| r.get("value_mac")))
    }

    async fn delete_mutation_macs(&self, name: &str, index_macs: &[Vec<u8>]) -> Result<()> {
        for mac in index_macs {
            sqlx::query(
                "DELETE FROM wa_app_state_mutation_macs
                 WHERE name = ? AND index_mac = ? AND device_id = ?",
            )
            .bind(name)
            .bind(mac.as_slice())
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }
}

// ─── ProtocolStore ────────────────────────────────────────────────────────────

#[async_trait]
impl ProtocolStore for SqlxStore {
    async fn get_skdm_recipients(&self, group_jid: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT device_jid FROM wa_skdm_recipients WHERE group_jid = ? AND device_id = ?",
        )
        .bind(group_jid)
        .bind(self.device_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(|r| r.get("device_jid")).collect())
    }

    async fn add_skdm_recipients(&self, group_jid: &str, device_jids: &[String]) -> Result<()> {
        for jid in device_jids {
            sqlx::query(
                "INSERT OR IGNORE INTO wa_skdm_recipients (group_jid, device_jid, device_id)
                 VALUES (?, ?, ?)",
            )
            .bind(group_jid)
            .bind(jid)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    async fn clear_skdm_recipients(&self, group_jid: &str) -> Result<()> {
        sqlx::query(
            "DELETE FROM wa_skdm_recipients WHERE group_jid = ? AND device_id = ?",
        )
        .bind(group_jid)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_lid_mapping(&self, lid: &str) -> Result<Option<LidPnMappingEntry>> {
        let row = sqlx::query(
            "SELECT lid, phone_number, created_at, updated_at, learning_source
             FROM wa_lid_pn_mapping WHERE lid = ? AND device_id = ?",
        )
        .bind(lid)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.map(|r| LidPnMappingEntry {
            lid: r.get("lid"),
            phone_number: r.get("phone_number"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
            learning_source: r.get("learning_source"),
        }))
    }

    async fn get_pn_mapping(&self, phone: &str) -> Result<Option<LidPnMappingEntry>> {
        let row = sqlx::query(
            "SELECT lid, phone_number, created_at, updated_at, learning_source
             FROM wa_lid_pn_mapping WHERE phone_number = ? AND device_id = ?",
        )
        .bind(phone)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.map(|r| LidPnMappingEntry {
            lid: r.get("lid"),
            phone_number: r.get("phone_number"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
            learning_source: r.get("learning_source"),
        }))
    }

    async fn put_lid_mapping(&self, entry: &LidPnMappingEntry) -> Result<()> {
        sqlx::query(
            "INSERT INTO wa_lid_pn_mapping (lid, phone_number, created_at, updated_at, learning_source, device_id)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(lid, device_id) DO UPDATE SET
                phone_number = excluded.phone_number,
                updated_at = excluded.updated_at,
                learning_source = excluded.learning_source",
        )
        .bind(&entry.lid)
        .bind(&entry.phone_number)
        .bind(entry.created_at)
        .bind(entry.updated_at)
        .bind(&entry.learning_source)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_all_lid_mappings(&self) -> Result<Vec<LidPnMappingEntry>> {
        let rows = sqlx::query(
            "SELECT lid, phone_number, created_at, updated_at, learning_source
             FROM wa_lid_pn_mapping WHERE device_id = ?",
        )
        .bind(self.device_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| LidPnMappingEntry {
                lid: r.get("lid"),
                phone_number: r.get("phone_number"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
                learning_source: r.get("learning_source"),
            })
            .collect())
    }

    async fn save_base_key(&self, address: &str, message_id: &str, base_key: &[u8]) -> Result<()> {
        sqlx::query(
            "INSERT INTO wa_base_keys (address, message_id, base_key, device_id) VALUES (?, ?, ?, ?)
             ON CONFLICT(address, message_id, device_id) DO UPDATE SET base_key = excluded.base_key",
        )
        .bind(address)
        .bind(message_id)
        .bind(base_key)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn has_same_base_key(
        &self,
        address: &str,
        message_id: &str,
        current_base_key: &[u8],
    ) -> Result<bool> {
        let row = sqlx::query(
            "SELECT base_key FROM wa_base_keys
             WHERE address = ? AND message_id = ? AND device_id = ?",
        )
        .bind(address)
        .bind(message_id)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        match row {
            Some(r) => {
                let stored: Vec<u8> = r.get("base_key");
                Ok(stored == current_base_key)
            }
            None => Ok(false),
        }
    }

    async fn delete_base_key(&self, address: &str, message_id: &str) -> Result<()> {
        sqlx::query(
            "DELETE FROM wa_base_keys WHERE address = ? AND message_id = ? AND device_id = ?",
        )
        .bind(address)
        .bind(message_id)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn update_device_list(&self, record: DeviceListRecord) -> Result<()> {
        let json =
            serde_json::to_string(&record).map_err(|e| StoreError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO wa_device_registry (user, device_id, data) VALUES (?, ?, ?)
             ON CONFLICT(user, device_id) DO UPDATE SET data = excluded.data",
        )
        .bind(&record.user)
        .bind(self.device_id)
        .bind(&json)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_devices(&self, user: &str) -> Result<Option<DeviceListRecord>> {
        let row = sqlx::query(
            "SELECT data FROM wa_device_registry WHERE user = ? AND device_id = ?",
        )
        .bind(user)
        .bind(self.device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        match row {
            Some(r) => {
                let json: String = r.get("data");
                let record: DeviceListRecord = serde_json::from_str(&json)
                    .map_err(|e| StoreError::Serialization(e.to_string()))?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    async fn mark_forget_sender_key(&self, group_jid: &str, participant: &str) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO wa_sender_key_forget (group_jid, participant, device_id)
             VALUES (?, ?, ?)",
        )
        .bind(group_jid)
        .bind(participant)
        .bind(self.device_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn consume_forget_marks(&self, group_jid: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT participant FROM wa_sender_key_forget
             WHERE group_jid = ? AND device_id = ?",
        )
        .bind(group_jid)
        .bind(self.device_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;

        let participants: Vec<String> = rows.into_iter().map(|r| r.get("participant")).collect();

        if !participants.is_empty() {
            sqlx::query(
                "DELETE FROM wa_sender_key_forget WHERE group_jid = ? AND device_id = ?",
            )
            .bind(group_jid)
            .bind(self.device_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        }
        Ok(participants)
    }
}

// ─── DeviceStore ──────────────────────────────────────────────────────────────

#[async_trait]
impl DeviceStore for SqlxStore {
    async fn save(&self, device: &Device) -> Result<()> {
        let bytes = rmp_serde::to_vec(device)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO wa_device (id, data) VALUES (?, ?)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
        )
        .bind(self.device_id)
        .bind(&bytes)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn load(&self) -> Result<Option<Device>> {
        let row = sqlx::query("SELECT data FROM wa_device WHERE id = ?")
            .bind(self.device_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        match row {
            Some(r) => {
                let data: Vec<u8> = r.get("data");
                let device: Device = match rmp_serde::from_slice(&data) {
                    Ok(d) => d,
                    Err(_) => {
                        // Old JSON-serialized data can't roundtrip (byte array issue).
                        // Delete it so the client re-pairs cleanly.
                        tracing::warn!("WhatsApp: clearing incompatible legacy device data — re-pair required");
                        let _ = sqlx::query("DELETE FROM wa_device WHERE id = ?")
                            .bind(self.device_id)
                            .execute(&self.pool)
                            .await;
                        return Ok(None);
                    }
                };
                Ok(Some(device))
            }
            None => Ok(None),
        }
    }

    async fn exists(&self) -> Result<bool> {
        let row = sqlx::query("SELECT 1 FROM wa_device WHERE id = ?")
            .bind(self.device_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.is_some())
    }

    async fn create(&self) -> Result<i32> {
        Ok(self.device_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> SqlxStore {
        SqlxStore::new(":memory:").await.unwrap()
    }

    #[tokio::test]
    async fn test_identity_roundtrip() {
        let store = test_store().await;
        let key = [42u8; 32];
        store.put_identity("alice@s.whatsapp.net", key).await.unwrap();

        let loaded = store.load_identity("alice@s.whatsapp.net").await.unwrap();
        assert_eq!(loaded.unwrap(), key.to_vec());
    }

    #[tokio::test]
    async fn test_identity_missing() {
        let store = test_store().await;
        let loaded = store.load_identity("nobody").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_identity_delete() {
        let store = test_store().await;
        store.put_identity("bob", [1u8; 32]).await.unwrap();
        store.delete_identity("bob").await.unwrap();
        assert!(store.load_identity("bob").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_session_roundtrip() {
        let store = test_store().await;
        let data = b"session-bytes";
        store.put_session("addr1", data).await.unwrap();
        let loaded = store.get_session("addr1").await.unwrap().unwrap();
        assert_eq!(loaded, data);
        assert!(store.has_session("addr1").await.unwrap());
        assert!(!store.has_session("nonexistent").await.unwrap());
    }

    #[tokio::test]
    async fn test_prekey_roundtrip() {
        let store = test_store().await;
        store.store_prekey(1, b"prekey-data", false).await.unwrap();
        let loaded = store.load_prekey(1).await.unwrap().unwrap();
        assert_eq!(loaded, b"prekey-data");
        store.remove_prekey(1).await.unwrap();
        assert!(store.load_prekey(1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_signed_prekey_roundtrip() {
        let store = test_store().await;
        store.store_signed_prekey(10, b"spk-data").await.unwrap();
        let loaded = store.load_signed_prekey(10).await.unwrap().unwrap();
        assert_eq!(loaded, b"spk-data");

        let all = store.load_all_signed_prekeys().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], (10, b"spk-data".to_vec()));

        store.remove_signed_prekey(10).await.unwrap();
        assert!(store.load_signed_prekey(10).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_sender_key_roundtrip() {
        let store = test_store().await;
        store.put_sender_key("group::sender", b"sk-data").await.unwrap();
        let loaded = store.get_sender_key("group::sender").await.unwrap().unwrap();
        assert_eq!(loaded, b"sk-data");
        store.delete_sender_key("group::sender").await.unwrap();
        assert!(store.get_sender_key("group::sender").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_app_sync_key_roundtrip() {
        let store = test_store().await;
        let key = AppStateSyncKey {
            key_data: vec![1, 2, 3],
            fingerprint: vec![4, 5],
            timestamp: 12345,
        };
        store.set_sync_key(b"kid1", key.clone()).await.unwrap();
        let loaded = store.get_sync_key(b"kid1").await.unwrap().unwrap();
        assert_eq!(loaded.key_data, key.key_data);
        assert_eq!(loaded.timestamp, key.timestamp);
    }

    #[tokio::test]
    async fn test_version_default() {
        let store = test_store().await;
        let state = store.get_version("critical_block").await.unwrap();
        assert_eq!(state.version, 0);
    }

    #[tokio::test]
    async fn test_skdm_recipients() {
        let store = test_store().await;
        store
            .add_skdm_recipients("group1", &["jid1".into(), "jid2".into()])
            .await
            .unwrap();
        let recipients = store.get_skdm_recipients("group1").await.unwrap();
        assert_eq!(recipients.len(), 2);
        store.clear_skdm_recipients("group1").await.unwrap();
        assert!(store.get_skdm_recipients("group1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_lid_mapping() {
        let store = test_store().await;
        let entry = LidPnMappingEntry {
            lid: "lid123".into(),
            phone_number: "+15551234".into(),
            created_at: 100,
            updated_at: 200,
            learning_source: "test".into(),
        };
        store.put_lid_mapping(&entry).await.unwrap();

        let by_lid = store.get_lid_mapping("lid123").await.unwrap().unwrap();
        assert_eq!(by_lid.phone_number, "+15551234");

        let by_phone = store.get_pn_mapping("+15551234").await.unwrap().unwrap();
        assert_eq!(by_phone.lid, "lid123");

        let all = store.get_all_lid_mappings().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn test_base_key_collision() {
        let store = test_store().await;
        store.save_base_key("addr", "msg1", b"key1").await.unwrap();
        assert!(store.has_same_base_key("addr", "msg1", b"key1").await.unwrap());
        assert!(!store.has_same_base_key("addr", "msg1", b"key2").await.unwrap());
        assert!(!store.has_same_base_key("addr", "msg2", b"key1").await.unwrap());
        store.delete_base_key("addr", "msg1").await.unwrap();
        assert!(!store.has_same_base_key("addr", "msg1", b"key1").await.unwrap());
    }

    #[tokio::test]
    async fn test_sender_key_forget_marks() {
        let store = test_store().await;
        store.mark_forget_sender_key("group1", "user1").await.unwrap();
        store.mark_forget_sender_key("group1", "user2").await.unwrap();

        let marks = store.consume_forget_marks("group1").await.unwrap();
        assert_eq!(marks.len(), 2);

        // Consumed — should be empty now
        let marks = store.consume_forget_marks("group1").await.unwrap();
        assert!(marks.is_empty());
    }

    #[tokio::test]
    async fn test_device_store_create_exists() {
        let store = test_store().await;
        assert!(!store.exists().await.unwrap());
        let id = store.create().await.unwrap();
        assert_eq!(id, 1);
        // create doesn't persist — only save does
        assert!(!store.exists().await.unwrap());
    }

    #[tokio::test]
    async fn test_mutation_macs() {
        let store = test_store().await;
        let macs = vec![
            AppStateMutationMAC {
                index_mac: vec![1, 2],
                value_mac: vec![3, 4],
            },
            AppStateMutationMAC {
                index_mac: vec![5, 6],
                value_mac: vec![7, 8],
            },
        ];
        store.put_mutation_macs("critical_block", 1, &macs).await.unwrap();

        let v = store.get_mutation_mac("critical_block", &[1, 2]).await.unwrap().unwrap();
        assert_eq!(v, vec![3, 4]);

        store.delete_mutation_macs("critical_block", &[vec![1, 2]]).await.unwrap();
        assert!(store.get_mutation_mac("critical_block", &[1, 2]).await.unwrap().is_none());
        // Second one still there
        assert!(store.get_mutation_mac("critical_block", &[5, 6]).await.unwrap().is_some());
    }
}
