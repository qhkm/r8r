---
title: Custom Logic with Rhai
description: Using Rhai expressions for data transformation and custom logic in r8r.
---

r8r uses the [Rhai](https://rhai.rs) scripting engine for data transformation and conditional logic. Rhai expressions power the `transform` node, `condition` fields, and template rendering.

## Transform Node

The `transform` node evaluates a Rhai expression and returns the result:

```yaml
- id: process
  type: transform
  config:
    expression: |
      let data = input;
      let active = data.items.filter(|item| item.status == "active");
      #{
        total: data.items.len(),
        active: active.len(),
        items: active
      }
```

The `input` variable contains the data from upstream nodes (or workflow input for the first node).

## Rhai Basics

### Data Types

```rhai
// Numbers
let count = 42;
let ratio = 3.14;

// Strings
let name = "r8r";
let greeting = `Hello, ${name}!`;  // String interpolation

// Arrays
let items = [1, 2, 3];
let first = items[0];

// Objects (maps)
let obj = #{
  key: "value",
  nested: #{
    inner: true
  }
};
```

### Array Operations

```rhai
let items = input.data;

// Filter
let active = items.filter(|item| item.active);

// Map
let names = items.map(|item| item.name);

// Find
let first = items.find(|item| item.id == target_id);

// Reduce
let total = items.reduce(|sum, item| sum + item.amount, 0);

// Sort
items.sort(|a, b| a.name < b.name);

// Length
let count = items.len();
```

### Conditional Logic

```rhai
if input.status == "ok" {
  input.data
} else {
  throw "Unexpected status: " + input.status
}
```

### Error Handling

Use `throw` to raise errors:

```rhai
if input.amount <= 0 {
  throw "Invalid amount: " + input.amount.to_string()
}
input
```

## Conditions

The `condition` field on any node uses Rhai expressions:

```yaml
- id: send-alert
  type: email
  config:
    to: admin@example.com
    subject: "Alert: High error rate"
  depends_on: [check]
  condition: "nodes.check.error_rate > 0.05"
```

## Template Expressions

Template strings (`{{ }}`) are evaluated in a simpler context:

```yaml
config:
  url: "https://api.example.com/users/{{ nodes.fetch.user_id }}"
  headers:
    Authorization: "Bearer {{ env.R8R_API_TOKEN }}"
```

Templates support accessing:
- `nodes.<id>.<field>` -- Output from upstream nodes
- `env.<VAR>` -- Environment variables (restricted to `R8R_*` or allowlisted)
- `input.<field>` -- Workflow input data

## Practical Examples

### Data Enrichment

```yaml
- id: enrich
  type: transform
  config:
    expression: |
      let user = nodes.fetch;
      #{
        name: user.name,
        email: user.email,
        tier: if user.total_spend > 1000 { "gold" } else { "standard" },
        joined_days: (timestamp() - user.created_at) / 86400
      }
  depends_on: [fetch]
```

### Validation

```yaml
- id: validate
  type: transform
  config:
    expression: |
      let errors = [];
      if input.email == () || input.email == "" {
        errors.push("Email is required");
      }
      if input.amount <= 0 {
        errors.push("Amount must be positive");
      }
      if errors.len() > 0 {
        throw "Validation failed: " + errors.join(", ")
      }
      input
```
