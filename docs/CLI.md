# r8r CLI Reference

## Exit Codes

| Code | Meaning |
|------|---------|
| `0`  | Success |
| `1`  | Error (see stderr for details) |
| `42` | Execution requires human approval (non-interactive mode) |

### Exit 42 — Non-Interactive Approval Guard

When `r8r run` is used in a non-interactive context (e.g. CI, piped stdin, scripts) and the workflow reaches an `approval` node, the process exits with code **42** instead of blocking.

This lets orchestrators detect approval-required executions and handle them:

~~~bash
r8r run my-workflow
EXIT=$?
if [ $EXIT -eq 42 ]; then
  echo "Workflow is waiting for human approval"
  # notify team, open approval UI, etc.
fi
~~~

---

## `r8r prompt` — Generate or Refine Workflows

~~~
r8r prompt [FLAGS] <description...>
r8r prompt --patch <workflow-name> [FLAGS] <description...>
~~~

### Flags

| Flag | Description |
|------|-------------|
| `--patch <name>` | Refine an existing workflow instead of generating a new one |
| `--emit` | Print generated YAML to stdout and exit (no interactive loop) |
| `--dry-run` | Generate and validate YAML but do not save to storage |
| `--yes` | Skip the save confirmation prompt (non-interactive mode) |
| `--json` | Output result as JSON `{"name": "...", "yaml": "..."}` |

### Examples

~~~bash
# Generate a new workflow interactively
r8r prompt fetch HN top stories and post to Slack

# Emit YAML to stdout for piping or inspection
r8r prompt --emit summarize PDF and send email > workflow.yaml

# Refine an existing workflow non-interactively
r8r prompt --patch hn-to-slack --yes add retry with exponential backoff

# Generate and output as JSON (for agent consumption)
r8r prompt --json --yes create daily report workflow
~~~

---

## `r8r run` — Execute a Workflow

~~~
r8r run <workflow-name> [--yes] [--json]
~~~

In interactive mode, shows a side-effect summary before execution.
In non-interactive mode (`--yes` or piped stdin), skips the confirmation.
