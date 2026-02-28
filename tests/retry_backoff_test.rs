//! Tests for retry backoff cap (max_delay_seconds) and jitter deserialization.

use r8r::workflow::{parse_workflow, BackoffType};

#[test]
fn test_backoff_cap_deserialization() {
    let yaml = r#"
name: retry-test
nodes:
  - id: step1
    type: http
    config:
      url: "https://example.com"
    retry:
      max_attempts: 5
      delay_seconds: 2
      backoff: exponential
      max_delay_seconds: 60
      jitter: true
"#;
    let workflow = parse_workflow(yaml).unwrap();
    let node = &workflow.nodes[0];
    let retry = node.retry.as_ref().unwrap();

    assert_eq!(retry.max_attempts, 5);
    assert_eq!(retry.delay_seconds, 2);
    assert!(matches!(retry.backoff, BackoffType::Exponential));
    assert_eq!(retry.max_delay_seconds, Some(60));
    assert!(retry.jitter);
}

#[test]
fn test_backoff_defaults_when_omitted() {
    let yaml = r#"
name: retry-defaults
nodes:
  - id: step1
    type: http
    config:
      url: "https://example.com"
    retry:
      max_attempts: 3
      delay_seconds: 1
      backoff: fixed
"#;
    let workflow = parse_workflow(yaml).unwrap();
    let node = &workflow.nodes[0];
    let retry = node.retry.as_ref().unwrap();

    assert_eq!(retry.max_delay_seconds, None);
    assert!(!retry.jitter);
}

#[test]
fn test_backoff_cap_zero_means_no_cap() {
    let yaml = r#"
name: retry-zero-cap
nodes:
  - id: step1
    type: http
    config:
      url: "https://example.com"
    retry:
      max_attempts: 3
      delay_seconds: 10
      backoff: exponential
      max_delay_seconds: 0
"#;
    let workflow = parse_workflow(yaml).unwrap();
    let node = &workflow.nodes[0];
    let retry = node.retry.as_ref().unwrap();

    // max_delay_seconds: 0 should be treated as "no cap"
    assert_eq!(retry.max_delay_seconds, Some(0));
}
