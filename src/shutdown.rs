//! Graceful shutdown handling for r8r.
//!
//! This module provides a `ShutdownCoordinator` that manages graceful shutdown
//! by listening for SIGTERM/SIGINT signals and coordinating the shutdown process
//! across the application.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::signal;
use tokio::sync::Notify;
use tracing::{info, warn};

/// Coordinates graceful shutdown across the application.
///
/// The shutdown coordinator listens for system signals (SIGTERM/SIGINT) and
/// provides mechanisms for components to:
/// - Check if shutdown has been requested
/// - Wait for shutdown to be requested
/// - Request shutdown programmatically
///
/// # Example
///
/// ```rust
/// use r8r::shutdown::ShutdownCoordinator;
///
/// async fn example() {
///     let coordinator = ShutdownCoordinator::new();
///     
///     // Check if shutdown is requested
///     if coordinator.is_shutdown_requested() {
///         // Clean up and exit
///     }
///     
///     // Wait for shutdown signal
///     coordinator.wait_for_shutdown().await;
/// }
/// ```
#[derive(Clone)]
pub struct ShutdownCoordinator {
    shutdown_requested: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl ShutdownCoordinator {
    /// Create a new shutdown coordinator.
    pub fn new() -> Self {
        Self {
            shutdown_requested: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Request shutdown.
    ///
    /// This sets the shutdown flag and notifies all waiters.
    /// Can be called multiple times safely.
    pub fn request_shutdown(&self) {
        let was_requested = self.shutdown_requested.swap(true, Ordering::SeqCst);
        if !was_requested {
            info!("Shutdown requested");
            self.notify.notify_waiters();
        }
    }

    /// Check if shutdown has been requested.
    pub fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::SeqCst)
    }

    /// Wait for shutdown to be requested.
    ///
    /// This future resolves when shutdown is requested via signal or programmatically.
    /// If shutdown is already requested, this returns immediately.
    pub async fn wait_for_shutdown(&self) {
        if self.is_shutdown_requested() {
            return;
        }

        // Wait for the notify signal
        self.notify.notified().await;
    }

    /// Start listening for shutdown signals.
    ///
    /// This spawns a task that listens for SIGTERM and SIGINT signals.
    /// When a signal is received, shutdown is requested.
    pub fn start_signal_listener(&self) {
        let coordinator = self.clone();

        tokio::spawn(async move {
            let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Failed to create SIGTERM handler");
            let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())
                .expect("Failed to create SIGINT handler");

            tokio::select! {
                _ = sigterm.recv() => {
                    info!("Received SIGTERM, initiating graceful shutdown");
                }
                _ = sigint.recv() => {
                    info!("Received SIGINT, initiating graceful shutdown");
                }
            }

            coordinator.request_shutdown();
        });
    }

    /// Start a cross-platform signal listener.
    ///
    /// On Unix: listens for SIGTERM and SIGINT
    /// On Windows: listens for Ctrl+C (ctrl_c)
    pub fn start_cross_platform_signal_listener(&self) {
        let coordinator = self.clone();

        tokio::spawn(async move {
            #[cfg(unix)]
            {
                let mut sigterm = match signal::unix::signal(signal::unix::SignalKind::terminate())
                {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Failed to create SIGTERM handler: {}", e);
                        // Fall back to ctrl_c only
                        tokio::signal::ctrl_c().await.ok();
                        coordinator.request_shutdown();
                        return;
                    }
                };
                let mut sigint = match signal::unix::signal(signal::unix::SignalKind::interrupt()) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Failed to create SIGINT handler: {}", e);
                        // Fall back to just SIGTERM
                        sigterm.recv().await;
                        coordinator.request_shutdown();
                        return;
                    }
                };

                tokio::select! {
                    _ = sigterm.recv() => {
                        info!("Received SIGTERM, initiating graceful shutdown");
                    }
                    _ = sigint.recv() => {
                        info!("Received SIGINT, initiating graceful shutdown");
                    }
                }
            }

            #[cfg(not(unix))]
            {
                if let Err(e) = tokio::signal::ctrl_c().await {
                    warn!("Failed to listen for Ctrl+C: {}", e);
                    return;
                }
                info!("Received Ctrl+C, initiating graceful shutdown");
            }

            coordinator.request_shutdown();
        });
    }
}

impl Default for ShutdownCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

/// Error type indicating that shutdown is in progress.
///
/// This error is returned when an operation cannot be completed because
/// the system is shutting down.
#[derive(Debug, Clone)]
pub struct ShutdownInProgressError;

impl std::fmt::Display for ShutdownInProgressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Shutdown in progress")
    }
}

impl std::error::Error for ShutdownInProgressError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_shutdown_coordinator_new() {
        let coordinator = ShutdownCoordinator::new();
        assert!(!coordinator.is_shutdown_requested());
    }

    #[tokio::test]
    async fn test_shutdown_request() {
        let coordinator = ShutdownCoordinator::new();

        assert!(!coordinator.is_shutdown_requested());

        coordinator.request_shutdown();

        assert!(coordinator.is_shutdown_requested());
    }

    #[tokio::test]
    async fn test_shutdown_wait_already_requested() {
        let coordinator = ShutdownCoordinator::new();

        coordinator.request_shutdown();

        // Should return immediately since shutdown is already requested
        let result =
            tokio::time::timeout(Duration::from_millis(100), coordinator.wait_for_shutdown()).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_shutdown_wait_then_request() {
        let coordinator = ShutdownCoordinator::new();
        let coordinator2 = coordinator.clone();

        // Spawn a task that will request shutdown after a short delay
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            coordinator2.request_shutdown();
        });

        // Wait for shutdown - should complete when the other task requests it
        let result =
            tokio::time::timeout(Duration::from_secs(1), coordinator.wait_for_shutdown()).await;

        assert!(result.is_ok());
        assert!(coordinator.is_shutdown_requested());
    }

    #[tokio::test]
    async fn test_multiple_shutdown_requests() {
        let coordinator = ShutdownCoordinator::new();

        // Multiple requests should not cause issues
        coordinator.request_shutdown();
        coordinator.request_shutdown();
        coordinator.request_shutdown();

        assert!(coordinator.is_shutdown_requested());
    }
}
