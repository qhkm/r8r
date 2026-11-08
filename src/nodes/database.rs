/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Database node - execute SQL queries.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::debug;

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Database node for SQL query execution.
pub struct DatabaseNode;

impl DatabaseNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DatabaseNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DatabaseConfig {
    /// Database type: "sqlite", "postgres", "mysql"
    #[serde(default = "default_db_type")]
    db_type: String,

    /// Connection string or DSN
    #[serde(default)]
    connection_string: Option<String>,

    /// SQL query to execute
    query: String,

    /// Query parameters (positional or named)
    #[serde(default)]
    params: Option<Vec<Value>>,

    /// Named parameters (for :name style)
    #[serde(default)]
    named_params: Option<Value>,

    /// Operation type: "query" (returns rows), "execute" (returns affected rows)
    #[serde(default = "default_operation")]
    operation: String,

    /// Credential name for connection string
    #[serde(default)]
    credential: Option<String>,

    /// Maximum rows to return (for safety)
    #[serde(default = "default_max_rows")]
    max_rows: usize,

    /// Query timeout in seconds
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
}

fn default_db_type() -> String {
    "sqlite".to_string()
}

fn default_operation() -> String {
    "query".to_string()
}

fn default_max_rows() -> usize {
    1000
}

fn default_timeout() -> u64 {
    30
}

/// Database types that are documented but not implemented.
fn unsupported_db_type(db_type: &str) -> Option<String> {
    match db_type {
        "postgres" | "postgresql" | "mysql" => Some(format!(
            "The database node does not support '{}' yet; only sqlite is implemented. \
             For Supabase or PostgREST, use the supabase integration or an http node",
            db_type
        )),
        _ => None,
    }
}

#[async_trait]
impl Node for DatabaseNode {
    fn node_type(&self) -> &str {
        "database"
    }

    fn description(&self) -> &str {
        "Execute SQL queries on databases"
    }

    fn validate_config(&self, config: &Value) -> Result<()> {
        let db_type = config
            .get("db_type")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(default_db_type);
        match unsupported_db_type(&db_type) {
            Some(reason) => Err(Error::Validation(reason)),
            None => Ok(()),
        }
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: DatabaseConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid database config: {}", e)))?;

        // Resolve credential if specified
        let connection_string = if let Some(cred_name) = &config.credential {
            ctx.credentials.get(cred_name).cloned()
        } else {
            config.connection_string.clone()
        };

        debug!(
            db_type = %config.db_type,
            operation = %config.operation,
            "Executing database query"
        );

        match config.db_type.as_str() {
            "sqlite" => execute_sqlite(&config, connection_string).await,
            other if unsupported_db_type(other).is_some() => {
                Err(Error::Node(unsupported_db_type(other).unwrap_or_default()))
            }
            _ => Err(Error::Node(format!(
                "Unknown database type: {}",
                config.db_type
            ))),
        }
    }
}

async fn execute_sqlite(
    config: &DatabaseConfig,
    connection_string: Option<String>,
) -> Result<NodeResult> {
    let path = connection_string.unwrap_or_else(|| ":memory:".to_string());

    // Use tokio's spawn_blocking for synchronous rusqlite operations
    let query = config.query.clone();
    let operation = config.operation.clone();
    let max_rows = config.max_rows;
    let params = config.params.clone();

    let result = tokio::task::spawn_blocking(move || -> Result<Value> {
        let conn = rusqlite::Connection::open(&path)
            .map_err(|e| Error::Node(format!("SQLite connection failed: {}", e)))?;

        if operation == "execute" {
            // Execute (INSERT, UPDATE, DELETE)
            let affected = if let Some(params) = params {
                let params: Vec<Box<dyn rusqlite::ToSql>> =
                    params.iter().map(|v| value_to_sql(v)).collect();
                let params_refs: Vec<&dyn rusqlite::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();
                conn.execute(&query, params_refs.as_slice())
            } else {
                conn.execute(&query, [])
            }
            .map_err(|e| Error::Node(format!("SQLite execute failed: {}", e)))?;

            Ok(json!({
                "success": true,
                "db_type": "sqlite",
                "operation": "execute",
                "affected_rows": affected,
            }))
        } else {
            // Query (SELECT)
            let mut stmt = conn
                .prepare(&query)
                .map_err(|e| Error::Node(format!("SQLite prepare failed: {}", e)))?;

            let column_names: Vec<String> =
                stmt.column_names().iter().map(|s| s.to_string()).collect();
            let column_count = column_names.len();

            let rows_result: std::result::Result<Vec<Vec<Value>>, rusqlite::Error> =
                if let Some(params) = params {
                    let params: Vec<Box<dyn rusqlite::ToSql>> =
                        params.iter().map(|v| value_to_sql(v)).collect();
                    let params_refs: Vec<&dyn rusqlite::ToSql> =
                        params.iter().map(|p| p.as_ref()).collect();
                    stmt.query_map(params_refs.as_slice(), |row| {
                        let mut values = Vec::with_capacity(column_count);
                        for i in 0..column_count {
                            values.push(row_value_to_json(row, i));
                        }
                        Ok(values)
                    })?
                    .collect()
                } else {
                    stmt.query_map([], |row| {
                        let mut values = Vec::with_capacity(column_count);
                        for i in 0..column_count {
                            values.push(row_value_to_json(row, i));
                        }
                        Ok(values)
                    })?
                    .collect()
                };

            let rows =
                rows_result.map_err(|e| Error::Node(format!("SQLite query failed: {}", e)))?;

            // Convert to array of objects
            let records: Vec<Value> = rows
                .into_iter()
                .take(max_rows)
                .map(|row| {
                    let mut obj = serde_json::Map::new();
                    for (i, col) in column_names.iter().enumerate() {
                        obj.insert(col.clone(), row.get(i).cloned().unwrap_or(Value::Null));
                    }
                    Value::Object(obj)
                })
                .collect();

            Ok(json!({
                "success": true,
                "db_type": "sqlite",
                "operation": "query",
                "columns": column_names,
                "rows": records,
                "row_count": records.len(),
            }))
        }
    })
    .await
    .map_err(|e| Error::Node(format!("SQLite task failed: {}", e)))??;

    Ok(NodeResult::new(result))
}

fn value_to_sql(v: &Value) -> Box<dyn rusqlite::ToSql> {
    match v {
        Value::Null => Box::new(Option::<String>::None),
        Value::Bool(b) => Box::new(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Box::new(i)
            } else if let Some(f) = n.as_f64() {
                Box::new(f)
            } else {
                Box::new(n.to_string())
            }
        }
        Value::String(s) => Box::new(s.clone()),
        _ => Box::new(v.to_string()),
    }
}

fn row_value_to_json(row: &rusqlite::Row, idx: usize) -> Value {
    // Try different types
    if let Ok(v) = row.get::<_, i64>(idx) {
        return json!(v);
    }
    if let Ok(v) = row.get::<_, f64>(idx) {
        return json!(v);
    }
    if let Ok(v) = row.get::<_, String>(idx) {
        return json!(v);
    }
    if let Ok(v) = row.get::<_, bool>(idx) {
        return json!(v);
    }
    if let Ok(v) = row.get::<_, Vec<u8>>(idx) {
        return json!(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &v
        ));
    }
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database_config_parse() {
        let config = json!({
            "db_type": "sqlite",
            "query": "SELECT * FROM users",
            "operation": "query"
        });

        let parsed: DatabaseConfig = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.db_type, "sqlite");
        assert_eq!(parsed.operation, "query");
    }

    #[tokio::test]
    async fn test_sqlite_in_memory() {
        let node = DatabaseNode::new();

        // Create table and insert
        let create_config = json!({
            "db_type": "sqlite",
            "connection_string": ":memory:",
            "query": "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)",
            "operation": "execute"
        });
        let ctx = NodeContext::new("exec-1", "test");
        let result = node.execute(&create_config, &ctx).await.unwrap();
        assert_eq!(result.data["success"], true);
    }

    #[tokio::test]
    async fn test_database_with_params() {
        let config = json!({
            "db_type": "sqlite",
            "query": "SELECT * FROM users WHERE id = ?",
            "params": [1],
            "operation": "query"
        });

        let parsed: DatabaseConfig = serde_json::from_value(config).unwrap();
        assert!(parsed.params.is_some());
        assert_eq!(parsed.params.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn postgres_and_mysql_are_refused_not_faked() {
        let node = DatabaseNode::new();
        let ctx = NodeContext::new("exec-1", "test");
        for db in ["postgres", "mysql"] {
            let config =
                json!({"db_type": db, "connection_string": "x://y", "query": "DELETE FROM t"});
            let err = node.validate_config(&config).unwrap_err().to_string();
            assert!(err.contains("only sqlite is implemented"), "{err}");
            assert!(node.execute(&config, &ctx).await.is_err());
        }
        assert!(
            node.validate_config(&json!({"db_type": "sqlite", "query": "SELECT 1"}))
                .is_ok()
        );
    }
}
