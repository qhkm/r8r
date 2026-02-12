# R8r Testing Strategy & Advanced Features

> Comprehensive edge cases, test cases, and n8n-inspired features for r8r.

## Part 1: Edge Cases & Test Cases

### 1.1 Workflow Parsing Edge Cases

| Test Case | Input | Expected Behavior |
|-----------|-------|-------------------|
| Empty workflow file | `""` | Error: "Empty workflow definition" |
| Invalid YAML syntax | `name: [broken` | Error: "Invalid YAML: ..." |
| Missing required `name` | `{nodes: []}` | Error: "Missing required field: name" |
| Missing required `nodes` | `{name: "test"}` | Error: "Missing required field: nodes" |
| Duplicate node IDs | Two nodes with `id: step1` | Error: "Duplicate node ID: step1" |
| Circular dependency | A→B→C→A | Error: "Circular dependency detected: A → B → C → A" |
| Non-existent dependency | `depends_on: [missing]` | Error: "Node 'x' depends on non-existent node: missing" |
| Empty nodes array | `nodes: []` | Warning or success (empty workflow) |
| Very long node ID | 1000+ char ID | Success (or configurable limit) |
| Special chars in node ID | `id: "step-1_v2.0"` | Success |
| Unicode in workflow name | `name: "工作流程"` | Success |
| Node ID starting with number | `id: "1step"` | Success (or require letter start) |
| Reserved keywords | `id: "input"` or `id: "output"` | Warning or success |

```rust
#[cfg(test)]
mod parsing_edge_cases {
    use super::*;

    #[test]
    fn test_empty_workflow() {
        let result = parse_workflow("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_circular_dependency_detection() {
        let yaml = r#"
name: circular
nodes:
  - id: a
    type: transform
    depends_on: [c]
  - id: b
    type: transform
    depends_on: [a]
  - id: c
    type: transform
    depends_on: [b]
"#;
        let workflow = parse_workflow(yaml).unwrap();
        let result = validate_workflow(&workflow);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("circular"));
    }

    #[test]
    fn test_duplicate_node_ids() {
        let yaml = r#"
name: dupe
nodes:
  - id: step1
    type: transform
  - id: step1
    type: http
"#;
        let result = parse_workflow(yaml);
        assert!(result.is_err());
    }
}
```

### 1.2 Execution Edge Cases

| Test Case | Scenario | Expected Behavior |
|-----------|----------|-------------------|
| Node timeout | Node takes longer than timeout | Error with partial results preserved |
| Node failure mid-workflow | Node 2 of 5 fails | Workflow fails, previous outputs preserved |
| Retry exhaustion | 3 retries, all fail | Final error with retry history |
| Concurrent executions | Same workflow x10 parallel | All complete independently |
| Large input payload | 10MB JSON input | Success or configurable limit error |
| Empty input | `{}` or `null` | Success with empty input |
| Deeply nested JSON | 100 levels deep | Success or depth limit |
| Output exceeds limit | Node returns 100MB | Error: "Output exceeds limit" |
| 100+ nodes workflow | Stress test | Success within reasonable time |
| Rapid sequential execution | 100 runs in 1 second | All complete, no race conditions |
| Workflow disabled | `enabled: false` | Error: "Workflow is disabled" |

```rust
#[cfg(test)]
mod execution_edge_cases {
    use super::*;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn test_node_timeout() {
        let workflow = create_workflow_with_slow_node(10); // 10 sec node
        let executor = Executor::new_with_timeout(Duration::from_secs(1));

        let result = executor.execute(&workflow, "test", "manual", json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));
    }

    #[tokio::test]
    async fn test_concurrent_executions() {
        let workflow = create_test_workflow();
        let executor = Arc::new(Executor::new());

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let exec = executor.clone();
                let wf = workflow.clone();
                tokio::spawn(async move {
                    exec.execute(&wf, "test", "manual", json!({"run": i})).await
                })
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(handles).await;
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[tokio::test]
    async fn test_large_input_payload() {
        let large_input = json!({
            "data": "x".repeat(10_000_000) // 10MB string
        });
        let result = executor.execute(&workflow, "test", "manual", large_input).await;
        // Should either succeed or fail gracefully
        assert!(result.is_ok() || result.unwrap_err().to_string().contains("size"));
    }
}
```

### 1.3 Transform Node Edge Cases

| Test Case | Expression | Expected Behavior |
|-----------|------------|-------------------|
| Invalid syntax | `2 + + 3` | Error: "Parse error: ..." |
| Division by zero | `10 / 0` | Error or Infinity |
| Undefined variable | `foo + 1` | Error: "Variable 'foo' not found" |
| Infinite loop | `loop { }` | Timeout error |
| Large array | `[1..1000000]` | Success or memory limit |
| Type coercion | `"5" + 3` | "53" or 8 (define behavior) |
| Null handling | `null + 1` | Error or null propagation |
| Non-serializable return | Return a function | Error: "Cannot serialize result" |
| Empty expression | `""` | Error: "Empty expression" |
| Multi-line expression | Complex script | Success |

```rust
#[cfg(test)]
mod transform_edge_cases {
    use super::*;

    #[tokio::test]
    async fn test_division_by_zero() {
        let node = TransformNode::new();
        let config = json!({"expression": "10 / 0"});
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await;
        // Rhai returns infinity for division by zero
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_undefined_variable() {
        let node = TransformNode::new();
        let config = json!({"expression": "undefined_var + 1"});
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_expression_timeout() {
        let node = TransformNode::new();
        // Rhai has built-in loop protection
        let config = json!({"expression": "let x = 0; while true { x += 1; }"});
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err()); // Should hit operation limit
    }
}
```

### 1.4 HTTP Node Edge Cases

| Test Case | Scenario | Expected Behavior |
|-----------|----------|-------------------|
| Connection timeout | Server unreachable | Error: "Connection timeout" |
| Read timeout | Server hangs | Error: "Read timeout" |
| DNS failure | Invalid domain | Error: "DNS resolution failed" |
| SSL error | Invalid certificate | Error: "SSL certificate error" |
| Redirect loop | 301 → 301 → ... | Error: "Too many redirects" |
| Large response | 100MB response | Truncate or stream |
| Binary response | Image/PDF | Base64 or raw bytes |
| Rate limited | 429 response | Retry with backoff or error |
| Server error | 500-599 | Return error with body |
| Empty response | 204 No Content | Success with null body |
| Invalid JSON | Non-JSON response | raw_response mode or error |
| Auth failure | 401/403 | Error with clear message |
| Request too large | 413 | Error: "Request entity too large" |

```rust
#[cfg(test)]
mod http_edge_cases {
    use super::*;
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;

    #[tokio::test]
    async fn test_http_timeout() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
            .mount(&mock_server)
            .await;

        let node = HttpNode::new();
        let config = json!({
            "url": mock_server.uri(),
            "timeout_seconds": 1
        });

        let result = node.execute(&config, &NodeContext::new("exec", "test")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));
    }

    #[tokio::test]
    async fn test_http_redirect_handling() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(301).insert_header("Location", "/new"))
            .mount(&mock_server)
            .await;

        let node = HttpNode::new();
        let config = json!({"url": mock_server.uri()});

        // Should follow redirects by default
        let result = node.execute(&config, &NodeContext::new("exec", "test")).await;
        // Behavior depends on implementation
    }

    #[tokio::test]
    async fn test_http_binary_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(vec![0x89, 0x50, 0x4E, 0x47]) // PNG header
                    .insert_header("Content-Type", "image/png")
            )
            .mount(&mock_server)
            .await;

        let node = HttpNode::new();
        let config = json!({
            "url": mock_server.uri(),
            "raw_response": true
        });

        let result = node.execute(&config, &NodeContext::new("exec", "test")).await;
        assert!(result.is_ok());
    }
}
```

### 1.5 Agent Node Edge Cases

| Test Case | Scenario | Expected Behavior |
|-----------|----------|-------------------|
| ZeptoClaw unavailable | Connection refused | Error: "Agent service unavailable" |
| LLM timeout | Model takes too long | Error with timeout message |
| Invalid JSON extraction | LLM returns prose | Fallback to full response or error |
| Token limit exceeded | Huge prompt | Error: "Token limit exceeded" |
| Empty LLM response | Model returns "" | Handle gracefully |
| LLM error | Model returns error | Propagate error clearly |
| Rate limited | Too many requests | Retry with backoff |
| Invalid model | Non-existent model | Error: "Model not found" |

### 1.6 Storage Edge Cases

| Test Case | Scenario | Expected Behavior |
|-----------|----------|-------------------|
| Database corruption | Invalid DB file | Error: "Database corrupted" |
| Concurrent writes | Multiple executors | Proper locking, no data loss |
| Disk full | No space left | Error: "Disk full" |
| Permission denied | Read-only directory | Error: "Permission denied" |
| Long execution history | 1M+ executions | Pagination, no memory issues |
| Query no results | Filter with no match | Empty array, not error |
| Unicode in data | 日本語 in workflow name | Proper encoding |

---

## Part 2: Advanced n8n-Inspired Features

Based on [n8n's feature set](https://n8n.io/features/) and [core nodes](https://docs.n8n.io/), here are advanced features to implement:

### 2.1 Flow Control Nodes

#### IF Node (Conditional Branching)
Route workflow based on conditions.

```yaml
- id: check-priority
  type: if
  config:
    conditions:
      - field: "{{ input.priority }}"
        operator: equals
        value: "high"
    true_branch: urgent-handler
    false_branch: normal-handler
```

**Implementation:**
```rust
pub struct IfNode;

#[derive(Debug, Deserialize)]
struct IfConfig {
    conditions: Vec<Condition>,
    true_branch: String,
    false_branch: String,
}

#[derive(Debug, Deserialize)]
struct Condition {
    field: String,
    operator: String,  // equals, not_equals, contains, gt, lt, gte, lte, regex
    value: Value,
}

#[async_trait]
impl Node for IfNode {
    fn node_type(&self) -> &str { "if" }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: IfConfig = serde_json::from_value(config.clone())?;
        let result = evaluate_conditions(&config.conditions, ctx)?;

        Ok(NodeResult::with_branch(
            json!({"condition_result": result}),
            if result { &config.true_branch } else { &config.false_branch }
        ))
    }
}
```

#### Switch Node (Multi-way Branching)
Route to multiple branches based on value matching.

```yaml
- id: route-by-type
  type: switch
  config:
    field: "{{ input.type }}"
    cases:
      - value: "order"
        branch: process-order
      - value: "refund"
        branch: process-refund
      - value: "inquiry"
        branch: process-inquiry
    default_branch: unknown-type
```

#### Merge Node
Combine outputs from multiple branches.

```yaml
- id: combine-results
  type: merge
  config:
    mode: append        # append | combine | zip
    wait_for: [branch-a, branch-b, branch-c]
```

**Modes:**
- `append`: Concatenate arrays
- `combine`: Merge objects (later overwrites)
- `zip`: Pair items from each branch

#### Split Node
Split array into individual items for parallel processing.

```yaml
- id: process-each-item
  type: split
  config:
    field: "{{ input.items }}"
    batch_size: 10  # Optional: process in batches
```

#### Aggregate Node
Combine multiple items back into one.

```yaml
- id: collect-results
  type: aggregate
  config:
    mode: array         # array | object | first | last
    key_field: id       # For object mode
    value_field: result
```

### 2.2 Data Manipulation Nodes

#### Filter Node
Filter array items based on conditions.

```yaml
- id: filter-active
  type: filter
  config:
    field: "{{ input.items }}"
    conditions:
      - field: status
        operator: equals
        value: "active"
```

#### Sort Node
Sort data by field(s).

```yaml
- id: sort-by-date
  type: sort
  config:
    field: "{{ input.items }}"
    by:
      - field: created_at
        order: desc
      - field: priority
        order: asc
```

#### Limit Node
Limit number of items.

```yaml
- id: top-10
  type: limit
  config:
    field: "{{ input.items }}"
    limit: 10
    offset: 0
```

#### Remove Duplicates Node
Deduplicate data.

```yaml
- id: unique-emails
  type: dedupe
  config:
    field: "{{ input.items }}"
    by: email
```

#### Summarize Node
Compute statistics.

```yaml
- id: calculate-stats
  type: summarize
  config:
    field: "{{ input.items }}"
    operations:
      - type: sum
        field: amount
        as: total
      - type: avg
        field: amount
        as: average
      - type: count
        as: count
      - type: min
        field: amount
        as: minimum
      - type: max
        field: amount
        as: maximum
```

### 2.3 Utility Nodes

#### Wait/Delay Node
Pause execution for specified duration.

```yaml
- id: rate-limit-pause
  type: wait
  config:
    duration_ms: 1000
    # OR
    until: "{{ input.scheduled_time }}"
```

#### Set Node
Set or modify data fields.

```yaml
- id: enrich-data
  type: set
  config:
    fields:
      - name: processed_at
        value: "{{ now() }}"
      - name: status
        value: "processed"
      - name: full_name
        value: "{{ input.first_name }} {{ input.last_name }}"
```

#### Code Node (Advanced Transform)
Execute custom code with more power than transform.

```yaml
- id: custom-logic
  type: code
  config:
    language: rhai  # or js, python (future)
    script: |
      let items = from_json(input);
      let filtered = items.filter(|x| x.active);
      let mapped = filtered.map(|x| #{
        id: x.id,
        name: x.name.to_upper()
      });
      to_json(mapped)
```

#### Crypto Node
Encryption, hashing, encoding.

```yaml
- id: hash-password
  type: crypto
  config:
    operation: hash     # hash | encrypt | decrypt | encode | decode
    algorithm: sha256   # sha256, sha512, md5, bcrypt, argon2
    input: "{{ input.password }}"
```

#### DateTime Node
Date/time manipulation.

```yaml
- id: format-date
  type: datetime
  config:
    operation: format   # format | parse | add | subtract | diff
    input: "{{ input.timestamp }}"
    format: "%Y-%m-%d %H:%M:%S"
    timezone: "UTC"
```

### 2.4 Advanced Trigger Nodes

#### Webhook Trigger
HTTP webhook to trigger workflows.

```yaml
triggers:
  - type: webhook
    config:
      path: /webhooks/my-workflow
      method: POST
      auth:
        type: bearer
        token_env: WEBHOOK_SECRET
```

#### Schedule/Cron Trigger
Time-based triggers.

```yaml
triggers:
  - type: schedule
    config:
      cron: "0 9 * * MON-FRI"  # 9 AM weekdays
      timezone: "America/New_York"
```

#### Event Trigger
Trigger on external events.

```yaml
triggers:
  - type: event
    config:
      source: redis
      channel: orders
      pattern: "order.created.*"
```

### 2.5 Workflow Features

#### Sub-workflow Node
Call another workflow as a node.

```yaml
- id: process-payment
  type: workflow
  config:
    name: payment-processor
    inputs:
      amount: "{{ input.total }}"
      currency: "{{ input.currency }}"
    wait: true
```

#### Error Workflow
Separate workflow for error handling.

```yaml
name: main-workflow
on_error:
  workflow: error-handler
  inputs:
    original_workflow: "{{ workflow.name }}"
    error: "{{ error.message }}"
    node: "{{ error.node_id }}"
```

#### Workflow Variables
Global variables accessible throughout workflow.

```yaml
name: my-workflow
variables:
  api_base: "https://api.example.com"
  retry_count: 3

nodes:
  - id: fetch-data
    type: http
    config:
      url: "{{ vars.api_base }}/data"
```

#### Pinned Data (Development Mode)
Save test data for development.

```yaml
nodes:
  - id: fetch-users
    type: http
    config:
      url: "https://api.example.com/users"
    pinned_data:  # Used in dev mode instead of actual request
      - {id: 1, name: "Test User"}
      - {id: 2, name: "Another User"}
```

### 2.6 Execution Features

#### Execution History with Search
Rich filtering and search for past executions.

```rust
pub struct ExecutionQuery {
    workflow_name: Option<String>,
    status: Option<ExecutionStatus>,
    started_after: Option<DateTime<Utc>>,
    started_before: Option<DateTime<Utc>>,
    trigger_type: Option<String>,
    search: Option<String>,  // Search in input/output
    limit: usize,
    offset: usize,
}
```

#### Workflow Versioning
Track changes over time.

```rust
pub struct WorkflowVersion {
    id: String,
    workflow_id: String,
    version: u32,
    definition: String,
    created_at: DateTime<Utc>,
    created_by: Option<String>,
    changelog: Option<String>,
}
```

#### Execution Replay
Re-run a workflow with same or modified inputs.

```rust
impl Executor {
    pub async fn replay(&self, execution_id: &str, modified_input: Option<Value>) -> Result<Execution> {
        let original = self.storage.get_execution(execution_id).await?;
        let input = modified_input.unwrap_or(original.input);
        self.execute(&workflow, &original.workflow_id, "replay", input).await
    }
}
```

---

## Part 3: Implementation Priority

### Phase 1: Core Flow Control (High Priority)
1. ✅ Transform node (done)
2. ✅ HTTP node (done)
3. ✅ Agent node (done)
4. **IF node** - Conditional branching
5. **Switch node** - Multi-way branching
6. **Merge node** - Combine branches

### Phase 2: Data Manipulation (Medium Priority)
7. **Filter node**
8. **Sort node**
9. **Limit node**
10. **Set node**
11. **Aggregate node**
12. **Split node**

### Phase 3: Triggers (Medium Priority)
13. **Webhook trigger**
14. **Schedule/Cron trigger**
15. **Event trigger** (Redis, etc.)

### Phase 4: Advanced (Lower Priority)
16. Sub-workflow node
17. Error workflow
18. Workflow versioning
19. Execution replay
20. Crypto node
21. DateTime node

---

## Sources

- [n8n Features](https://n8n.io/features/)
- [n8n Merge Node](https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.merge/)
- [n8n Aggregate Node](https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.aggregate/)
- [n8n IF Node](https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.if/)
- [n8n Switch Node](https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.switch/)
- [n8n Merging Data](https://docs.n8n.io/flow-logic/merging/)
- [n8n Splitting with Conditionals](https://docs.n8n.io/flow-logic/splitting/)
- [Aggregate vs Merge vs Split Guide](https://www.c-sharpcorner.com/article/aggregate-vs-merge-vs-split-in-n8n-a-complete-guide-with-examples/)
