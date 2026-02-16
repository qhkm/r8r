//! Per-workflow rate limiting.
//!
//! Implements a token bucket rate limiter to prevent workflow abuse
//! and ensure fair resource usage.
//!
//! ## Configuration
//!
//! Rate limits can be configured per workflow in the workflow YAML:
//!
//! ```yaml
//! metadata:
//!   rate_limit:
//!     requests_per_minute: 60
//!     burst_size: 10
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::Duration;

/// Rate limit configuration.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum requests per time window
    pub requests_per_window: u32,
    /// Time window duration
    pub window: Duration,
    /// Burst size (tokens available immediately)
    pub burst_size: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_window: 60,
            window: Duration::from_secs(60),
            burst_size: 10,
        }
    }
}

impl RateLimitConfig {
    /// Create a config with requests per minute.
    pub fn per_minute(requests: u32) -> Self {
        Self {
            requests_per_window: requests,
            window: Duration::from_secs(60),
            burst_size: requests.min(10),
        }
    }

    /// Create a config with requests per second.
    pub fn per_second(requests: u32) -> Self {
        Self {
            requests_per_window: requests,
            window: Duration::from_secs(1),
            burst_size: requests.min(5),
        }
    }

    /// Set burst size.
    pub fn with_burst(mut self, burst: u32) -> Self {
        self.burst_size = burst;
        self
    }
}

/// Token bucket rate limiter for a single workflow.
pub struct TokenBucket {
    /// Available tokens (scaled by 1000 for precision)
    tokens: AtomicU64,
    /// Maximum tokens (burst size * 1000)
    max_tokens: u64,
    /// Token refill rate per millisecond (scaled by 1000)
    refill_rate: u64,
    /// Last refill timestamp (unix millis)
    last_refill: AtomicU64,
}

impl TokenBucket {
    /// Create a new token bucket with the given configuration.
    pub fn new(config: &RateLimitConfig) -> Self {
        let max_tokens = (config.burst_size as u64) * 1000;
        // Calculate refill rate: tokens per window / window_millis
        let window_millis = config.window.as_millis() as u64;
        let refill_rate = if window_millis > 0 {
            ((config.requests_per_window as u64) * 1000) / window_millis
        } else {
            config.requests_per_window as u64 * 1000
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            tokens: AtomicU64::new(max_tokens),
            max_tokens,
            refill_rate,
            last_refill: AtomicU64::new(now),
        }
    }

    /// Try to acquire a token. Returns true if successful.
    pub fn try_acquire(&self) -> bool {
        self.refill();

        loop {
            let current = self.tokens.load(Ordering::SeqCst);
            if current < 1000 {
                return false;
            }

            let new_tokens = current - 1000;
            if self
                .tokens
                .compare_exchange(current, new_tokens, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return true;
            }
            // CAS failed, retry
        }
    }

    /// Check if a token is available without consuming it.
    pub fn check(&self) -> bool {
        self.refill();
        self.tokens.load(Ordering::SeqCst) >= 1000
    }

    /// Get current available tokens.
    pub fn available_tokens(&self) -> u32 {
        self.refill();
        (self.tokens.load(Ordering::SeqCst) / 1000) as u32
    }

    /// Refill tokens based on elapsed time.
    fn refill(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let last = self.last_refill.load(Ordering::SeqCst);
        let elapsed = now.saturating_sub(last);

        if elapsed == 0 {
            return;
        }

        // Calculate tokens to add
        let tokens_to_add = elapsed * self.refill_rate;

        if tokens_to_add > 0 {
            loop {
                let current = self.tokens.load(Ordering::SeqCst);
                let new_tokens = (current + tokens_to_add).min(self.max_tokens);

                if self
                    .tokens
                    .compare_exchange(current, new_tokens, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    self.last_refill.store(now, Ordering::SeqCst);
                    break;
                }
            }
        }
    }
}

/// Rate limiter registry for all workflows.
pub struct RateLimiterRegistry {
    limiters: RwLock<HashMap<String, TokenBucket>>,
    default_config: RateLimitConfig,
}

impl RateLimiterRegistry {
    /// Create a new registry with default configuration.
    pub fn new() -> Self {
        Self {
            limiters: RwLock::new(HashMap::new()),
            default_config: RateLimitConfig::default(),
        }
    }

    /// Create a new registry with custom default configuration.
    pub fn with_config(config: RateLimitConfig) -> Self {
        Self {
            limiters: RwLock::new(HashMap::new()),
            default_config: config,
        }
    }

    /// Try to acquire a rate limit token for a workflow.
    pub fn try_acquire(&self, workflow_name: &str) -> bool {
        // First try to get existing limiter
        {
            let limiters = self.limiters.read().unwrap();
            if let Some(limiter) = limiters.get(workflow_name) {
                return limiter.try_acquire();
            }
        }

        // Need to create new limiter
        let mut limiters = self.limiters.write().unwrap();
        let limiter = limiters
            .entry(workflow_name.to_string())
            .or_insert_with(|| TokenBucket::new(&self.default_config));
        limiter.try_acquire()
    }

    /// Try to acquire with a custom config for this workflow.
    pub fn try_acquire_with_config(&self, workflow_name: &str, config: &RateLimitConfig) -> bool {
        let mut limiters = self.limiters.write().unwrap();
        let limiter = limiters
            .entry(workflow_name.to_string())
            .or_insert_with(|| TokenBucket::new(config));
        limiter.try_acquire()
    }

    /// Check if a workflow is rate limited (without consuming a token).
    pub fn is_limited(&self, workflow_name: &str) -> bool {
        let limiters = self.limiters.read().unwrap();
        match limiters.get(workflow_name) {
            Some(limiter) => !limiter.check(),
            None => false,
        }
    }

    /// Get available tokens for a workflow.
    pub fn available_tokens(&self, workflow_name: &str) -> u32 {
        let limiters = self.limiters.read().unwrap();
        match limiters.get(workflow_name) {
            Some(limiter) => limiter.available_tokens(),
            None => self.default_config.burst_size,
        }
    }

    /// Register a workflow with custom rate limit.
    pub fn register(&self, workflow_name: &str, config: RateLimitConfig) {
        let mut limiters = self.limiters.write().unwrap();
        limiters.insert(workflow_name.to_string(), TokenBucket::new(&config));
    }

    /// Remove rate limit for a workflow.
    pub fn remove(&self, workflow_name: &str) {
        let mut limiters = self.limiters.write().unwrap();
        limiters.remove(workflow_name);
    }

    /// Clear all rate limiters.
    pub fn clear(&self) {
        let mut limiters = self.limiters.write().unwrap();
        limiters.clear();
    }
}

impl Default for RateLimiterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_bucket_basic() {
        let config = RateLimitConfig {
            requests_per_window: 10,
            window: Duration::from_secs(1),
            burst_size: 5,
        };
        let bucket = TokenBucket::new(&config);

        // Should be able to acquire burst_size tokens immediately
        for _ in 0..5 {
            assert!(bucket.try_acquire());
        }

        // Next one should fail (no more burst tokens)
        assert!(!bucket.try_acquire());
    }

    #[test]
    fn test_rate_limit_config_per_minute() {
        let config = RateLimitConfig::per_minute(120);
        assert_eq!(config.requests_per_window, 120);
        assert_eq!(config.window, Duration::from_secs(60));
        assert_eq!(config.burst_size, 10); // capped at 10
    }

    #[test]
    fn test_rate_limit_config_per_second() {
        let config = RateLimitConfig::per_second(10);
        assert_eq!(config.requests_per_window, 10);
        assert_eq!(config.window, Duration::from_secs(1));
        assert_eq!(config.burst_size, 5); // capped at 5
    }

    #[test]
    fn test_registry_basic() {
        let registry = RateLimiterRegistry::with_config(RateLimitConfig {
            requests_per_window: 10,
            window: Duration::from_secs(1),
            burst_size: 3,
        });

        // Should be able to acquire 3 tokens
        assert!(registry.try_acquire("workflow1"));
        assert!(registry.try_acquire("workflow1"));
        assert!(registry.try_acquire("workflow1"));

        // Next one should fail
        assert!(!registry.try_acquire("workflow1"));

        // Different workflow should work
        assert!(registry.try_acquire("workflow2"));
    }

    #[test]
    fn test_available_tokens() {
        let registry = RateLimiterRegistry::with_config(RateLimitConfig {
            requests_per_window: 10,
            window: Duration::from_secs(1),
            burst_size: 5,
        });

        // Before any requests
        assert_eq!(registry.available_tokens("workflow1"), 5);

        registry.try_acquire("workflow1");
        assert_eq!(registry.available_tokens("workflow1"), 4);

        registry.try_acquire("workflow1");
        assert_eq!(registry.available_tokens("workflow1"), 3);
    }

    #[test]
    fn test_is_limited() {
        let registry = RateLimiterRegistry::with_config(RateLimitConfig {
            requests_per_window: 10,
            window: Duration::from_secs(1),
            burst_size: 2,
        });

        // Not limited initially
        assert!(!registry.is_limited("workflow1"));

        // Consume all tokens
        registry.try_acquire("workflow1");
        registry.try_acquire("workflow1");

        // Now limited
        assert!(registry.is_limited("workflow1"));
    }

    #[test]
    fn test_custom_workflow_config() {
        let registry = RateLimiterRegistry::new();

        // Register with custom config
        registry.register(
            "special-workflow",
            RateLimitConfig {
                requests_per_window: 100,
                window: Duration::from_secs(60),
                burst_size: 20,
            },
        );

        assert_eq!(registry.available_tokens("special-workflow"), 20);
    }
}
