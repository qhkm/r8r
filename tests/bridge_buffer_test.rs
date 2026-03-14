/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Tests for the bridge replay buffer (TTL, ack tracking, capacity limits).

use r8r::bridge::buffer::ReplayBuffer;
use r8r::bridge::events::{BridgeEvent, BridgeEventEnvelope};

fn make_event(id_suffix: &str) -> BridgeEventEnvelope {
    let mut env = BridgeEventEnvelope::new(BridgeEvent::HealthPing, None);
    env.id = format!("evt_{}", id_suffix);
    env
}

#[test]
fn test_push_and_drain_unacked() {
    let mut buf = ReplayBuffer::new(256, 300);
    buf.push(make_event("a"));
    buf.push(make_event("b"));

    let events = buf.unacked_events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].id, "evt_a");
    assert_eq!(events[1].id, "evt_b");
}

#[test]
fn test_ack_removes_event() {
    let mut buf = ReplayBuffer::new(256, 300);
    buf.push(make_event("a"));
    buf.push(make_event("b"));
    buf.push(make_event("c"));

    buf.ack("evt_b");

    let events = buf.unacked_events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].id, "evt_a");
    assert_eq!(events[1].id, "evt_c");
}

#[test]
fn test_max_events_cap() {
    let mut buf = ReplayBuffer::new(3, 300);
    for suffix in &["a", "b", "c", "d", "e"] {
        buf.push(make_event(suffix));
    }

    assert_eq!(buf.len(), 3);
    let events = buf.unacked_events();
    assert_eq!(events[0].id, "evt_c");
    assert_eq!(events[1].id, "evt_d");
    assert_eq!(events[2].id, "evt_e");
}

#[test]
fn test_expired_events_pruned() {
    let mut buf = ReplayBuffer::new(256, 0);
    buf.push(make_event("x"));

    std::thread::sleep(std::time::Duration::from_millis(10));

    buf.prune_expired();
    assert_eq!(buf.len(), 0);
}

#[test]
fn test_ack_nonexistent_is_noop() {
    let mut buf = ReplayBuffer::new(256, 300);
    buf.push(make_event("a"));

    buf.ack("evt_does_not_exist");

    assert_eq!(buf.len(), 1);
    assert_eq!(buf.unacked_events()[0].id, "evt_a");
}
