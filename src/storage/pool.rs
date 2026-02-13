//! Connection pooling for SQLite storage.
//!
//! This module provides an optional connection pool for high-concurrency scenarios.
//! It uses r2d2 for connection pooling with SQLite.
//!
//! # When to Use
//!
//! The default `SqliteStorage` with `tokio::sync::Mutex` is sufficient for most workloads.
//! Use the pooled storage when:
//! - You have many concurrent read operations
//! - Read latency is critical
//! - You're running multiple async tasks that need database access
//!
//! # Environment Variables
//!
//! - `R8R_DB_POOL_SIZE`: Number of connections in the pool (default: 4)
//! - `R8R_DB_POOL_TIMEOUT_SECS`: Connection acquisition timeout (default: 30)

use std::path::Path;
use std::time::Duration;

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

use crate::error::{Error, Result};

/// Default pool size.
const DEFAULT_POOL_SIZE: u32 = 4;
/// Default connection timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Configuration for the connection pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Number of connections in the pool.
    pub pool_size: u32,
    /// Timeout for acquiring a connection.
    pub connection_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            pool_size: std::env::var("R8R_DB_POOL_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_POOL_SIZE),
            connection_timeout: Duration::from_secs(
                std::env::var("R8R_DB_POOL_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_TIMEOUT_SECS),
            ),
        }
    }
}

/// Connection pool for SQLite.
#[derive(Clone)]
pub struct ConnectionPool {
    pool: Pool<SqliteConnectionManager>,
}

impl ConnectionPool {
    /// Create a new connection pool for the given database path.
    pub fn new(path: &Path, config: PoolConfig) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path)
            .with_init(|conn| {
                // Configure each connection
                conn.execute_batch(
                    r#"
                    PRAGMA journal_mode = WAL;
                    PRAGMA busy_timeout = 5000;
                    PRAGMA synchronous = NORMAL;
                    PRAGMA foreign_keys = ON;
                    "#,
                )?;
                Ok(())
            });

        let pool = Pool::builder()
            .max_size(config.pool_size)
            .connection_timeout(config.connection_timeout)
            .build(manager)
            .map_err(|e| Error::Storage(format!("Failed to create connection pool: {}", e)))?;

        Ok(Self { pool })
    }

    /// Create an in-memory connection pool (for testing).
    ///
    /// Note: In-memory databases with connection pools have limitations
    /// as each connection gets its own in-memory database.
    /// For testing, prefer using `SqliteStorage::open_in_memory()`.
    pub fn new_in_memory(config: PoolConfig) -> Result<Self> {
        // Use shared cache for in-memory databases
        let manager = SqliteConnectionManager::memory()
            .with_init(|conn| {
                conn.execute_batch(
                    r#"
                    PRAGMA foreign_keys = ON;
                    "#,
                )?;
                Ok(())
            });

        let pool = Pool::builder()
            .max_size(config.pool_size)
            .connection_timeout(config.connection_timeout)
            .build(manager)
            .map_err(|e| Error::Storage(format!("Failed to create connection pool: {}", e)))?;

        Ok(Self { pool })
    }

    /// Get a connection from the pool.
    pub fn get(&self) -> Result<PooledConnection<SqliteConnectionManager>> {
        self.pool
            .get()
            .map_err(|e| Error::Storage(format!("Failed to acquire connection: {}", e)))
    }

    /// Get a connection, blocking the current thread.
    ///
    /// This is useful when you need synchronous access from an async context.
    pub fn get_timeout(&self, timeout: Duration) -> Result<PooledConnection<SqliteConnectionManager>> {
        self.pool
            .get_timeout(timeout)
            .map_err(|e| Error::Storage(format!("Connection timeout: {}", e)))
    }

    /// Get pool statistics.
    pub fn stats(&self) -> PoolStats {
        let state = self.pool.state();
        PoolStats {
            connections: state.connections,
            idle_connections: state.idle_connections,
        }
    }

    /// Execute a function with a connection from the pool.
    ///
    /// This is the preferred way to use the pool in async contexts,
    /// as it handles the connection lifecycle automatically.
    pub async fn with_connection<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let pool = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get()?;
            f(&conn)
        })
        .await
        .map_err(|e| Error::Storage(format!("Task failed: {}", e)))?
    }

    /// Execute a function with mutable connection access.
    pub async fn with_connection_mut<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let pool = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get()?;
            f(&mut conn)
        })
        .await
        .map_err(|e| Error::Storage(format!("Task failed: {}", e)))?
    }
}

/// Pool statistics.
#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    /// Total number of connections.
    pub connections: u32,
    /// Number of idle connections.
    pub idle_connections: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_default_config() {
        let config = PoolConfig::default();
        assert_eq!(config.pool_size, DEFAULT_POOL_SIZE);
        assert_eq!(config.connection_timeout.as_secs(), DEFAULT_TIMEOUT_SECS);
    }

    #[tokio::test]
    async fn test_pool_basic_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let config = PoolConfig {
            pool_size: 2,
            connection_timeout: Duration::from_secs(5),
        };

        let pool = ConnectionPool::new(&db_path, config).unwrap();

        // Create a test table
        pool.with_connection_mut(|conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS test (id INTEGER PRIMARY KEY, value TEXT)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // Insert data
        pool.with_connection_mut(|conn| {
            conn.execute("INSERT INTO test (value) VALUES (?1)", ["hello"])?;
            Ok(())
        })
        .await
        .unwrap();

        // Read data
        let value: String = pool
            .with_connection(|conn| {
                let value: String =
                    conn.query_row("SELECT value FROM test WHERE id = 1", [], |row| row.get(0))?;
                Ok(value)
            })
            .await
            .unwrap();

        assert_eq!(value, "hello");
    }

    #[tokio::test]
    async fn test_pool_concurrent_access() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_concurrent.db");

        let config = PoolConfig {
            pool_size: 4,
            connection_timeout: Duration::from_secs(10),
        };

        let pool = Arc::new(ConnectionPool::new(&db_path, config).unwrap());

        // Create table
        pool.with_connection_mut(|conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS counter (id INTEGER PRIMARY KEY, count INTEGER)",
                [],
            )?;
            conn.execute("INSERT INTO counter (id, count) VALUES (1, 0)", [])?;
            Ok(())
        })
        .await
        .unwrap();

        // Concurrent increments
        let counter = Arc::new(AtomicU32::new(0));
        let mut handles = vec![];

        for _ in 0..10 {
            let pool = Arc::clone(&pool);
            let counter = Arc::clone(&counter);
            let handle = tokio::spawn(async move {
                pool.with_connection_mut(move |conn| {
                    conn.execute("UPDATE counter SET count = count + 1 WHERE id = 1", [])?;
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .await
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap().unwrap();
        }

        // Verify final count
        let count: i32 = pool
            .with_connection(|conn| {
                let count: i32 =
                    conn.query_row("SELECT count FROM counter WHERE id = 1", [], |row| row.get(0))?;
                Ok(count)
            })
            .await
            .unwrap();

        assert_eq!(count, 10);
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn test_pool_stats() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_stats.db");

        let config = PoolConfig {
            pool_size: 3,
            connection_timeout: Duration::from_secs(5),
        };

        let pool = ConnectionPool::new(&db_path, config).unwrap();

        // Initially should have idle connections
        let stats = pool.stats();
        assert!(stats.connections <= 3);
    }
}
