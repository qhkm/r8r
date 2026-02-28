#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  cat <<'EOF'
Deploy r8r directly on the current VPS using Docker Compose (no SSH hop).
The r8r container includes API + embedded UI (/ and /editor).

Usage:
  scripts/deploy-on-vps.sh --image <image> --monitor-token <token> [options]

Required:
  --image <image>            Container image (e.g. ghcr.io/acme/r8r:abcdef)
  --monitor-token <token>    R8R_MONITOR_TOKEN for /api/monitor auth

Optional:
  --deploy-dir <path>        Deployment directory (default: /opt/r8r)
  --r8r-port <port>          Exposed API/UI port (default: 8080)
  --bootstrap                Install Docker + Compose on Debian/Ubuntu first
  --skip-health-check        Skip local API/UI health checks after deploy
  --api-key <key>            R8R_API_KEY to protect /api/*
  --cors-origins <origins>   Comma-separated CORS origins
  --rust-log <level>         RUST_LOG (default: r8r=info)
  --volume-name <name>       Docker volume name (default: r8r-data)
  --registry-username <u>    Container registry username (optional)
  --registry-token <token>   Container registry token/password (optional)
  -h, --help                 Show help
EOF
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: missing required command: $1" >&2
    exit 1
  fi
}

require_no_newline() {
  local value="$1"
  local name="$2"
  if [[ "$value" == *$'\n'* ]]; then
    echo "error: ${name} must not contain newline characters" >&2
    exit 1
  fi
}

bootstrap_docker_if_needed() {
  if command -v docker >/dev/null 2>&1 && (docker compose version >/dev/null 2>&1 || command -v docker-compose >/dev/null 2>&1); then
    echo "docker + compose already installed"
    return
  fi

  if ! command -v sudo >/dev/null 2>&1; then
    echo "error: --bootstrap requires sudo" >&2
    exit 1
  fi

  if [[ ! -r /etc/os-release ]]; then
    echo "error: cannot detect OS (/etc/os-release missing)" >&2
    exit 1
  fi

  . /etc/os-release

  case "${ID:-}" in
    ubuntu|debian)
      sudo apt-get update
      sudo apt-get install -y ca-certificates curl gnupg
      sudo install -m 0755 -d /etc/apt/keyrings

      if [[ ! -f /etc/apt/keyrings/docker.asc ]]; then
        curl -fsSL "https://download.docker.com/linux/${ID}/gpg" | sudo gpg --dearmor -o /etc/apt/keyrings/docker.asc
      fi
      sudo chmod a+r /etc/apt/keyrings/docker.asc

      arch="$(dpkg --print-architecture)"
      if [[ -z "${VERSION_CODENAME:-}" ]]; then
        echo "error: VERSION_CODENAME missing in /etc/os-release" >&2
        exit 1
      fi

      echo "deb [arch=${arch} signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/${ID} ${VERSION_CODENAME} stable" | \
        sudo tee /etc/apt/sources.list.d/docker.list >/dev/null

      sudo apt-get update
      sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
      sudo systemctl enable --now docker
      ;;
    *)
      echo "error: --bootstrap supports ubuntu/debian only. Install Docker + Compose manually." >&2
      exit 1
      ;;
  esac

  if id -nG "$(whoami)" | grep -qw docker; then
    echo "current user already in docker group"
  else
    sudo usermod -aG docker "$(whoami)"
    echo "notice: added $(whoami) to docker group; new login session may be required"
  fi
}

IMAGE=""
MONITOR_TOKEN=""
DEPLOY_DIR="/opt/r8r"
R8R_PORT="8080"
BOOTSTRAP="false"
SKIP_HEALTH_CHECK="false"
API_KEY=""
CORS_ORIGINS=""
RUST_LOG="r8r=info"
VOLUME_NAME="r8r-data"
REGISTRY_USERNAME=""
REGISTRY_TOKEN=""
COMPOSE_SOURCE="${script_dir}/../deploy/docker-compose.cloud.yml"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --image)
      IMAGE="$2"
      shift 2
      ;;
    --monitor-token)
      MONITOR_TOKEN="$2"
      shift 2
      ;;
    --deploy-dir)
      DEPLOY_DIR="$2"
      shift 2
      ;;
    --r8r-port)
      R8R_PORT="$2"
      shift 2
      ;;
    --bootstrap)
      BOOTSTRAP="true"
      shift
      ;;
    --skip-health-check)
      SKIP_HEALTH_CHECK="true"
      shift
      ;;
    --api-key)
      API_KEY="$2"
      shift 2
      ;;
    --cors-origins)
      CORS_ORIGINS="$2"
      shift 2
      ;;
    --rust-log)
      RUST_LOG="$2"
      shift 2
      ;;
    --volume-name)
      VOLUME_NAME="$2"
      shift 2
      ;;
    --registry-username)
      REGISTRY_USERNAME="$2"
      shift 2
      ;;
    --registry-token)
      REGISTRY_TOKEN="$2"
      shift 2
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

if [[ -z "$IMAGE" || -z "$MONITOR_TOKEN" ]]; then
  echo "error: missing required arguments" >&2
  usage
  exit 1
fi

if [[ ! -f "$COMPOSE_SOURCE" ]]; then
  echo "error: compose file not found: $COMPOSE_SOURCE" >&2
  exit 1
fi

if [[ -n "$REGISTRY_USERNAME" && -z "$REGISTRY_TOKEN" ]]; then
  echo "error: --registry-token is required when --registry-username is set" >&2
  exit 1
fi
if [[ -z "$REGISTRY_USERNAME" && -n "$REGISTRY_TOKEN" ]]; then
  echo "error: --registry-username is required when --registry-token is set" >&2
  exit 1
fi

require_no_newline "$MONITOR_TOKEN" "monitor token"
require_no_newline "$API_KEY" "api key"
require_no_newline "$REGISTRY_TOKEN" "registry token"

require_cmd cp
require_cmd mkdir

if [[ "$BOOTSTRAP" == "true" ]]; then
  echo "bootstrap mode: installing Docker + Compose"
  bootstrap_docker_if_needed
fi

if docker info >/dev/null 2>&1; then
  docker_cmd=(docker)
elif command -v sudo >/dev/null 2>&1 && sudo -n docker info >/dev/null 2>&1; then
  docker_cmd=(sudo docker)
else
  echo "error: docker daemon is not accessible for user $(whoami)" >&2
  echo "hint: use --bootstrap or ensure user has docker permissions" >&2
  exit 1
fi

if "${docker_cmd[@]}" compose version >/dev/null 2>&1; then
  compose_cmd=("${docker_cmd[@]}" compose)
elif command -v docker-compose >/dev/null 2>&1; then
  if docker-compose version >/dev/null 2>&1; then
    compose_cmd=(docker-compose)
  elif command -v sudo >/dev/null 2>&1 && sudo -n docker-compose version >/dev/null 2>&1; then
    compose_cmd=(sudo docker-compose)
  else
    echo "error: docker-compose exists but is not executable for current user" >&2
    exit 1
  fi
else
  echo "error: docker compose is required" >&2
  exit 1
fi

use_sudo_files="false"
if mkdir -p "$DEPLOY_DIR" 2>/dev/null; then
  :
elif command -v sudo >/dev/null 2>&1; then
  sudo mkdir -p "$DEPLOY_DIR"
  use_sudo_files="true"
else
  echo "error: cannot create deploy directory '$DEPLOY_DIR' (try with sudo-capable user)" >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
env_file="${tmp_dir}/.env"
trap 'rm -rf "$tmp_dir"' EXIT

{
  printf 'R8R_IMAGE=%s\n' "$IMAGE"
  printf 'R8R_PORT=%s\n' "$R8R_PORT"
  printf 'R8R_MONITOR_TOKEN=%s\n' "$MONITOR_TOKEN"
  printf 'R8R_API_KEY=%s\n' "$API_KEY"
  printf 'R8R_CORS_ORIGINS=%s\n' "$CORS_ORIGINS"
  printf 'RUST_LOG=%s\n' "$RUST_LOG"
  printf 'R8R_VOLUME_NAME=%s\n' "$VOLUME_NAME"
} > "$env_file"

compose_target="${DEPLOY_DIR}/docker-compose.cloud.yml"
env_target="${DEPLOY_DIR}/.env"

echo "preparing deployment files in ${DEPLOY_DIR}"
if [[ "$use_sudo_files" == "true" ]]; then
  sudo cp "$COMPOSE_SOURCE" "$compose_target"
  sudo cp "$env_file" "$env_target"
else
  cp "$COMPOSE_SOURCE" "$compose_target"
  cp "$env_file" "$env_target"
fi

if [[ -n "$REGISTRY_USERNAME" ]]; then
  echo "logging in to container registry"
  printf '%s' "$REGISTRY_TOKEN" | "${docker_cmd[@]}" login ghcr.io -u "$REGISTRY_USERNAME" --password-stdin
fi

echo "starting local services"
"${compose_cmd[@]}" --env-file "$env_target" -f "$compose_target" pull
"${compose_cmd[@]}" --env-file "$env_target" -f "$compose_target" up -d
"${compose_cmd[@]}" --env-file "$env_target" -f "$compose_target" ps

if [[ "$SKIP_HEALTH_CHECK" == "true" ]]; then
  echo "skipping health checks (--skip-health-check)"
elif command -v curl >/dev/null 2>&1; then
  curl -fsS "http://127.0.0.1:${R8R_PORT}/api/health" >/dev/null
  curl -fsS "http://127.0.0.1:${R8R_PORT}/" >/dev/null
  echo "health checks passed: /api/health and / (UI)"
else
  echo "warning: curl not found; skipping API/UI health checks"
fi

echo "deployment complete"
echo "api: http://127.0.0.1:${R8R_PORT}"
echo "dashboard: http://127.0.0.1:${R8R_PORT}/"
echo "editor: http://127.0.0.1:${R8R_PORT}/editor"
