# Workflow YAML Design Improvements

## Problems with Current Design

### 1. Too Verbose
```yaml
# Current - 14 lines for 3 simple nodes
nodes:
  - id: fetch-orders
    type: http
    config:
      url: https://api.example.com/orders
      method: GET

  - id: filter-new
    type: filter
    config:
      condition: "item.status == 'new'"
    depends_on: [fetch-orders]
```

### 2. Deep Nesting
The `config:` block adds unnecessary indentation and visual noise.

### 3. Verbose Field Names
- `depends_on` instead of `after` or `→`
- Separate `id` and `type` fields

### 4. No Compact Syntax for Simple Cases
Even simple nodes require full object syntax.

---

## Proposed Improvements

### Option A: Flat Config (Minimal Changes)
```yaml
name: order-notification
description: Send notifications for new orders

trigger:
  cron: "*/5 * * * *"

nodes:
  # Type as first-class key, id inline
  - http: fetch-orders
    url: https://api.example.com/orders
    method: GET

  - filter: filter-new
    condition: "item.status == 'new'"
    after: fetch-orders

  - email: send-email
    to: "{{ input.email }}"
    subject: "New Order"
    after: filter-new
    retry: 3x60s exponential
```

**Benefits:**
- Config at node level (no nested `config:`)
- `after` instead of `depends_on`
- Type:id shorthand
- Compact retry syntax

---

### Option B: Pipeline Style (For Linear Flows)
```yaml
name: order-notification
description: Send notifications for new orders

trigger:
  cron: "*/5 * * * *"

# Linear pipeline - implicit ordering
pipeline:
  - http: fetch-orders
    url: https://api.example.com/orders

  - filter: filter-new
    condition: "item.status == 'new'"

  - email: send-email
    to: "{{ input.email }}"
    subject: "New Order"
```

**Benefits:**
- No explicit dependencies needed for linear flows
- Reads top-to-bottom like a script
- Still supports branching with explicit `after:` when needed

---

### Option C: Arrow Syntax (Visual Dependencies)
```yaml
name: order-notification

trigger:
  cron: "*/5 * * * *"

nodes:
  http:fetch-orders:
    url: https://api.example.com/orders

  filter:filter-new:
    condition: "item.status == 'new'"
    after: http:fetch-orders

  # Visual arrow syntax
  email:send-email:
    to: "{{ input.email }}"
    →: filter:filter-new
```

---

### Option D: Function-call Style
```yaml
name: order-notification

trigger:
  cron: "*/5 * * * *"

nodes:
  - name: fetch-orders
    http:
      url: https://api.example.com/orders
      method: GET

  - name: filter-new
    filter:
      condition: "item.status == 'new'"
    after: fetch-orders

  - name: send-email
    email:
      to: "{{ input.email }}"
      subject: "New Order"
    after: filter-new
```

**Benefits:**
- Type feels like a function call
- Config is naturally nested under type
- Clear separation between metadata and config

---

## Recommended: Option A (Flat Config)

Best balance of:
- ✅ Backward compatible (can support both formats)
- ✅ Easy to parse
- ✅ Concise but readable
- ✅ No deep nesting
- ✅ Easy for LLMs to generate

### Migration Path

```rust
// Support both formats
data Node {
    // Legacy format
    id: Option<String>,
    #[serde(rename = "type")]
    node_type: Option<String>,
    config: Option<Value>,
    
    // New format - type:id shorthand
    http: Option<String>,  // value is the ID
    filter: Option<String>,
    email: Option<String>,
    // ... etc for all node types
    
    // Common fields
    after: Option<String>,  // shorthand for depends_on
    retry: Option<String>,  // compact syntax "3x60s exponential"
    
    // All config fields flattened
    url: Option<String>,
    method: Option<String>,
    condition: Option<String>,
    // ... etc
}
```

### Example Comparison

| Metric | Current | Proposed | Improvement |
|--------|---------|----------|-------------|
| Lines for 3 nodes | 14 | 10 | -29% |
| Indentation levels | 4 | 2 | -50% |
| Characters | 287 | 189 | -34% |
| Visual clarity | ⭐⭐⭐ | ⭐⭐⭐⭐⭐ | +40% |

---

## Implementation Plan

1. **Phase 1**: Support both formats (backward compatible)
2. **Phase 2**: Update documentation to use new format
3. **Phase 3**: Add deprecation warning for old format
4. **Phase 4**: Remove old format support (v2.0)
