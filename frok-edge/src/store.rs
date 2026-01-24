use anyhow::{Context, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use frok_common::paths::data_path;
use redb::{Database, Durability, TableDefinition};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum AuthKind {
    Oidc,
    Key,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct IdentityRecord {
    pub subject: String,
    pub auth_mode: AuthKind,
    pub agent_label: String,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub public_key_fingerprint: Option<String>,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct RouteOwnership {
    pub hostname: String,
    pub subject: String,
    pub registered_at: i64,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct KeyTrustRecord {
    pub public_key_fingerprint: String,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub agent_label: String,
}

const IDENTITY_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("identity_registry");
const ROUTE_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("route_ownership");
const KEY_TRUST_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("key_trust");

pub struct EdgeStore {
    db: Database,
}

impl EdgeStore {
    pub fn open() -> Result<Arc<Self>> {
        let path = default_store_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create store dir {}", parent.display()))?;
        }

        let db = if path.exists() {
            open_database(&path).with_context(|| format!("open store {}", path.display()))?
        } else {
            create_database(&path).with_context(|| format!("create store {}", path.display()))?
        };

        let store = Arc::new(Self { db });
        store.ensure_tables()?;
        Ok(store)
    }

    pub fn load_identity(&self, subject: &str) -> Result<Option<IdentityRecord>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(IDENTITY_TABLE)?;
        Ok(table
            .get(subject)?
            .map(|value| deserialize(&value.value()))
            .transpose()?)
    }

    pub fn upsert_identity(&self, record: &IdentityRecord) -> Result<()> {
        let mut write = self.db.begin_write()?;
        write.set_durability(Durability::Immediate);
        write.set_two_phase_commit(true);
        {
            let mut table = write.open_table(IDENTITY_TABLE)?;
            table.insert(record.subject.as_str(), serialize(record)?)?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn load_route(&self, hostname: &str) -> Result<Option<RouteOwnership>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(ROUTE_TABLE)?;
        Ok(table
            .get(hostname)?
            .map(|value| deserialize(&value.value()))
            .transpose()?)
    }

    pub fn insert_route(&self, record: &RouteOwnership) -> Result<()> {
        let mut write = self.db.begin_write()?;
        write.set_durability(Durability::Immediate);
        write.set_two_phase_commit(true);
        {
            let mut table = write.open_table(ROUTE_TABLE)?;
            table.insert(record.hostname.as_str(), serialize(record)?)?;
        }
        write.commit()?;
        Ok(())
    }

    pub fn load_key_trust(&self, fingerprint: &str) -> Result<Option<KeyTrustRecord>> {
        let read = self.db.begin_read()?;
        let table = read.open_table(KEY_TRUST_TABLE)?;
        Ok(table
            .get(fingerprint)?
            .map(|value| deserialize(&value.value()))
            .transpose()?)
    }

    pub fn upsert_key_trust(&self, record: &KeyTrustRecord) -> Result<()> {
        let mut write = self.db.begin_write()?;
        write.set_durability(Durability::Immediate);
        write.set_two_phase_commit(true);
        {
            let mut table = write.open_table(KEY_TRUST_TABLE)?;
            table.insert(record.public_key_fingerprint.as_str(), serialize(record)?)?;
        }
        write.commit()?;
        Ok(())
    }

    fn ensure_tables(&self) -> Result<()> {
        let mut write = self.db.begin_write()?;
        write.set_durability(Durability::Immediate);
        write.set_two_phase_commit(true);
        {
            let _ = write.open_table(IDENTITY_TABLE)?;
            let _ = write.open_table(ROUTE_TABLE)?;
            let _ = write.open_table(KEY_TRUST_TABLE)?;
        }
        write.commit()?;
        Ok(())
    }
}

fn serialize<T: BorshSerialize>(value: &T) -> Result<Vec<u8>> {
    borsh::to_vec(value).context("serialize record")
}

fn deserialize<T: BorshDeserialize>(bytes: &[u8]) -> Result<T> {
    T::try_from_slice(bytes).context("deserialize record")
}

fn default_store_path() -> PathBuf {
    data_path("edge.db")
}

fn open_database(path: &PathBuf) -> Result<Database> {
    let repaired = Arc::new(AtomicBool::new(false));
    let repaired_flag = repaired.clone();
    let mut builder = Database::builder();
    builder.set_repair_callback(move |session| {
        repaired_flag.store(true, Ordering::Release);
        tracing::warn!(progress = session.progress(), "repairing edge store");
    });
    let db = builder.open(path).map_err(anyhow::Error::from)?;
    if repaired.load(Ordering::Acquire) {
        tracing::info!("edge store repaired");
    }
    Ok(db)
}

fn create_database(path: &PathBuf) -> Result<Database> {
    let repaired = Arc::new(AtomicBool::new(false));
    let repaired_flag = repaired.clone();
    let mut builder = Database::builder();
    builder.set_repair_callback(move |session| {
        repaired_flag.store(true, Ordering::Release);
        tracing::warn!(progress = session.progress(), "repairing edge store");
    });
    let db = builder.create(path).map_err(anyhow::Error::from)?;
    if repaired.load(Ordering::Acquire) {
        tracing::info!("edge store repaired");
    }
    Ok(db)
}
