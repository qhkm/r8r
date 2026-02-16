---
title: Connections & Data Flow
description: How data flows between nodes in r8r workflows.
---

r8r uses a DAG (Directed Acyclic Graph) model for connecting nodes. Nodes declare their dependencies explicitly, and data flows through template expressions.

## depends_on

The `depends_on` field creates execution order between nodes:

```yaml
nodes:
  - id: step-a
    type: http
    config:
      url: https://api.example.com/a

  - id: step-b
    type: http
    config:
      url: https://api.example.com/b

  - id: step-c
    type: transform
    config:
      expression: |
        #{
          a: nodes.`step-a`,
          b: nodes.`step-b`
        }
    depends_on: [step-a, step-b]
```

In this example:
- `step-a` and `step-b` have no dependencies and run **in parallel**
- `step-c` waits for both to complete before executing

## Accessing Node Output

Use the `{{ nodes.<id>.<field> }}` template syntax to reference output from upstream nodes:

```yaml
- id: fetch
  type: http
  config:
    url: https://api.example.com/users

- id: process
  type: transform
  config:
    expression: "nodes.fetch.body.users.len()"
  depends_on: [fetch]

- id: log
  type: debug
  config:
    message: "Found {{ nodes.process }} users"
  depends_on: [process]
```

## Environment Variables

Access environment variables (restricted to `R8R_*` prefix or allowlisted variables):

```yaml
config:
  url: "https://api.example.com"
  headers:
    Authorization: "Bearer {{ env.R8R_API_TOKEN }}"
```

To allow additional env vars, set `R8R_ALLOWED_ENV_VARS`:

```bash
export R8R_ALLOWED_ENV_VARS="MY_TOKEN, CUSTOM_KEY"
```

## Iteration with for_each

The `for_each` field iterates over an array, executing the node once per item:

```yaml
- id: fetch-list
  type: http
  config:
    url: https://api.example.com/items

- id: process-each
  type: http
  config:
    url: "https://api.example.com/items/{{ item.id }}"
    method: PUT
    body: '{{ item }}'
  depends_on: [fetch-list]
  for_each: "nodes.fetch-list.items"
```

The current item is available as `item` within the node's config.

Concurrency for `for_each` is controlled by the workflow's `settings.max_concurrency` (default: 10).

## Conditional Execution

Use `condition` to skip nodes based on runtime values:

```yaml
- id: check
  type: transform
  config:
    expression: "input.items.len()"
  depends_on: [fetch]

- id: alert
  type: email
  config:
    to: admin@example.com
    subject: "No items found"
  depends_on: [check]
  condition: "nodes.check == 0"
```

If the condition evaluates to `false`, the node is skipped and downstream nodes receive no output from it.

## Inter-Workflow Dependencies

Workflows can depend on other workflows using `depends_on_workflows`:

```yaml
name: downstream-workflow
depends_on_workflows:
  - workflow: upstream-workflow
```

Use `r8r workflows dag <name>` to visualize workflow dependency graphs.
