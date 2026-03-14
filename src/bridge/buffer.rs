/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Replay buffer for at-least-once delivery of bridge events.
//!
//! Buffers events so they can be replayed to newly connected clients.
//! Placeholder implementation — will be fully fleshed out in Task 2.

use std::collections::VecDeque;

use chrono::Utc;

use super::events::BridgeEventEnvelope;

/// A bounded, TTL-aware replay buffer for bridge events.
///
/// Events beyond `max_events` or older than `ttl_secs` are evicted.
pub struct ReplayBuffer {
    events: VecDeque<BridgeEventEnvelope>,
    max_events: usize,
    ttl_secs: u64,
}

impl ReplayBuffer {
    /// Create a new replay buffer with the given capacity and TTL.
    pub fn new(max_events: usize, ttl_secs: u64) -> Self {
        Self {
            events: VecDeque::with_capacity(max_events),
            max_events,
            ttl_secs,
        }
    }

    /// Push an event into the buffer, evicting old entries if needed.
    pub fn push(&mut self, event: BridgeEventEnvelope) {
        // Evict expired events
        let cutoff = Utc::now() - chrono::Duration::seconds(self.ttl_secs as i64);
        while self
            .events
            .front()
            .map_or(false, |e| e.timestamp < cutoff)
        {
            self.events.pop_front();
        }

        // Evict oldest if at capacity
        if self.events.len() >= self.max_events {
            self.events.pop_front();
        }

        self.events.push_back(event);
    }

    /// Returns the number of buffered events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns true if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}
