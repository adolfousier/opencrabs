//! Rusqlite-backed WhatsApp session store
//!
//! Implements `wacore::store::Backend` using deadpool-sqlite + rusqlite,
//! matching the rest of the OpenCrabs database layer.

use async_trait::async_trait;
use deadpool_sqlite::{Config, Hook, Pool, Runtime};
use rusqlite::params;

use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::Device;
use wacore::store::error::{Result, StoreError, db_err};
use wacore::store::traits::{
    AppStateSyncKey, AppSyncStore, DeviceListRecord, DeviceStore, LidPnMappingEntry, ProtocolStore,
    SignalStore, TcTokenEntry,
};
use wacore_binary::jid::Jid;

/// Map a deadpool InteractError to StoreError
fn interact_to_store_err(e: deadpool_sqlite::InteractError) -> StoreError {
    StoreError::Database(format!("interact error: {}", e))
}

/// Map a deadpool PoolError to StoreError
fn pool_err(e: deadpool_sqlite::PoolError) -> StoreError {
    StoreError::Connection(format!("pool error: {}", e))
}

/// Rusqlite-backed storage for `whatsapp-rust`.
///
/// Uses a dedicated SQLite file at `~/.opencrabs/whatsapp/session.db`,
/// completely separate from the main OpenCrabs database.
#[derive(Clone)]
pub struct Store {
    pool: Pool,
    device_id: i32,
}

impl Store {
    /// Open (or create) the store at the given path.
    pub async fn new(path: &str) -> Result<Self> {
        let pool = Config::new(path)
            .builder(Runtime::Tokio1)
            .map_err(|e| StoreError::Connection(e.to_string()))?
            .max_size(4)
            .post_create(Hook::async_fn(|conn, _| {
                Box::pin(async move {
                    conn.interact(|conn| {
                        conn.execute_batch(
                            "PRAGMA journal_mode = WAL;
                             PRAGMA busy_timeout = 10000;",
                        )
                    })
                    .await
                    .map_err(|e| deadpool_sqlite::HookError::Message(e.to_string().into()))?
                    .map_err(|e| deadpool_sqlite::HookError::Message(e.to_string().into()))?;
                    Ok(())
                })
            }))
            .build()
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
            CREATE TABLE IF NOT EXISTS wa_tc_tokens (
                jid              TEXT NOT NULL,
                token            BLOB NOT NULL,
                token_timestamp  INTEGER NOT NULL,
                sender_timestamp INTEGER,
                device_id        INTEGER NOT NULL,
                PRIMARY KEY (jid, device_id)
            );
        "#;

        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| conn.execute_batch(sql))
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    /// Check if a paired device record exists (valid MessagePack data).
    pub async fn device_exists(&self) -> Result<bool> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| -> std::result::Result<bool, rusqlite::Error> {
                let mut stmt = conn.prepare("SELECT data FROM wa_device WHERE id = ?1")?;
                let result = stmt
                    .query_row(params![did], |row| row.get::<_, Vec<u8>>(0))
                    .optional()?;
                match result {
                    Some(data) => Ok(rmp_serde::from_slice::<Device>(&data).is_ok()),
                    None => Ok(false),
                }
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }
}

/// Extension trait for rusqlite optional queries
trait OptionalExt<T> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for std::result::Result<T, rusqlite::Error> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

// ─── SignalStore ───────────────────────────────────────────────────────────────

#[async_trait]
impl SignalStore for Store {
    async fn put_identity(&self, address: &str, key: [u8; 32]) -> Result<()> {
        let addr = address.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_identities (address, device_id, key) VALUES (?1, ?2, ?3)
                     ON CONFLICT(address, device_id) DO UPDATE SET key = excluded.key",
                    params![addr, did, key.as_slice()],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn load_identity(&self, address: &str) -> Result<Option<Vec<u8>>> {
        let addr = address.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare("SELECT key FROM wa_identities WHERE address = ?1 AND device_id = ?2")?
                    .query_row(params![addr, did], |row| row.get::<_, Vec<u8>>(0))
                    .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn delete_identity(&self, address: &str) -> Result<()> {
        let addr = address.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_identities WHERE address = ?1 AND device_id = ?2",
                    params![addr, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_session(&self, address: &str) -> Result<Option<Vec<u8>>> {
        let addr = address.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT record FROM wa_sessions WHERE address = ?1 AND device_id = ?2",
                )?
                .query_row(params![addr, did], |row| row.get::<_, Vec<u8>>(0))
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn put_session(&self, address: &str, session: &[u8]) -> Result<()> {
        let addr = address.to_string();
        let did = self.device_id;
        let data = session.to_vec();
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_sessions (address, device_id, record) VALUES (?1, ?2, ?3)
                     ON CONFLICT(address, device_id) DO UPDATE SET record = excluded.record",
                    params![addr, did, data],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn delete_session(&self, address: &str) -> Result<()> {
        let addr = address.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_sessions WHERE address = ?1 AND device_id = ?2",
                    params![addr, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn store_prekey(&self, id: u32, record: &[u8], uploaded: bool) -> Result<()> {
        let did = self.device_id;
        let data = record.to_vec();
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_prekeys (id, device_id, record, uploaded) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(id, device_id) DO UPDATE SET record = excluded.record, uploaded = excluded.uploaded",
                    params![id, did, data, uploaded],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn load_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare("SELECT record FROM wa_prekeys WHERE id = ?1 AND device_id = ?2")?
                    .query_row(params![id, did], |row| row.get::<_, Vec<u8>>(0))
                    .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn remove_prekey(&self, id: u32) -> Result<()> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_prekeys WHERE id = ?1 AND device_id = ?2",
                    params![id, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn store_signed_prekey(&self, id: u32, record: &[u8]) -> Result<()> {
        let did = self.device_id;
        let data = record.to_vec();
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_signed_prekeys (id, device_id, record) VALUES (?1, ?2, ?3)
                     ON CONFLICT(id, device_id) DO UPDATE SET record = excluded.record",
                    params![id, did, data],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn load_signed_prekey(&self, id: u32) -> Result<Option<Vec<u8>>> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT record FROM wa_signed_prekeys WHERE id = ?1 AND device_id = ?2",
                )?
                .query_row(params![id, did], |row| row.get::<_, Vec<u8>>(0))
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn load_all_signed_prekeys(&self) -> Result<Vec<(u32, Vec<u8>)>> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                let mut stmt =
                    conn.prepare("SELECT id, record FROM wa_signed_prekeys WHERE device_id = ?1")?;
                let rows = stmt.query_map(params![did], |row| {
                    Ok((row.get::<_, i64>(0)? as u32, row.get::<_, Vec<u8>>(1)?))
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn remove_signed_prekey(&self, id: u32) -> Result<()> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_signed_prekeys WHERE id = ?1 AND device_id = ?2",
                    params![id, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn put_sender_key(&self, address: &str, record: &[u8]) -> Result<()> {
        let addr = address.to_string();
        let did = self.device_id;
        let data = record.to_vec();
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_sender_keys (address, device_id, record) VALUES (?1, ?2, ?3)
                     ON CONFLICT(address, device_id) DO UPDATE SET record = excluded.record",
                    params![addr, did, data],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_sender_key(&self, address: &str) -> Result<Option<Vec<u8>>> {
        let addr = address.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT record FROM wa_sender_keys WHERE address = ?1 AND device_id = ?2",
                )?
                .query_row(params![addr, did], |row| row.get::<_, Vec<u8>>(0))
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn delete_sender_key(&self, address: &str) -> Result<()> {
        let addr = address.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_sender_keys WHERE address = ?1 AND device_id = ?2",
                    params![addr, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }
}

// ─── AppSyncStore ─────────────────────────────────────────────────────────────

#[async_trait]
impl AppSyncStore for Store {
    async fn get_sync_key(&self, key_id: &[u8]) -> Result<Option<AppStateSyncKey>> {
        let kid = key_id.to_vec();
        let did = self.device_id;
        let json_opt = self
            .pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT data FROM wa_app_state_keys WHERE key_id = ?1 AND device_id = ?2",
                )?
                .query_row(params![kid, did], |row| row.get::<_, String>(0))
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        match json_opt {
            Some(json) => {
                let key: AppStateSyncKey = serde_json::from_str(&json)
                    .map_err(|e| StoreError::Serialization(e.to_string()))?;
                Ok(Some(key))
            }
            None => Ok(None),
        }
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        let json =
            serde_json::to_string(&key).map_err(|e| StoreError::Serialization(e.to_string()))?;
        let kid = key_id.to_vec();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_app_state_keys (key_id, device_id, data) VALUES (?1, ?2, ?3)
                     ON CONFLICT(key_id, device_id) DO UPDATE SET data = excluded.data",
                    params![kid, did, json],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_version(&self, name: &str) -> Result<HashState> {
        let n = name.to_string();
        let did = self.device_id;
        let json_opt = self
            .pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT data FROM wa_app_state_versions WHERE name = ?1 AND device_id = ?2",
                )?
                .query_row(params![n, did], |row| row.get::<_, String>(0))
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        match json_opt {
            Some(json) => {
                let state: HashState = serde_json::from_str(&json)
                    .map_err(|e| StoreError::Serialization(e.to_string()))?;
                Ok(state)
            }
            None => Ok(HashState::default()),
        }
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        let json =
            serde_json::to_string(&state).map_err(|e| StoreError::Serialization(e.to_string()))?;
        let n = name.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_app_state_versions (name, device_id, data) VALUES (?1, ?2, ?3)
                     ON CONFLICT(name, device_id) DO UPDATE SET data = excluded.data",
                    params![n, did, json],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn put_mutation_macs(
        &self,
        name: &str,
        version: u64,
        mutations: &[AppStateMutationMAC],
    ) -> Result<()> {
        let n = name.to_string();
        let did = self.device_id;
        let muts: Vec<_> = mutations.to_vec();
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                for m in &muts {
                    conn.execute(
                        "INSERT INTO wa_app_state_mutation_macs (name, version, index_mac, value_mac, device_id)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![n, version as i64, m.index_mac, m.value_mac, did],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_mutation_mac(&self, name: &str, index_mac: &[u8]) -> Result<Option<Vec<u8>>> {
        let n = name.to_string();
        let im = index_mac.to_vec();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT value_mac FROM wa_app_state_mutation_macs
                     WHERE name = ?1 AND index_mac = ?2 AND device_id = ?3",
                )?
                .query_row(params![n, im, did], |row| row.get::<_, Vec<u8>>(0))
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn delete_mutation_macs(&self, name: &str, index_macs: &[Vec<u8>]) -> Result<()> {
        let n = name.to_string();
        let did = self.device_id;
        let macs: Vec<Vec<u8>> = index_macs.to_vec();
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                for mac in &macs {
                    conn.execute(
                        "DELETE FROM wa_app_state_mutation_macs
                         WHERE name = ?1 AND index_mac = ?2 AND device_id = ?3",
                        params![n, mac, did],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }
}

// ─── ProtocolStore ────────────────────────────────────────────────────────────

#[async_trait]
impl ProtocolStore for Store {
    async fn get_skdm_recipients(&self, group_jid: &str) -> Result<Vec<Jid>> {
        let gj = group_jid.to_string();
        let did = self.device_id;
        let strings = self
            .pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT device_jid FROM wa_skdm_recipients WHERE group_jid = ?1 AND device_id = ?2",
                )?;
                let rows = stmt.query_map(params![gj, did], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        let mut jids = Vec::with_capacity(strings.len());
        for s in strings {
            if let Ok(jid) = s.parse::<Jid>() {
                jids.push(jid);
            }
        }
        Ok(jids)
    }

    async fn add_skdm_recipients(&self, group_jid: &str, device_jids: &[Jid]) -> Result<()> {
        let gj = group_jid.to_string();
        let did = self.device_id;
        let jid_strings: Vec<String> = device_jids.iter().map(|j| j.to_string()).collect();
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                for jid_str in &jid_strings {
                    conn.execute(
                        "INSERT OR IGNORE INTO wa_skdm_recipients (group_jid, device_jid, device_id)
                         VALUES (?1, ?2, ?3)",
                        params![gj, jid_str, did],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn clear_skdm_recipients(&self, group_jid: &str) -> Result<()> {
        let gj = group_jid.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_skdm_recipients WHERE group_jid = ?1 AND device_id = ?2",
                    params![gj, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_tc_token(&self, jid: &str) -> Result<Option<TcTokenEntry>> {
        let j = jid.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT token, token_timestamp, sender_timestamp FROM wa_tc_tokens WHERE jid = ?1 AND device_id = ?2",
                )?
                .query_row(params![j, did], |row| {
                    Ok(TcTokenEntry {
                        token: row.get(0)?,
                        token_timestamp: row.get(1)?,
                        sender_timestamp: row.get(2)?,
                    })
                })
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn put_tc_token(&self, jid: &str, entry: &TcTokenEntry) -> Result<()> {
        let j = jid.to_string();
        let did = self.device_id;
        let token = entry.token.clone();
        let tt = entry.token_timestamp;
        let st = entry.sender_timestamp;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_tc_tokens (jid, token, token_timestamp, sender_timestamp, device_id)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(jid, device_id) DO UPDATE SET
                        token = excluded.token,
                        token_timestamp = excluded.token_timestamp,
                        sender_timestamp = excluded.sender_timestamp",
                    params![j, token, tt, st, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn delete_tc_token(&self, jid: &str) -> Result<()> {
        let j = jid.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_tc_tokens WHERE jid = ?1 AND device_id = ?2",
                    params![j, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_all_tc_token_jids(&self) -> Result<Vec<String>> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                let mut stmt = conn.prepare("SELECT jid FROM wa_tc_tokens WHERE device_id = ?1")?;
                let rows = stmt.query_map(params![did], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn delete_expired_tc_tokens(&self, cutoff_timestamp: i64) -> Result<u32> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_tc_tokens WHERE token_timestamp < ?1 AND device_id = ?2",
                    params![cutoff_timestamp, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map(|n| n as u32)
            .map_err(db_err)
    }

    async fn get_lid_mapping(&self, lid: &str) -> Result<Option<LidPnMappingEntry>> {
        let l = lid.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT lid, phone_number, created_at, updated_at, learning_source
                     FROM wa_lid_pn_mapping WHERE lid = ?1 AND device_id = ?2",
                )?
                .query_row(params![l, did], |row| {
                    Ok(LidPnMappingEntry {
                        lid: row.get(0)?,
                        phone_number: row.get(1)?,
                        created_at: row.get(2)?,
                        updated_at: row.get(3)?,
                        learning_source: row.get(4)?,
                    })
                })
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn get_pn_mapping(&self, phone: &str) -> Result<Option<LidPnMappingEntry>> {
        let p = phone.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT lid, phone_number, created_at, updated_at, learning_source
                     FROM wa_lid_pn_mapping WHERE phone_number = ?1 AND device_id = ?2",
                )?
                .query_row(params![p, did], |row| {
                    Ok(LidPnMappingEntry {
                        lid: row.get(0)?,
                        phone_number: row.get(1)?,
                        created_at: row.get(2)?,
                        updated_at: row.get(3)?,
                        learning_source: row.get(4)?,
                    })
                })
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn put_lid_mapping(&self, entry: &LidPnMappingEntry) -> Result<()> {
        let e = entry.clone();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_lid_pn_mapping (lid, phone_number, created_at, updated_at, learning_source, device_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(lid, device_id) DO UPDATE SET
                        phone_number = excluded.phone_number,
                        updated_at = excluded.updated_at,
                        learning_source = excluded.learning_source",
                    params![e.lid, e.phone_number, e.created_at, e.updated_at, e.learning_source, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_all_lid_mappings(&self) -> Result<Vec<LidPnMappingEntry>> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT lid, phone_number, created_at, updated_at, learning_source
                     FROM wa_lid_pn_mapping WHERE device_id = ?1",
                )?;
                let rows = stmt.query_map(params![did], |row| {
                    Ok(LidPnMappingEntry {
                        lid: row.get(0)?,
                        phone_number: row.get(1)?,
                        created_at: row.get(2)?,
                        updated_at: row.get(3)?,
                        learning_source: row.get(4)?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn save_base_key(&self, address: &str, message_id: &str, base_key: &[u8]) -> Result<()> {
        let addr = address.to_string();
        let mid = message_id.to_string();
        let bk = base_key.to_vec();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_base_keys (address, message_id, base_key, device_id) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(address, message_id, device_id) DO UPDATE SET base_key = excluded.base_key",
                    params![addr, mid, bk, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn has_same_base_key(
        &self,
        address: &str,
        message_id: &str,
        current_base_key: &[u8],
    ) -> Result<bool> {
        let addr = address.to_string();
        let mid = message_id.to_string();
        let cbk = current_base_key.to_vec();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                let stored = conn
                    .prepare(
                        "SELECT base_key FROM wa_base_keys
                         WHERE address = ?1 AND message_id = ?2 AND device_id = ?3",
                    )?
                    .query_row(params![addr, mid, did], |row| row.get::<_, Vec<u8>>(0))
                    .optional()?;
                Ok::<_, rusqlite::Error>(stored.is_some_and(|s| s == cbk))
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn delete_base_key(&self, address: &str, message_id: &str) -> Result<()> {
        let addr = address.to_string();
        let mid = message_id.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_base_keys WHERE address = ?1 AND message_id = ?2 AND device_id = ?3",
                    params![addr, mid, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn update_device_list(&self, record: DeviceListRecord) -> Result<()> {
        let json =
            serde_json::to_string(&record).map_err(|e| StoreError::Serialization(e.to_string()))?;
        let user = record.user.clone();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_device_registry (user, device_id, data) VALUES (?1, ?2, ?3)
                     ON CONFLICT(user, device_id) DO UPDATE SET data = excluded.data",
                    params![user, did, json],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_devices(&self, user: &str) -> Result<Option<DeviceListRecord>> {
        let u = user.to_string();
        let did = self.device_id;
        let json_opt = self
            .pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT data FROM wa_device_registry WHERE user = ?1 AND device_id = ?2",
                )?
                .query_row(params![u, did], |row| row.get::<_, String>(0))
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        match json_opt {
            Some(json) => {
                let record: DeviceListRecord = serde_json::from_str(&json)
                    .map_err(|e| StoreError::Serialization(e.to_string()))?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    async fn mark_forget_sender_key(&self, group_jid: &str, participant: &str) -> Result<()> {
        let gj = group_jid.to_string();
        let p = participant.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO wa_sender_key_forget (group_jid, participant, device_id)
                     VALUES (?1, ?2, ?3)",
                    params![gj, p, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn consume_forget_marks(&self, group_jid: &str) -> Result<Vec<String>> {
        let gj = group_jid.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT participant FROM wa_sender_key_forget
                     WHERE group_jid = ?1 AND device_id = ?2",
                )?;
                let rows = stmt.query_map(params![&gj, did], |row| row.get::<_, String>(0))?;
                let participants: Vec<String> = rows.collect::<std::result::Result<Vec<_>, _>>()?;

                if !participants.is_empty() {
                    conn.execute(
                        "DELETE FROM wa_sender_key_forget WHERE group_jid = ?1 AND device_id = ?2",
                        params![gj, did],
                    )?;
                }
                Ok::<_, rusqlite::Error>(participants)
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }
}

// ─── DeviceStore ──────────────────────────────────────────────────────────────

#[async_trait]
impl DeviceStore for Store {
    async fn save(&self, device: &Device) -> Result<()> {
        let bytes =
            rmp_serde::to_vec(device).map_err(|e| StoreError::Serialization(e.to_string()))?;
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_device (id, data) VALUES (?1, ?2)
                     ON CONFLICT(id) DO UPDATE SET data = excluded.data",
                    params![did, bytes],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn load(&self) -> Result<Option<Device>> {
        let did = self.device_id;
        let data_opt = self
            .pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare("SELECT data FROM wa_device WHERE id = ?1")?
                    .query_row(params![did], |row| row.get::<_, Vec<u8>>(0))
                    .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        match data_opt {
            Some(data) => {
                let device: Device = match rmp_serde::from_slice(&data) {
                    Ok(d) => d,
                    Err(_) => {
                        // Old JSON-serialized data can't roundtrip (byte array issue).
                        // Delete it so the client re-pairs cleanly.
                        tracing::warn!(
                            "WhatsApp: clearing incompatible legacy device data — re-pair required"
                        );
                        let did2 = self.device_id;
                        let _ = self.pool.get().await.ok().map(|conn| {
                            tokio::spawn(async move {
                                let _ = conn
                                    .interact(move |conn| {
                                        conn.execute(
                                            "DELETE FROM wa_device WHERE id = ?1",
                                            params![did2],
                                        )
                                    })
                                    .await;
                            });
                        });
                        return Ok(None);
                    }
                };
                Ok(Some(device))
            }
            None => Ok(None),
        }
    }

    async fn exists(&self) -> Result<bool> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare("SELECT 1 FROM wa_device WHERE id = ?1")?
                    .query_row(params![did], |_| Ok(()))
                    .optional()
                    .map(|opt| opt.is_some())
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn create(&self) -> Result<i32> {
        Ok(self.device_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> Store {
        Store::new(":memory:").await.unwrap()
    }

    #[tokio::test]
    async fn test_identity_roundtrip() {
        let store = test_store().await;
        let key = [42u8; 32];
        store
            .put_identity("alice@s.whatsapp.net", key)
            .await
            .unwrap();

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
        store
            .put_sender_key("group::sender", b"sk-data")
            .await
            .unwrap();
        let loaded = store
            .get_sender_key("group::sender")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded, b"sk-data");
        store.delete_sender_key("group::sender").await.unwrap();
        assert!(
            store
                .get_sender_key("group::sender")
                .await
                .unwrap()
                .is_none()
        );
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
            .add_skdm_recipients(
                "group1",
                &[
                    "user1@s.whatsapp.net".parse().unwrap(),
                    "user2@s.whatsapp.net".parse().unwrap(),
                ],
            )
            .await
            .unwrap();
        let recipients = store.get_skdm_recipients("group1").await.unwrap();
        assert_eq!(recipients.len(), 2);
        store.clear_skdm_recipients("group1").await.unwrap();
        assert!(
            store
                .get_skdm_recipients("group1")
                .await
                .unwrap()
                .is_empty()
        );
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
        assert!(
            store
                .has_same_base_key("addr", "msg1", b"key1")
                .await
                .unwrap()
        );
        assert!(
            !store
                .has_same_base_key("addr", "msg1", b"key2")
                .await
                .unwrap()
        );
        assert!(
            !store
                .has_same_base_key("addr", "msg2", b"key1")
                .await
                .unwrap()
        );
        store.delete_base_key("addr", "msg1").await.unwrap();
        assert!(
            !store
                .has_same_base_key("addr", "msg1", b"key1")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_sender_key_forget_marks() {
        let store = test_store().await;
        store
            .mark_forget_sender_key("group1", "user1")
            .await
            .unwrap();
        store
            .mark_forget_sender_key("group1", "user2")
            .await
            .unwrap();

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
        store
            .put_mutation_macs("critical_block", 1, &macs)
            .await
            .unwrap();

        let v = store
            .get_mutation_mac("critical_block", &[1, 2])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(v, vec![3, 4]);

        store
            .delete_mutation_macs("critical_block", &[vec![1, 2]])
            .await
            .unwrap();
        assert!(
            store
                .get_mutation_mac("critical_block", &[1, 2])
                .await
                .unwrap()
                .is_none()
        );
        // Second one still there
        assert!(
            store
                .get_mutation_mac("critical_block", &[5, 6])
                .await
                .unwrap()
                .is_some()
        );
    }
}
