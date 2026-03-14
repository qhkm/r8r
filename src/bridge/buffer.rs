/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Replay buffer for at-least-once delivery of bridge events.
//!
//! Stores unacknowledged outbound events in a `VecDeque` ordered by insertion
//! time. On reconnect, r8r replays all events that haven't been acknowledged
//! and haven't expired past the configured TTL.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::events::BridgeEventEnvelope;

/// A bounded, TTL-aware replay buffer for bridge events.
///
/// Events beyond `max_events` or older than `ttl_secs` are evicted.
/// Uses `VecDeque` for O(1) front removal during capacity eviction
/// and TTL pruning.
pub struct ReplayBuffer {
    /// Events paired with their insertion timestamp for TTL tracking.
    events: VecDeque<(BridgeEventEnvelope, Instant)>,
    /// Maximum number of events to retain.
    max_events: usize,
    /// Time-to-live in seconds; events older than this are pruned.
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

    /// Push an event into the buffer.
    ///
    /// If the buffer is at capacity, the oldest event is dropped and a
    /// warning is logged via `tracing::warn!`.
    pub fn push(&mut self, event: BridgeEventEnvelope) {
        if self.events.len() >= self.max_events {
            if let Some(dropped) = self.events.pop_front() {
                tracing::warn!(
                    "Bridge: replay buffer full ({}), dropping event {}",
                    self.max_events,
                    dropped.0.id
                );
            }
        }
        self.events.push_back((event, Instant::now()));
    }

    /// Remove the event with the given ID from the buffer (acknowledge it).
    ///
    /// If the ID is not found this is a no-op.
    pub fn ack(&mut self, event_id: &str) {
        self.events.retain(|(e, _)| e.id != event_id);
    }

    /// Return clones of all non-expired events, oldest first.
    ///
    /// Expired events are not removed by this call; use [`prune_expired`]
    /// for that.
    pub fn unacked_events(&self) -> Vec<BridgeEventEnvelope> {
        let cutoff = Duration::from_secs(self.ttl_secs);
        let now = Instant::now();
        self.events
            .iter()
            .filter(|(_, inserted_at)| now.duration_since(*inserted_at) < cutoff)
            .map(|(e, _)| e.clone())
            .collect()
    }

    /// Remove events older than the configured TTL.
    ///
    /// Because the deque is ordered by insertion time, this pops from the
    /// front in O(k) where k is the number of expired events.
    pub fn prune_expired(&mut self) {
        let cutoff = Duration::from_secs(self.ttl_secs);
        let now = Instant::now();
        while self
            .events
            .front()
            .map_or(false, |(_, inserted_at)| {
                now.duration_since(*inserted_at) >= cutoff
            })
        {
            self.events.pop_front();
        }
    }

    /// Returns the number of buffered events (including potentially expired ones).
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns true if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}
