#!/bin/sh
# r8r release script
# Usage: ./scripts/release.sh [patch|minor|major] [--dry-run]
#
# Automates the release flow:
#   1. Bump version in Cargo.toml
#   2. Update Cargo.lock
#   3. Commit version bump
#   4. Create & push git tag (triggers GitHub Actions release workflow)
#
# Requires: main branch, clean working tree, gh CLI

set -e

# ── Config ──────────────────────────────────────────────────────────
MAIN_BRANCH="main"

# ── Args ────────────────────────────────────────────────────────────
BUMP="${1:-patch}"
DRY_RUN=false
if [ "$2" = "--dry-run" ] || [ "$1" = "--dry-run" ]; then
    DRY_RUN=true
    if [ "$1" = "--dry-run" ]; then BUMP="patch"; fi
fi

# ── Helpers ─────────────────────────────────────────────────────────
say()  { printf '\033[1;34m▸\033[0m %s\n' "$1"; }
ok()   { printf '\033[1;32m✓\033[0m %s\n' "$1"; }
err()  { printf '\033[1;31m✗\033[0m %s\n' "$1" >&2; exit 1; }
warn() { printf '\033[1;33m!\033[0m %s\n' "$1"; }

# ── Preflight ───────────────────────────────────────────────────────
say "Running preflight checks..."

# Must be on main
CURRENT_BRANCH="$(git branch --show-current)"
if [ "$CURRENT_BRANCH" != "$MAIN_BRANCH" ]; then
    err "Must be on '$MAIN_BRANCH' branch (currently on '$CURRENT_BRANCH')"
fi

# Working tree must be clean
if [ -n "$(git status --porcelain)" ]; then
    err "Working tree is not clean. Commit or stash changes first."
fi

# Must be up to date with remote
git fetch origin "$MAIN_BRANCH" --quiet
LOCAL="$(git rev-parse HEAD)"
REMOTE="$(git rev-parse origin/$MAIN_BRANCH)"
if [ "$LOCAL" != "$REMOTE" ]; then
    err "Local $MAIN_BRANCH is not up to date with origin. Run 'git pull' first."
fi

# Cargo.toml must exist
if [ ! -f Cargo.toml ]; then
    err "Cargo.toml not found. Run from project root."
fi

ok "Preflight passed"

# ── Read current version ────────────────────────────────────────────
CURRENT_VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')"
if [ -z "$CURRENT_VERSION" ]; then
    err "Could not read version from Cargo.toml"
fi

# ── Compute next version ────────────────────────────────────────────
IFS='.' read -r MAJOR MINOR PATCH <<EOF
$CURRENT_VERSION
EOF

case "$BUMP" in
    patch) PATCH=$((PATCH + 1)) ;;
    minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
    major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
    *)     err "Unknown bump type: $BUMP (use patch, minor, or major)" ;;
esac

NEXT_VERSION="${MAJOR}.${MINOR}.${PATCH}"
TAG="v${NEXT_VERSION}"

say "Version: $CURRENT_VERSION → $NEXT_VERSION ($BUMP)"

# Check tag doesn't already exist
if git tag -l "$TAG" | grep -q .; then
    err "Tag $TAG already exists"
fi

# ── Dry run exit ────────────────────────────────────────────────────
if [ "$DRY_RUN" = true ]; then
    warn "Dry run — would release $TAG"
    exit 0
fi

# ── Confirm ─────────────────────────────────────────────────────────
printf '\nRelease \033[1m%s\033[0m? (y/N) ' "$TAG"
read -r CONFIRM
case "$CONFIRM" in
    y|Y|yes) ;;
    *)       say "Aborted."; exit 0 ;;
esac

# ── Bump version ────────────────────────────────────────────────────
say "Bumping Cargo.toml version to $NEXT_VERSION..."
sed -i.bak "s/^version = \"$CURRENT_VERSION\"/version = \"$NEXT_VERSION\"/" Cargo.toml
rm -f Cargo.toml.bak

# Update Cargo.lock
say "Updating Cargo.lock..."
cargo generate-lockfile --quiet 2>/dev/null || cargo check --quiet 2>/dev/null || true

# ── Commit & tag ────────────────────────────────────────────────────
say "Committing version bump..."
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to $NEXT_VERSION"

say "Creating tag $TAG..."
git tag -a "$TAG" -m "Release $TAG"

# ── Push ────────────────────────────────────────────────────────────
say "Pushing to origin..."
git push origin "$MAIN_BRANCH"
git push origin "$TAG"

ok "Released $TAG"
echo ""
say "GitHub Actions release workflow triggered."
say "Track progress: https://github.com/qhkm/r8r/actions"
echo ""
say "After build completes, verify:"
say "  - GitHub Release: https://github.com/qhkm/r8r/releases/tag/$TAG"
say "  - Install test:   curl -sSf https://r8r.sh/install.sh | R8R_VERSION=$TAG sh"
