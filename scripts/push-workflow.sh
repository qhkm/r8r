#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Push a workflow YAML file to a running r8r cloud instance.

Usage:
  scripts/push-workflow.sh --server <url> --file <workflow.yaml> [options]

Required:
  --server <url>         Base URL for r8r API (e.g. https://r8r.example.com)
  --file <path>          Workflow YAML file to push

Options:
  --name <name>          Override workflow name (defaults to top-level "name:" in YAML)
  --api-key <key>        API key for Authorization header (or set R8R_API_KEY)
  --enabled <bool>       true/false (default: true)
  --execute              Execute workflow immediately after push
  --input <path>         JSON input file for --execute (default: {})
  --wait <bool>          true/false wait for completion when executing (default: true)
  --skip-validate        Skip local "r8r workflows validate" check
  -h, --help             Show help

Examples:
  scripts/push-workflow.sh --server https://r8r.example.com --file workflows/lead.yaml
  scripts/push-workflow.sh --server https://r8r.example.com --file out.yaml --execute --input input.json
EOF
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: missing required command: $1" >&2
    exit 1
  fi
}

require_bool() {
  case "$1" in
    true|false) ;;
    *)
      echo "error: expected boolean true|false, got: $1" >&2
      exit 1
      ;;
  esac
}

extract_name_from_yaml() {
  local file="$1"
  awk '
    /^name:[[:space:]]*/ {
      sub(/^name:[[:space:]]*/, "", $0)
      gsub(/^[\"\047]|[\"\047]$/, "", $0)
      print $0
      exit
    }
  ' "$file"
}

SERVER_URL="${R8R_SERVER_URL:-}"
WORKFLOW_FILE=""
WORKFLOW_NAME=""
API_KEY="${R8R_API_KEY:-}"
ENABLED="true"
EXECUTE_AFTER_PUSH="false"
INPUT_FILE=""
WAIT_FOR_RESULT="true"
SKIP_VALIDATE="false"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --server)
      SERVER_URL="$2"
      shift 2
      ;;
    --file)
      WORKFLOW_FILE="$2"
      shift 2
      ;;
    --name)
      WORKFLOW_NAME="$2"
      shift 2
      ;;
    --api-key)
      API_KEY="$2"
      shift 2
      ;;
    --enabled)
      ENABLED="$2"
      shift 2
      ;;
    --execute)
      EXECUTE_AFTER_PUSH="true"
      shift
      ;;
    --input)
      INPUT_FILE="$2"
      shift 2
      ;;
    --wait)
      WAIT_FOR_RESULT="$2"
      shift 2
      ;;
    --skip-validate)
      SKIP_VALIDATE="true"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ -z "$SERVER_URL" || -z "$WORKFLOW_FILE" ]]; then
  echo "error: --server and --file are required" >&2
  usage
  exit 1
fi

if [[ ! -f "$WORKFLOW_FILE" ]]; then
  echo "error: workflow file not found: $WORKFLOW_FILE" >&2
  exit 1
fi

require_bool "$ENABLED"
require_bool "$WAIT_FOR_RESULT"
require_cmd curl
require_cmd jq

if [[ -z "$WORKFLOW_NAME" ]]; then
  WORKFLOW_NAME="$(extract_name_from_yaml "$WORKFLOW_FILE" || true)"
fi

if [[ -z "$WORKFLOW_NAME" ]]; then
  echo "error: could not determine workflow name from YAML; pass --name" >&2
  exit 1
fi

if [[ "$SKIP_VALIDATE" != "true" ]]; then
  if command -v r8r >/dev/null 2>&1; then
    r8r workflows validate "$WORKFLOW_FILE"
  else
    echo "warning: r8r CLI not found; skipping local validation" >&2
  fi
fi

SERVER_URL="${SERVER_URL%/}"
tmp_response="$(mktemp)"
trap 'rm -f "$tmp_response"' EXIT

create_payload="$(jq -n \
  --arg name "$WORKFLOW_NAME" \
  --rawfile definition "$WORKFLOW_FILE" \
  --argjson enabled "$ENABLED" \
  '{name: $name, definition: $definition, enabled: $enabled}')"

headers=(-H "Content-Type: application/json")
if [[ -n "$API_KEY" ]]; then
  headers+=(-H "Authorization: Bearer ${API_KEY}")
fi

status_code="$(
  curl -sS -o "$tmp_response" -w "%{http_code}" \
    -X POST "${SERVER_URL}/api/workflows" \
    "${headers[@]}" \
    -d "$create_payload"
)"

if [[ "$status_code" -lt 200 || "$status_code" -gt 299 ]]; then
  echo "error: failed to push workflow (HTTP $status_code)" >&2
  cat "$tmp_response" >&2
  exit 1
fi

echo "workflow pushed: ${WORKFLOW_NAME}"
echo "server: ${SERVER_URL}"
cat "$tmp_response"
echo

if [[ "$EXECUTE_AFTER_PUSH" == "true" ]]; then
  if [[ -n "$INPUT_FILE" ]]; then
    if [[ ! -f "$INPUT_FILE" ]]; then
      echo "error: input file not found: $INPUT_FILE" >&2
      exit 1
    fi
    jq -e . "$INPUT_FILE" >/dev/null
    execute_payload="$(jq -n \
      --slurpfile input "$INPUT_FILE" \
      --argjson wait "$WAIT_FOR_RESULT" \
      '{input: $input[0], wait: $wait}')"
  else
    execute_payload="$(jq -n \
      --argjson wait "$WAIT_FOR_RESULT" \
      '{input: {}, wait: $wait}')"
  fi

  status_code="$(
    curl -sS -o "$tmp_response" -w "%{http_code}" \
      -X POST "${SERVER_URL}/api/workflows/${WORKFLOW_NAME}/execute" \
      "${headers[@]}" \
      -d "$execute_payload"
  )"

  if [[ "$status_code" -lt 200 || "$status_code" -gt 299 ]]; then
    echo "error: workflow execution failed (HTTP $status_code)" >&2
    cat "$tmp_response" >&2
    exit 1
  fi

  echo "execution response:"
  cat "$tmp_response"
  echo
fi
