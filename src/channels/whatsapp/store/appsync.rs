use super::Store;
use super::errors::{OptionalExt, db_err, interact_to_store_err, pool_err};
use async_trait::async_trait;
use rusqlite::params;
use wacore::appstate::hash::HashState;
use wacore::appstate::processor::AppStateMutationMAC;
use wacore::store::error::{Result, StoreError};
use wacore::store::traits::{AppStateSyncKey, AppSyncStore};

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
                    .map_err(|e| StoreError::Serialization(Box::new(e)))?;
                Ok(Some(key))
            }
            None => Ok(None),
        }
    }

    async fn set_sync_key(&self, key_id: &[u8], key: AppStateSyncKey) -> Result<()> {
        let json =
            serde_json::to_string(&key).map_err(|e| StoreError::Serialization(Box::new(e)))?;
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
                    .map_err(|e| StoreError::Serialization(Box::new(e)))?;
                Ok(state)
            }
            None => Ok(HashState::default()),
        }
    }

    async fn set_version(&self, name: &str, state: HashState) -> Result<()> {
        let json =
            serde_json::to_string(&state).map_err(|e| StoreError::Serialization(Box::new(e)))?;
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

    async fn clear_mutation_macs(&self, name: &str) -> Result<()> {
        // Snapshot re-sync rebuilds the MAC store from the snapshot; leftover
        // entries for this collection would corrupt the next patch's ltHash.
        let n = name.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_app_state_mutation_macs WHERE name = ?1 AND device_id = ?2",
                    params![n, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn get_latest_sync_key_id(&self) -> Result<Option<Vec<u8>>> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare(
                    "SELECT key_id FROM wa_app_state_keys WHERE device_id = ?1
                     ORDER BY rowid DESC LIMIT 1",
                )?
                .query_row(params![did], |row| row.get::<_, Vec<u8>>(0))
                .optional()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }
}
