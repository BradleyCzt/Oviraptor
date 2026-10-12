//! Lossless SQLite values, column order and original physical rowids.
use rusqlite::{types::Value, Connection};
use serde::{Deserialize, Serialize};
#[path = "restore.rs"]
mod restore;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Table {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<RowIdentity>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(super) enum RowIdentity {
    Rowid,
    PrimaryKey { columns: Vec<String> },
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub(super) enum Cell {
    Null,
    Integer(i64),
    Real(u64),
    Text(String),
    Blob(Vec<u8>),
}
impl Cell {
    fn from(v: Value) -> Self {
        match v {
            Value::Null => Self::Null,
            Value::Integer(v) => Self::Integer(v),
            Value::Real(v) => Self::Real(v.to_bits()),
            Value::Text(v) => Self::Text(v),
            Value::Blob(v) => Self::Blob(v),
        }
    }
    fn literal(&self) -> Result<String, String> {
        fn hex(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }
        match self {
            Self::Integer(v) => Ok(v.to_string()),
            Self::Text(v) => Ok(format!("CAST(X'{}' AS TEXT)", hex(v.as_bytes()))),
            Self::Blob(v) => Ok(format!("X'{}'", hex(v))),
            Self::Real(v) if f64::from_bits(*v).is_finite() => {
                Ok(format!("{:?}", f64::from_bits(*v)))
            }
            _ => Err("deleted_audit_primary_key_null_or_invalid".into()),
        }
    }
    fn value(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Integer(v) => Value::Integer(*v),
            Self::Real(v) => Value::Real(f64::from_bits(*v)),
            Self::Text(v) => Value::Text(v.clone()),
            Self::Blob(v) => Value::Blob(v.clone()),
        }
    }
}
pub(super) fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
impl Table {
    pub fn read(db: &Connection, name: &str, filter: &str, scan: &str) -> Result<Self, String> {
        let (identity, columns, order) = Self::original_layout(db, name)?;
        let projection = if identity.is_none() { "rowid,*" } else { "*" };
        let mut q = db
            .prepare(&format!(
                "SELECT {projection} FROM {} WHERE {filter} ORDER BY {order}",
                quote(name)
            ))
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([scan], |r| {
                (0..columns.len())
                    .map(|i| r.get::<_, Value>(i).map(Cell::from))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        let table = Self {
            name: name.into(),
            columns,
            rows,
            identity,
        };
        table.validate_identity()?;
        Ok(table)
    }
    pub fn original_layout(
        db: &Connection,
        name: &str,
    ) -> Result<(Option<RowIdentity>, Vec<String>, String), String> {
        let wr: bool = db
            .query_row(
                "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut columns = db
            .prepare(&format!(
                "SELECT {} FROM {} LIMIT 0",
                if wr { "*" } else { "rowid,*" },
                quote(name)
            ))
            .map_err(|e| e.to_string())?
            .column_names()
            .iter()
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>();
        if !wr {
            columns[0] = "rowid".into();
            return Ok((None, columns, "rowid".into()));
        }
        let keys = db
            .prepare("SELECT name FROM pragma_table_info(?1,'main') WHERE pk>0 ORDER BY pk")
            .map_err(|e| e.to_string())?
            .query_map([name], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        if keys.is_empty() {
            return Err("deleted_audit_primary_key_missing".into());
        }
        let order = keys.iter().map(|c| quote(c)).collect::<Vec<_>>().join(",");
        Ok((
            Some(RowIdentity::PrimaryKey { columns: keys }),
            columns,
            order,
        ))
    }
    pub fn effective_identity(&self) -> RowIdentity {
        self.identity.clone().unwrap_or(RowIdentity::Rowid)
    }
    pub fn validate_identity(&self) -> Result<(), String> {
        let mut names = std::collections::BTreeSet::new();
        if self.columns.is_empty()
            || self
                .columns
                .iter()
                .any(|c| c.is_empty() || !names.insert(c.to_ascii_lowercase()))
        {
            return Err("deleted_audit_columns_invalid".into());
        }
        let indices = match self.effective_identity() {
            RowIdentity::Rowid => {
                if self.columns.len() < 2 || self.columns[0] != "rowid" {
                    return Err("deleted_audit_columns_invalid".into());
                }
                vec![0]
            }
            RowIdentity::PrimaryKey { columns } => {
                let mut unique = std::collections::BTreeSet::new();
                if columns.is_empty() {
                    return Err("deleted_audit_primary_key_missing".into());
                }
                columns
                    .iter()
                    .map(|key| {
                        if !unique.insert(key) {
                            return Err("deleted_audit_primary_key_invalid".into());
                        }
                        self.columns
                            .iter()
                            .position(|c| c == key)
                            .ok_or_else(|| "deleted_audit_primary_key_invalid".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        let mut keys = std::collections::BTreeSet::new();
        for row in &self.rows {
            if row.len() != self.columns.len() {
                return Err("deleted_audit_row_invalid".into());
            }
            if matches!(self.effective_identity(), RowIdentity::Rowid)
                && !matches!(row[0], Cell::Integer(v) if v>0)
            {
                return Err("deleted_audit_row_invalid".into());
            }
            let key = indices
                .iter()
                .map(|i| row[*i].literal())
                .collect::<Result<Vec<_>, _>>()?;
            if !keys.insert(key) {
                return Err("deleted_audit_primary_key_duplicate".into());
            }
        }
        Ok(())
    }
    fn key_predicate(&self) -> Result<String, String> {
        self.validate_identity()?;
        let keys = match self.effective_identity() {
            RowIdentity::Rowid => vec!["rowid".to_string()],
            RowIdentity::PrimaryKey { columns } => columns,
        };
        let indices = keys
            .iter()
            .map(|k| {
                self.columns
                    .iter()
                    .position(|c| c == k)
                    .ok_or("deleted_audit_primary_key_invalid")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let clauses = self
            .rows
            .iter()
            .map(|row| {
                keys.iter()
                    .zip(&indices)
                    .map(|(key, index)| Ok(format!("{}={}", quote(key), row[*index].literal()?)))
                    .collect::<Result<Vec<_>, String>>()
                    .map(|parts| format!("({})", parts.join(" AND ")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(if clauses.is_empty() {
            "0".into()
        } else {
            clauses.join(" OR ")
        })
    }
    pub fn original_filter(&self) -> Result<String, String> {
        Ok(format!("({}) AND ?1 IS NOT NULL", self.key_predicate()?))
    }
    pub fn outside_filter(&self) -> Result<String, String> {
        if self.name == "sentinel_deleted_scans" {
            return Ok("scan_id<>?1".into());
        }
        Ok(format!(
            "NOT ({}) AND ?1 IS NOT NULL",
            self.key_predicate()?
        ))
    }
    pub fn canonical(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}
