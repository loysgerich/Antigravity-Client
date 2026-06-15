#!/bin/bash
set -e

echo "=== Syncing with GitHub ==="

# Initialize git if needed
if [ ! -d .git ]; then
    git init
    git remote add origin git@github.com:loysgerich/Antigravity-Client.git
fi

# Fetch remote state
git fetch origin

# Detect main/master branch
BRANCH="main"
if git show-ref --verify --quiet refs/remotes/origin/master; then
    BRANCH="master"
fi

echo "Detected default branch: $BRANCH"

# Reset local git history to match origin without losing local file changes
git reset --mixed origin/$BRANCH || echo "Note: Starting from empty history as origin branch was not found"

# Configure user if not set
if [ -z "$(git config user.name)" ]; then
    git config user.name "loysgerich"
fi
if [ -z "$(git config user.email)" ]; then
    git config user.email "loysgerich@users.noreply.github.com"
fi

# Stage files
git add .gitignore package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/src/lib.rs src-tauri/src/local_proxy.rs system_architecture_and_proxy_prompt.md README.md push_to_github.sh

# Commit
git commit -m "Release v1.0.11: Patch macOS permissions, self-killing fix, optimized search and Antigravity IDE support"

# Push
git push origin HEAD:$BRANCH

# Tag & Release
git tag -d v1.0.11 2>/dev/null || true
git push origin :refs/tags/v1.0.11 2>/dev/null || true
git tag -a v1.0.11 -m "Release v1.0.11"
git push origin v1.0.11

echo "=== Release v1.0.11 successfully pushed to GitHub! ==="
