//! Generic, schema-agnostic persistence for Velos objects.
//!
//! Objects are stored as opaque JSON documents plus index columns
//! (`name`, `uid`, `resource_version`, `node_name`, `labels`). The store knows
//! nothing about specific resource kinds. (Principle #7: storage != protocol.)

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::Connection;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("stored uid is not a valid uuid: {0}")]
    Uid(String),
    #[error("store lock poisoned")]
    Lock,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredObject {
    pub kind: String,
    pub name: String,
    pub uid: Uuid,
    pub resource_version: u64,
    pub node_name: Option<String>,
    pub labels: HashMap<String, String>,
    pub document: Value,
}

#[derive(Debug, Clone, Default)]
pub struct Selector {
    /// Equality label matches (all must hold).
    pub labels: Vec<(String, String)>,
    /// Field selector on `spec.nodeName`.
    pub node_name: Option<String>,
}

pub trait Store: Send + Sync {
    fn next_resource_version(&self) -> Result<u64, StoreError>;
    fn put(&self, obj: &StoredObject) -> Result<(), StoreError>;
    fn get(&self, kind: &str, name: &str) -> Result<Option<StoredObject>, StoreError>;
    fn list(&self, kind: &str, selector: &Selector) -> Result<Vec<StoredObject>, StoreError>;
    fn delete(&self, kind: &str, name: &str) -> Result<bool, StoreError>;
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS objects (
                kind             TEXT NOT NULL,
                name             TEXT NOT NULL,
                uid              TEXT NOT NULL,
                resource_version INTEGER NOT NULL,
                node_name        TEXT,
                labels           TEXT NOT NULL,
                document         TEXT NOT NULL,
                PRIMARY KEY (kind, name)
            );
            CREATE TABLE IF NOT EXISTS rv_seq (
                id    INTEGER PRIMARY KEY CHECK (id = 0),
                value INTEGER NOT NULL
            );
            INSERT OR IGNORE INTO rv_seq (id, value) VALUES (0, 0);",
        )?;
        Ok(())
    }

    fn parse_uid(s: &str) -> Result<Uuid, StoreError> {
        Uuid::parse_str(s).map_err(|_| StoreError::Uid(s.to_string()))
    }
}

impl Store for SqliteStore {
    fn next_resource_version(&self) -> Result<u64, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        conn.execute("UPDATE rv_seq SET value = value + 1 WHERE id = 0", [])?;
        let v: i64 = conn.query_row("SELECT value FROM rv_seq WHERE id = 0", [], |r| r.get(0))?;
        Ok(v as u64)
    }

    fn put(&self, obj: &StoredObject) -> Result<(), StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        let labels = serde_json::to_string(&obj.labels)?;
        let document = serde_json::to_string(&obj.document)?;
        conn.execute(
            "INSERT INTO objects (kind, name, uid, resource_version, node_name, labels, document)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(kind, name) DO UPDATE SET
                uid              = excluded.uid,
                resource_version = excluded.resource_version,
                node_name        = excluded.node_name,
                labels           = excluded.labels,
                document         = excluded.document",
            rusqlite::params![
                obj.kind,
                obj.name,
                obj.uid.to_string(),
                obj.resource_version as i64,
                obj.node_name,
                labels,
                document,
            ],
        )?;
        Ok(())
    }

    fn get(&self, kind: &str, name: &str) -> Result<Option<StoredObject>, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        let mut stmt = conn.prepare(
            "SELECT uid, resource_version, node_name, labels, document
             FROM objects WHERE kind = ?1 AND name = ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![kind, name])?;
        match rows.next()? {
            Some(row) => {
                let uid_s: String = row.get(0)?;
                let rv: i64 = row.get(1)?;
                let node_name: Option<String> = row.get(2)?;
                let labels_s: String = row.get(3)?;
                let document_s: String = row.get(4)?;
                Ok(Some(StoredObject {
                    kind: kind.to_string(),
                    name: name.to_string(),
                    uid: Self::parse_uid(&uid_s)?,
                    resource_version: rv as u64,
                    node_name,
                    labels: serde_json::from_str(&labels_s)?,
                    document: serde_json::from_str(&document_s)?,
                }))
            }
            None => Ok(None),
        }
    }

    fn list(&self, kind: &str, selector: &Selector) -> Result<Vec<StoredObject>, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        let mut stmt = conn.prepare(
            "SELECT name, uid, resource_version, node_name, labels, document
             FROM objects WHERE kind = ?1 ORDER BY name",
        )?;
        let raw = stmt.query_map(rusqlite::params![kind], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;

        let mut out = Vec::new();
        for r in raw {
            let (name, uid_s, rv, node_name, labels_s, document_s) = r?;
            if let Some(want) = &selector.node_name
                && node_name.as_deref() != Some(want.as_str())
            {
                continue;
            }
            let labels: HashMap<String, String> = serde_json::from_str(&labels_s)?;
            let matches = selector
                .labels
                .iter()
                .all(|(k, v)| labels.get(k).map(|x| x == v).unwrap_or(false));
            if !matches {
                continue;
            }
            out.push(StoredObject {
                kind: kind.to_string(),
                name,
                uid: Self::parse_uid(&uid_s)?,
                resource_version: rv as u64,
                node_name,
                labels,
                document: serde_json::from_str(&document_s)?,
            });
        }
        Ok(out)
    }

    fn delete(&self, kind: &str, name: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        let n = conn.execute(
            "DELETE FROM objects WHERE kind = ?1 AND name = ?2",
            rusqlite::params![kind, name],
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn obj(kind: &str, name: &str, rv: u64) -> StoredObject {
        StoredObject {
            kind: kind.to_string(),
            name: name.to_string(),
            uid: Uuid::new_v4(),
            resource_version: rv,
            node_name: None,
            labels: HashMap::new(),
            document: serde_json::json!({ "metadata": { "name": name } }),
        }
    }

    #[test]
    fn resource_version_is_monotonic() {
        let s = SqliteStore::in_memory().unwrap();
        assert_eq!(s.next_resource_version().unwrap(), 1);
        assert_eq!(s.next_resource_version().unwrap(), 2);
        assert_eq!(s.next_resource_version().unwrap(), 3);
    }

    #[test]
    fn put_then_get_round_trips() {
        let s = SqliteStore::in_memory().unwrap();
        let o = obj("Container", "c1", 7);
        s.put(&o).unwrap();

        let got = s.get("Container", "c1").unwrap().unwrap();
        assert_eq!(got, o);
        assert!(s.get("Container", "missing").unwrap().is_none());
    }

    #[test]
    fn put_upserts_on_same_kind_and_name() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj("Container", "c1", 1)).unwrap();
        let mut updated = obj("Container", "c1", 2);
        updated.document = serde_json::json!({ "metadata": { "name": "c1" }, "v": 2 });
        s.put(&updated).unwrap();

        let got = s.get("Container", "c1").unwrap().unwrap();
        assert_eq!(got.resource_version, 2);
        assert_eq!(got.document["v"], 2);
    }

    fn obj_with(
        kind: &str,
        name: &str,
        node: Option<&str>,
        labels: &[(&str, &str)],
    ) -> StoredObject {
        StoredObject {
            kind: kind.to_string(),
            name: name.to_string(),
            uid: Uuid::new_v4(),
            resource_version: 1,
            node_name: node.map(|s| s.to_string()),
            labels: labels
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            document: serde_json::json!({ "metadata": { "name": name } }),
        }
    }

    #[test]
    fn list_filters_by_kind() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj_with("Container", "c1", None, &[])).unwrap();
        s.put(&obj_with("Container", "c2", None, &[])).unwrap();
        s.put(&obj_with("Worker", "w1", None, &[])).unwrap();

        let containers = s.list("Container", &Selector::default()).unwrap();
        assert_eq!(containers.len(), 2);
        let workers = s.list("Worker", &Selector::default()).unwrap();
        assert_eq!(workers.len(), 1);
    }

    #[test]
    fn list_filters_by_label_equality() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj_with("Container", "c1", None, &[("team", "a")]))
            .unwrap();
        s.put(&obj_with("Container", "c2", None, &[("team", "b")]))
            .unwrap();

        let sel = Selector {
            labels: vec![("team".into(), "a".into())],
            node_name: None,
        };
        let got = s.list("Container", &sel).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "c1");
    }

    #[test]
    fn list_filters_by_node_name() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj_with("Container", "c1", Some("node-7"), &[]))
            .unwrap();
        s.put(&obj_with("Container", "c2", Some("node-8"), &[]))
            .unwrap();
        s.put(&obj_with("Container", "c3", None, &[])).unwrap();

        let sel = Selector {
            labels: vec![],
            node_name: Some("node-7".into()),
        };
        let got = s.list("Container", &sel).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "c1");
    }

    #[test]
    fn delete_removes_and_reports() {
        let s = SqliteStore::in_memory().unwrap();
        s.put(&obj_with("Container", "c1", None, &[])).unwrap();
        assert!(s.delete("Container", "c1").unwrap());
        assert!(!s.delete("Container", "c1").unwrap());
        assert!(s.get("Container", "c1").unwrap().is_none());
    }
}
