// The `Backend` trait returns `Bytes` for sessions/prekeys. `bytes` is
// not a direct dependency, so we name the identical (cargo-unified) type via
// axum's public re-export.
use super::Store;
use super::errors::{OptionalExt, db_err, interact_to_store_err, pool_err};
use async_trait::async_trait;
use axum::body::Bytes;
use rusqlite::params;
use wacore::store::error::{Result, StoreError};
use wacore::store::traits::SignalStore;

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

    async fn load_identity(&self, address: &str) -> Result<Option<[u8; 32]>> {
        let addr = address.to_string();
        let did = self.device_id;
        let bytes_opt = self
            .pool
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
            .map_err(db_err)?;
        match bytes_opt {
            Some(v) => {
                let len = v.len();
                let arr: [u8; 32] = v.try_into().map_err(|_| {
                    StoreError::Validation(format!("identity key length {len} != 32"))
                })?;
                Ok(Some(arr))
            }
            None => Ok(None),
        }
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

    async fn get_session(&self, address: &str) -> Result<Option<Bytes>> {
        let addr = address.to_string();
        let did = self.device_id;
        let vec_opt = self
            .pool
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
            .map_err(db_err)?;
        Ok(vec_opt.map(Bytes::from))
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

    async fn mark_prekeys_uploaded(&self, ids: &[u32]) -> Result<()> {
        // Flip the uploaded flag for keys still present. UPDATE (not upsert) so a
        // key consumed/deleted between the upload snapshot and this call stays
        // deleted and is never resurrected.
        let did = self.device_id;
        let ids: Vec<u32> = ids.to_vec();
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                for id in &ids {
                    conn.execute(
                        "UPDATE wa_prekeys SET uploaded = 1 WHERE id = ?1 AND device_id = ?2",
                        params![id, did],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn load_prekey(&self, id: u32) -> Result<Option<Bytes>> {
        let did = self.device_id;
        let vec_opt = self
            .pool
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
            .map_err(db_err)?;
        Ok(vec_opt.map(Bytes::from))
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

    async fn get_max_prekey_id(&self) -> Result<u32> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.prepare("SELECT COALESCE(MAX(id), 0) FROM wa_prekeys WHERE device_id = ?1")?
                    .query_row(params![did], |row| row.get::<_, i64>(0))
            })
            .await
            .map_err(interact_to_store_err)?
            .map(|v| v as u32)
            .map_err(db_err)
    }
}
