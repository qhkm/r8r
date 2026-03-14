/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Bridge module for r8r <-> ZeptoClaw real-time communication.
//!
//! Provides event types, a replay buffer for at-least-once delivery,
//! and a WebSocket endpoint for bidirectional event streaming.

pub mod buffer;
pub mod events;
pub mod handlers;
pub mod websocket;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

use crate::api::Monitor;
use crate::engine::{Executor, PauseRegistry};
use crate::nodes::NodeRegistry;
use crate::storage::Storage;

use self::buffer::ReplayBuffer;
use self::events::{BridgeEvent, BridgeEventEnvelope};

#[derive(Clone)]
struct BridgeExecutionContext {
    registry: Arc<NodeRegistry>,
    monitor: Option<Arc<Monitor>>,
    pause_registry: PauseRegistry,
}

/// Shared state for the bridge subsystem.
///
/// Holds the replay buffer, current WebSocket client sender, health tracking,
/// auth token, and an optional storage handle for approval resolution.
pub struct BridgeState {
    /// Replay buffer for at-least-once delivery (256-event cap, 300s TTL).
    pub buffer: Arc<Mutex<ReplayBuffer>>,

    /// Channel sender to the currently connected WebSocket client.
    /// `None` when no client is connected.
    pub client_tx: Arc<Mutex<Option<mpsc::Sender<String>>>>,

    /// Timestamp of the last health ping received from ZeptoClaw.
    pub last_ping: Arc<Mutex<Option<DateTime<Utc>>>>,

    /// Authentication token for the bridge connection.
    pub token: Option<String>,

    /// Optional storage backend for approval resolution.
    pub storage: Arc<Mutex<Option<Arc<dyn Storage>>>>,

    /// Executor dependencies for bridge-triggered workflow control.
    execution_context: Arc<Mutex<Option<BridgeExecutionContext>>>,
}

impl BridgeState {
    /// Create a new `BridgeState`.
    ///
    /// Token resolution order:
    /// 1. Explicit `token` parameter
    /// 2. `R8R_BRIDGE_TOKEN` environment variable
    /// 3. `R8R_API_KEY` environment variable
    /// 4. `None`
    pub fn new(token: Option<String>) -> Self {
        let resolved_token = token
            .or_else(|| std::env::var("R8R_BRIDGE_TOKEN").ok())
            .or_else(|| std::env::var("R8R_API_KEY").ok());

        Self {
            buffer: Arc::new(Mutex::new(ReplayBuffer::new(256, 300))),
            client_tx: Arc::new(Mutex::new(None)),
            last_ping: Arc::new(Mutex::new(None)),
            token: resolved_token,
            storage: Arc::new(Mutex::new(None)),
            execution_context: Arc::new(Mutex::new(None)),
        }
    }

    /// Attach runtime dependencies so bridge handlers can execute workflows.
    pub async fn configure_execution_context(
        &self,
        registry: Arc<NodeRegistry>,
        monitor: Option<Arc<Monitor>>,
        pause_registry: PauseRegistry,
    ) {
        let mut guard = self.execution_context.lock().await;
        *guard = Some(BridgeExecutionContext {
            registry,
            monitor,
            pause_registry,
        });
    }

    /// Returns `true` if a WebSocket client is currently connected.
    pub async fn is_connected(&self) -> bool {
        self.client_tx.lock().await.is_some()
    }

    /// Returns the timestamp of the last health ping, if any.
    pub async fn last_ping_time(&self) -> Option<DateTime<Utc>> {
        *self.last_ping.lock().await
    }

    /// Send an event through the bridge.
    ///
    /// The event is **always** buffered first (for at-least-once delivery),
    /// then forwarded to the connected client if one exists.
    pub async fn send_event(
        &self,
        event: BridgeEvent,
        correlation_id: Option<String>,
    ) -> Result<(), String> {
        let envelope = BridgeEventEnvelope::new(event, correlation_id);
        let json = serde_json::to_string(&envelope)
            .map_err(|e| format!("Failed to serialize bridge event: {}", e))?;

        // Always buffer first
        {
            let mut buf = self.buffer.lock().await;
            buf.push(envelope);
        }

        // Try to send to connected client
        let tx_guard = self.client_tx.lock().await;
        if let Some(tx) = tx_guard.as_ref() {
            match tx.try_send(json) {
                Ok(_) => {
                    info!("Bridge event sent to client");
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    warn!("Bridge client channel full, event buffered only");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    warn!("Bridge client disconnected, event buffered only");
                }
            }
        }

        Ok(())
    }

    /// Clone the configured storage backend, if available.
    pub async fn storage_backend(&self) -> Option<Arc<dyn Storage>> {
        self.storage.lock().await.as_ref().cloned()
    }

    /// Build an executor configured the same way as the API-triggered paths.
    pub async fn create_executor(self: &Arc<Self>) -> Option<Executor> {
        let storage = self.storage_backend().await?;
        let execution_context = self.execution_context.lock().await.clone()?;

        let mut executor = Executor::new((*execution_context.registry).clone(), storage);
        if let Some(monitor) = execution_context.monitor {
            executor = executor.with_monitor(monitor);
        }

        Some(
            executor
                .with_pause_registry(execution_context.pause_registry)
                .with_bridge(self.clone()),
        )
    }
}
