//! Rusqlite-backed WhatsApp session store
//!
//! Implements `wacore::store::Backend` using deadpool-sqlite + rusqlite,
//! matching the rest of the OpenCrabs database layer.
//!
//! Layout: [`pool`] (the `Store` handle and its connection pool),
//! [`schema`] (the `wa_*` DDL), [`errors`] (error mapping), and one module
//! per `wacore` backend trait (`appsync`, `device`, `msgsecret`,
//! `protocol`, `signal`). This file is declarations only — no function
//! definitions live here (CONTRIBUTING.md).

mod appsync;
mod device;
mod errors;
mod msgsecret;
mod pool;
mod protocol;
mod schema;
mod signal;

pub use pool::Store;
