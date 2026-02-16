---
title: Quick Start
description: Create and run your first r8r workflow in minutes.
---

## Create a Workflow

Create a file called `hello.yaml`:

```yaml
name: hello-world
nodes:
  - id: greet
    type: transform
    config:
      expression: '"Hello, " + (input.name ?? "World") + "!"'
```

## Start the Server

```bash
r8r server --workflows .
```

This loads all `.yaml` workflow files from the current directory and starts the API server on port 8080.

## Execute the Workflow

```bash
curl -X POST http://localhost:8080/api/workflows/hello-world/execute \
  -H "Content-Type: application/json" \
  -d '{"input": {"name": "Agent"}}'
```

Or use the CLI:

```bash
r8r workflows run hello-world -p name=Agent
```

## A More Complete Example

Create `data-pipeline.yaml`:

```yaml
name: data-pipeline
description: Fetch, transform, and notify

triggers:
  - type: cron
    schedule: "*/30 * * * *"
  - type: manual

nodes:
  - id: fetch
    type: http
    config:
      url: https://api.example.com/data
      method: GET
      headers:
        Authorization: "Bearer {{ env.R8R_API_TOKEN }}"
    retry:
      max_attempts: 3
      backoff: exponential

  - id: process
    type: transform
    config:
      expression: |
        let data = input;
        data.items.filter(|item| item.status == "active")
    depends_on: [fetch]

  - id: log-result
    type: debug
    config:
      message: "Processed {{ nodes.process.length }} items"
    depends_on: [process]
```

## Using Templates

r8r includes built-in workflow templates:

```bash
# List available templates
r8r templates list

# Show template details
r8r templates show api-monitor

# Create a workflow from a template
r8r templates use api-monitor \
  --var url=https://api.example.com/health \
  --var interval="*/5 * * * *" \
  -o monitor.yaml
```

## Development Mode

Use `dev` mode for hot-reload during development:

```bash
r8r dev ./my-workflow.yaml
```

This watches the file for changes and validates automatically on save.

## Validate Workflows

Check a workflow file for errors without running it:

```bash
r8r workflows validate hello.yaml
```

## Next Steps

- Learn about [Workflows](/concepts/workflows) in depth
- Explore [Node Types](/reference/node-types)
- Set up [API Integration](/guides/api-integration)
