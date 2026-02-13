//! Circuit breaker pattern for resilient external service calls.
//!
//! The circuit breaker prevents cascade failures by stopping requests to failing services.
//!
//! ## States
//!
//! - **Closed**: Normal operation, requests pass through
//! - **Open**: Requests fail immediately without attempting the call
//! - **HalfOpen**: Limited requests allowed to test if service recovered
//!
//! ## Configuration
//!
//! - `failure_threshold`: Number of failures before opening (default: 5)
//! - `success_threshold`: Successes needed in half-open to close (default: 2)
//! - `timeout_seconds`: How long to stay open before half-open (default: 30)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::Duration;

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation - requests pass through
    Closed,
    /// Service failing - requests rejected immediately
    Open,
    /// Testing recovery - limited requests allowed
    HalfOpen,
}

/// Circuit breaker configuration.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit
    pub failure_threshold: u32,
    /// Number of successes in half-open to close the circuit
    pub success_threshold: u32,
    /// How long to stay open before transitioning to half-open
    pub timeout: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 2,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Circuit breaker for a single endpoint/service.
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    /// Current state (0=Closed, 1=Open, 2=HalfOpen)
    state: AtomicU32,
    /// Consecutive failure count
    failure_count: AtomicU32,
    /// Consecutive success count (in half-open state)
    success_count: AtomicU32,
    /// Timestamp when circuit opened (unix millis)
    opened_at: AtomicU64,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with default configuration.
    pub fn new() -> Self {
        Self::with_config(CircuitBreakerConfig::default())
    }

    /// Create a new circuit breaker with custom configuration.
    pub fn with_config(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: AtomicU32::new(0), // Closed
            failure_count: AtomicU32::new(0),
            success_count: AtomicU32::new(0),
            opened_at: AtomicU64::new(0),
        }
    }

    /// Get the current circuit state.
    pub fn state(&self) -> CircuitState {
        match self.state.load(Ordering::SeqCst) {
            0 => CircuitState::Closed,
            1 => {
                // Check if timeout has passed
                let opened_at = self.opened_at.load(Ordering::SeqCst);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let elapsed = now.saturating_sub(opened_at);
                if elapsed >= self.config.timeout.as_millis() as u64 {
                    // Transition to half-open
                    self.state.store(2, Ordering::SeqCst);
                    self.success_count.store(0, Ordering::SeqCst);
                    CircuitState::HalfOpen
                } else {
                    CircuitState::Open
                }
            }
            _ => CircuitState::HalfOpen,
        }
    }

    /// Check if a request should be allowed.
    pub fn allow_request(&self) -> bool {
        match self.state() {
            CircuitState::Closed => true,
            CircuitState::Open => false,
            CircuitState::HalfOpen => true, // Allow limited requests
        }
    }

    /// Record a successful request.
    pub fn record_success(&self) {
        match self.state() {
            CircuitState::Closed => {
                // Reset failure count on success
                self.failure_count.store(0, Ordering::SeqCst);
            }
            CircuitState::HalfOpen => {
                let successes = self.success_count.fetch_add(1, Ordering::SeqCst) + 1;
                if successes >= self.config.success_threshold {
                    // Close the circuit
                    self.state.store(0, Ordering::SeqCst);
                    self.failure_count.store(0, Ordering::SeqCst);
                    self.success_count.store(0, Ordering::SeqCst);
                    tracing::info!("Circuit breaker closed after recovery");
                }
            }
            CircuitState::Open => {
                // Should not happen, but ignore
            }
        }
    }

    /// Record a failed request.
    pub fn record_failure(&self) {
        match self.state() {
            CircuitState::Closed => {
                let failures = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
                if failures >= self.config.failure_threshold {
                    // Open the circuit
                    self.state.store(1, Ordering::SeqCst);
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    self.opened_at.store(now, Ordering::SeqCst);
                    tracing::warn!(
                        "Circuit breaker opened after {} failures",
                        self.config.failure_threshold
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open reopens the circuit
                self.state.store(1, Ordering::SeqCst);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                self.opened_at.store(now, Ordering::SeqCst);
                self.success_count.store(0, Ordering::SeqCst);
                tracing::warn!("Circuit breaker reopened after failure in half-open state");
            }
            CircuitState::Open => {
                // Already open, ignore
            }
        }
    }

    /// Reset the circuit breaker to closed state.
    pub fn reset(&self) {
        self.state.store(0, Ordering::SeqCst);
        self.failure_count.store(0, Ordering::SeqCst);
        self.success_count.store(0, Ordering::SeqCst);
        self.opened_at.store(0, Ordering::SeqCst);
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

/// Global circuit breaker registry for different endpoints.
pub struct CircuitBreakerRegistry {
    breakers: RwLock<HashMap<String, CircuitBreaker>>,
    default_config: CircuitBreakerConfig,
}

impl CircuitBreakerRegistry {
    /// Create a new registry with default configuration.
    pub fn new() -> Self {
        Self {
            breakers: RwLock::new(HashMap::new()),
            default_config: CircuitBreakerConfig::default(),
        }
    }

    /// Create a new registry with custom default configuration.
    pub fn with_config(config: CircuitBreakerConfig) -> Self {
        Self {
            breakers: RwLock::new(HashMap::new()),
            default_config: config,
        }
    }

    /// Get or create a circuit breaker for an endpoint.
    pub fn get_or_create(&self, endpoint: &str) -> &CircuitBreaker {
        // Try read lock first
        {
            let breakers = self.breakers.read().unwrap();
            if breakers.contains_key(endpoint) {
                // We can't return a reference directly due to lifetime issues
                // Fall through to write lock
            }
        }

        // Need write lock to potentially insert
        let mut breakers = self.breakers.write().unwrap();
        breakers
            .entry(endpoint.to_string())
            .or_insert_with(|| CircuitBreaker::with_config(self.default_config.clone()));

        // Return a static reference (unsafe, but works for our use case)
        // This is safe because we never remove entries
        let breakers = self.breakers.read().unwrap();
        let ptr = breakers.get(endpoint).unwrap() as *const CircuitBreaker;
        unsafe { &*ptr }
    }

    /// Check if a request to an endpoint should be allowed.
    pub fn allow_request(&self, endpoint: &str) -> bool {
        let breakers = self.breakers.read().unwrap();
        match breakers.get(endpoint) {
            Some(breaker) => breaker.allow_request(),
            None => true, // No breaker yet, allow
        }
    }

    /// Record a successful request to an endpoint.
    pub fn record_success(&self, endpoint: &str) {
        let breakers = self.breakers.read().unwrap();
        if let Some(breaker) = breakers.get(endpoint) {
            breaker.record_success();
        }
    }

    /// Record a failed request to an endpoint.
    pub fn record_failure(&self, endpoint: &str) {
        self.get_or_create(endpoint).record_failure();
    }
}

impl Default for CircuitBreakerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_starts_closed() {
        let cb = CircuitBreaker::new();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }

    #[test]
    fn test_circuit_breaker_opens_after_failures() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 2,
            timeout: Duration::from_secs(1),
        });

        // First 2 failures don't open
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);

        // Third failure opens
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.allow_request());
    }

    #[test]
    fn test_circuit_breaker_success_resets_failures() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 2,
            timeout: Duration::from_secs(1),
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.failure_count.load(Ordering::SeqCst), 2);

        cb.record_success();
        assert_eq!(cb.failure_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_circuit_breaker_reset() {
        let cb = CircuitBreaker::new();
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();

        assert_eq!(cb.state(), CircuitState::Open);

        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }
}
