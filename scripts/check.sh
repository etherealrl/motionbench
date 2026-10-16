#!/usr/bin/env bash
# the gates, in one place. `commit` is the quick set the pre-commit hook runs, `push` (the default) is everything,
# which the pre-push hook and CI on the public repo run. playwright stays out because it needs a browser and a dev
# server, it runs in the review gate instead
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
stage="${1:-push}"
# git points hooks at the commit's index, and jscpd and fallow make their own worktree of the base commit, which
# must use its own index
unset GIT_INDEX_FILE

# the version is written in four places and a release reads it from Cargo.toml, so they must agree
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
for other in \
  "$(sed -n 's/^version = "\(.*\)"$/\1/p' src-tauri/Cargo.toml | head -n 1)" \
  "$(sed -n 's/^  "version": "\(.*\)",$/\1/p' package.json | head -n 1)" \
  "$(sed -n 's/^  "version": "\(.*\)",$/\1/p' src-tauri/tauri.conf.json | head -n 1)"; do
  if [ "$other" != "$version" ]; then
    echo "check: the version isn't the same everywhere, Cargo.toml says $version and another file says ${other:-nothing}" >&2
    exit 1
  fi
done

dev="--features motion-bench-desktop/developer-tools"
# CI builds the app with and without the developer tools as two jobs side by side, CHECK_VARIANT says which one
# this is. the hooks leave it unset and do both
variant="${CHECK_VARIANT:-both}"
if [ "$variant" = dev ]; then
  # shellcheck disable=SC2086
  cargo clippy --workspace --all-targets --no-default-features $dev -- -D warnings
  # shellcheck disable=SC2086
  cargo test --workspace --no-default-features $dev
  exit 0
fi

# the checks below that look for their own files first only run where those files are
if [ "$stage" = push ] && [ -f scripts/tests/git-identity.sh ]; then
  bash scripts/tests/git-identity.sh
fi
if [ -f .fallowrc.json ]; then
  # only findings this change brings in fail, the existing debt is reported, not enforced. a commit compares
  # against HEAD, a push against what the remote already has or the last commit on a branch never pushed
  base=HEAD
  if [ "$stage" = push ]; then
    base="$(git rev-parse --abbrev-ref "@{upstream}" 2>/dev/null || echo HEAD~1)"
  fi
  FALLOW_AUDIT_BASE="$base" bun x --no-install fallow audit --format compact
  bun x --no-install jscpd --baseline-from-ref "$base" --fail-on-new-clones --silent
fi

cargo fmt --all -- --check
# formatting, lint with the shadcn rules and the type check, all through Vite+
bun run check
if [ -d tools/checks ]; then
  bun run check:docs
  bun run check:css
fi
if [ -f .privacy-patterns.txt ]; then bun run check:identity; fi
cargo clippy --workspace --all-targets --no-default-features -- -D warnings
if [ "$stage" = commit ]; then
  exit 0
fi
cargo test --workspace --no-default-features
bun run test
if [ "$variant" = both ]; then
  # shellcheck disable=SC2086
  cargo clippy --workspace --all-targets --no-default-features $dev -- -D warnings
  # shellcheck disable=SC2086
  cargo test --workspace --no-default-features $dev
fi
