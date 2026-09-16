use super::Store;
use super::errors::{OptionalExt, db_err, interact_to_store_err, pool_err};
use async_trait::async_trait;
use rusqlite::params;
use wacore::store::error::{Result, StoreError};
use wacore::store::traits::{DeviceListRecord, LidPnMappingEntry, ProtocolStore, TcTokenEntry};

#[async_trait]
impl ProtocolStore for Store {
    // --- Per-Device Sender Key Tracking ---

    async fn get_sender_key_devices(&self, group_jid: &str) -> Result<Vec<(String, bool)>> {
        let gj = group_jid.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT device_jid, has_key FROM wa_sender_key_devices
                     WHERE group_jid = ?1 AND device_id = ?2",
                )?;
                let rows = stmt.query_map(params![gj, did], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0))
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn set_sender_key_status(&self, group_jid: &str, entries: &[(&str, bool)]) -> Result<()> {
        let gj = group_jid.to_string();
        let did = self.device_id;
        let owned: Vec<(String, bool)> = entries
            .iter()
            .map(|(jid, has_key)| (jid.to_string(), *has_key))
            .collect();
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                for (device_jid, has_key) in &owned {
                    conn.execute(
                        "INSERT INTO wa_sender_key_devices (group_jid, device_jid, has_key, device_id)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(group_jid, device_jid, device_id)
                         DO UPDATE SET has_key = excluded.has_key",
                        params![gj, device_jid, *has_key as i64, did],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn clear_sender_key_devices(&self, group_jid: &str) -> Result<()> {
        let gj = group_jid.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_sender_key_devices WHERE group_jid = ?1 AND device_id = ?2",
                    params![gj, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn clear_all_sender_key_devices(&self) -> Result<()> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_sender_key_devices WHERE device_id = ?1",
                    params![did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn delete_sender_key_device_rows(&self, device_jids: &[&str]) -> Result<()> {
        if device_jids.is_empty() {
            return Ok(());
        }
        let did = self.device_id;
        let owned: Vec<String> = device_jids.iter().map(|j| j.to_string()).collect();
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                let placeholders = std::iter::repeat_n("?", owned.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "DELETE FROM wa_sender_key_devices
                     WHERE device_id = ? AND device_jid IN ({placeholders})"
                );
                let mut stmt = conn.prepare(&sql)?;
                stmt.raw_bind_parameter(1, did)?;
                for (i, jid) in owned.iter().enumerate() {
                    stmt.raw_bind_parameter(i + 2, jid)?;
                }
                stmt.raw_execute()?;
                Ok::<_, rusqlite::Error>(())
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

    /// A row goes only when BOTH windows are stale: the received token
    /// (`token_timestamp < token_cutoff`, or no token bytes at all) and the
    /// sender bucket (`sender_timestamp < sender_cutoff`, or never set). A
    /// recent sender bucket keeps the row even after the received token expires.
    async fn delete_expired_tc_tokens(&self, token_cutoff: i64, sender_cutoff: i64) -> Result<u32> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_tc_tokens \
                     WHERE device_id = ?3 \
                       AND (token_timestamp < ?1 OR token IS NULL OR length(token) = 0) \
                       AND (sender_timestamp IS NULL OR sender_timestamp < ?2)",
                    params![token_cutoff, sender_cutoff, did],
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
            serde_json::to_string(&record).map_err(|e| StoreError::Serialization(Box::new(e)))?;
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
                    .map_err(|e| StoreError::Serialization(Box::new(e)))?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    async fn delete_devices(&self, user: &str) -> Result<()> {
        let u = user.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_device_registry WHERE user = ?1 AND device_id = ?2",
                    params![u, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn store_sent_message(
        &self,
        chat_jid: &str,
        message_id: &str,
        payload: &[u8],
    ) -> Result<()> {
        let cj = chat_jid.to_string();
        let mid = message_id.to_string();
        let data = payload.to_vec();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO wa_sent_messages (chat_jid, message_id, payload, device_id)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(chat_jid, message_id, device_id) DO UPDATE SET payload = excluded.payload",
                    params![cj, mid, data, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)?;
        Ok(())
    }

    async fn take_sent_message(&self, chat_jid: &str, message_id: &str) -> Result<Option<Vec<u8>>> {
        let cj = chat_jid.to_string();
        let mid = message_id.to_string();
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                let payload = conn
                    .prepare(
                        "SELECT payload FROM wa_sent_messages
                         WHERE chat_jid = ?1 AND message_id = ?2 AND device_id = ?3",
                    )?
                    .query_row(params![&cj, &mid, did], |row| row.get::<_, Vec<u8>>(0))
                    .optional()?;
                if payload.is_some() {
                    conn.execute(
                        "DELETE FROM wa_sent_messages
                         WHERE chat_jid = ?1 AND message_id = ?2 AND device_id = ?3",
                        params![cj, mid, did],
                    )?;
                }
                Ok::<_, rusqlite::Error>(payload)
            })
            .await
            .map_err(interact_to_store_err)?
            .map_err(db_err)
    }

    async fn delete_expired_sent_messages(&self, cutoff_timestamp: i64) -> Result<u32> {
        let did = self.device_id;
        self.pool
            .get()
            .await
            .map_err(pool_err)?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM wa_sent_messages WHERE created_at < ?1 AND device_id = ?2",
                    params![cutoff_timestamp, did],
                )
            })
            .await
            .map_err(interact_to_store_err)?
            .map(|n| n as u32)
            .map_err(db_err)
    }
}
