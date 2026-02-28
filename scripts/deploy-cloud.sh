#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Deploy r8r Docker service to a remote Linux host over SSH.
The r8r container includes API + embedded UI (/ and /editor).

Usage:
  scripts/deploy-cloud.sh --host <host> --user <user> --ssh-key <path> --image <image> [options]

Required:
  --host <host>              Remote host (IP or DNS)
  --user <user>              Remote SSH user
  --ssh-key <path>           SSH private key file
  --image <image>            Container image (e.g. ghcr.io/acme/r8r:abcdef)
  --monitor-token <token>    R8R_MONITOR_TOKEN for /api/monitor auth

Optional:
  --ssh-port <port>          SSH port (default: 22)
  --remote-dir <path>        Remote deployment directory (default: /opt/r8r)
  --r8r-port <port>          Exposed API port (default: 8080)
  --bootstrap                Install Docker + Compose on Debian/Ubuntu VPS first
  --skip-health-check        Skip remote API/UI health checks after deploy
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

HOST=""
USER=""
SSH_KEY=""
SSH_PORT="22"
REMOTE_DIR="/opt/r8r"
IMAGE=""
R8R_PORT="8080"
MONITOR_TOKEN=""
BOOTSTRAP="false"
SKIP_HEALTH_CHECK="false"
API_KEY=""
CORS_ORIGINS=""
RUST_LOG="r8r=info"
VOLUME_NAME="r8r-data"
REGISTRY_USERNAME=""
REGISTRY_TOKEN=""
LOCAL_COMPOSE_FILE="deploy/docker-compose.cloud.yml"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host)
      HOST="$2"
      shift 2
      ;;
    --user)
      USER="$2"
      shift 2
      ;;
    --ssh-key)
      SSH_KEY="$2"
      shift 2
      ;;
    --ssh-port)
      SSH_PORT="$2"
      shift 2
      ;;
    --remote-dir)
      REMOTE_DIR="$2"
      shift 2
      ;;
    --image)
      IMAGE="$2"
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
    --monitor-token)
      MONITOR_TOKEN="$2"
      shift 2
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

if [[ -z "$HOST" || -z "$USER" || -z "$SSH_KEY" || -z "$IMAGE" || -z "$MONITOR_TOKEN" ]]; then
  echo "error: missing required arguments" >&2
  usage
  exit 1
fi

if [[ ! -f "$SSH_KEY" ]]; then
  echo "error: SSH key not found: $SSH_KEY" >&2
  exit 1
fi

if [[ ! -f "$LOCAL_COMPOSE_FILE" ]]; then
  echo "error: compose file not found: $LOCAL_COMPOSE_FILE" >&2
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

require_cmd ssh
require_cmd scp

ssh_opts=(
  -i "$SSH_KEY"
  -p "$SSH_PORT"
  -o StrictHostKeyChecking=accept-new
)
scp_opts=(
  -i "$SSH_KEY"
  -P "$SSH_PORT"
  -o StrictHostKeyChecking=accept-new
)
target="${USER}@${HOST}"

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

echo "creating remote directory: ${REMOTE_DIR}"
ssh "${ssh_opts[@]}" "$target" "mkdir -p '$REMOTE_DIR'"

if [[ "$BOOTSTRAP" == "true" ]]; then
  echo "bootstrap mode: installing Docker + Compose on remote host (Debian/Ubuntu)"
  ssh "${ssh_opts[@]}" "$target" "bash -s" <<'EOF'
set -euo pipefail

if command -v docker >/dev/null 2>&1 && (docker compose version >/dev/null 2>&1 || command -v docker-compose >/dev/null 2>&1); then
  echo "docker + compose already installed"
  exit 0
fi

if ! command -v sudo >/dev/null 2>&1; then
  echo "error: bootstrap requires sudo access on remote host" >&2
  exit 1
fi

if [[ ! -r /etc/os-release ]]; then
  echo "error: cannot detect remote OS" >&2
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
    echo "error: --bootstrap only supports ubuntu/debian. Install Docker + Compose manually." >&2
    exit 1
    ;;
esac

if id -nG "$(whoami)" | grep -qw docker; then
  echo "remote user already in docker group"
else
  sudo usermod -aG docker "$(whoami)"
  echo "notice: added $(whoami) to docker group; future SSH sessions will pick this up"
fi
EOF
fi

echo "uploading compose + environment files"
scp "${scp_opts[@]}" "$LOCAL_COMPOSE_FILE" "$target:$REMOTE_DIR/docker-compose.cloud.yml"
scp "${scp_opts[@]}" "$env_file" "$target:$REMOTE_DIR/.env"

if [[ -n "$REGISTRY_USERNAME" ]]; then
  echo "logging in to registry on remote host"
  quoted_registry_username="$(printf '%q' "$REGISTRY_USERNAME")"
  printf '%s' "$REGISTRY_TOKEN" | ssh "${ssh_opts[@]}" "$target" \
    "if docker info >/dev/null 2>&1; then docker login ghcr.io -u ${quoted_registry_username} --password-stdin; elif sudo -n docker info >/dev/null 2>&1; then sudo docker login ghcr.io -u ${quoted_registry_username} --password-stdin; else echo 'error: docker daemon is not accessible for registry login' >&2; exit 1; fi"
fi

echo "starting remote services"
ssh "${ssh_opts[@]}" "$target" \
  "REMOTE_DIR='$REMOTE_DIR' R8R_PORT='$R8R_PORT' SKIP_HEALTH_CHECK='$SKIP_HEALTH_CHECK' bash -s" <<'EOF'
set -euo pipefail

cd "$REMOTE_DIR"

if docker info >/dev/null 2>&1; then
  docker_cmd=(docker)
elif sudo -n docker info >/dev/null 2>&1; then
  docker_cmd=(sudo docker)
else
  echo "error: docker daemon is not accessible for user $(whoami)" >&2
  exit 1
fi

if "${docker_cmd[@]}" compose version >/dev/null 2>&1; then
  compose_cmd=("${docker_cmd[@]}" compose)
elif command -v docker-compose >/dev/null 2>&1; then
  if docker-compose version >/dev/null 2>&1; then
    compose_cmd=(docker-compose)
  elif sudo -n docker-compose version >/dev/null 2>&1; then
    compose_cmd=(sudo docker-compose)
  else
    echo 'error: docker-compose exists but is not executable for current user' >&2
    exit 1
  fi
else
  echo 'error: docker compose is required on remote host' >&2
  exit 1
fi

"${compose_cmd[@]}" --env-file .env -f docker-compose.cloud.yml pull
"${compose_cmd[@]}" --env-file .env -f docker-compose.cloud.yml up -d
"${compose_cmd[@]}" --env-file .env -f docker-compose.cloud.yml ps

if [[ "$SKIP_HEALTH_CHECK" == "true" ]]; then
  echo "skipping health checks (--skip-health-check)"
elif command -v curl >/dev/null 2>&1; then
  curl -fsS "http://127.0.0.1:${R8R_PORT}/api/health" >/dev/null
  curl -fsS "http://127.0.0.1:${R8R_PORT}/" >/dev/null
  echo "health checks passed: /api/health and / (UI)"
else
  echo "warning: curl not found on remote host; skipping API/UI health checks"
fi
EOF

echo "deployment complete"
echo "api: http://${HOST}:${R8R_PORT}"
echo "dashboard: http://${HOST}:${R8R_PORT}/"
echo "editor: http://${HOST}:${R8R_PORT}/editor"
