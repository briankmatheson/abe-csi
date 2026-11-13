#!/usr/bin/env bash
# The repo is github.com/briankmatheson/...; a recurring typo drops the "k".
# Fails if the bad form appears anywhere (email addresses do not match).
set -euo pipefail
cd "$(dirname "$0")/.."
if hits=$(grep -rn 'github\.com/brianmatheson/' --exclude-dir=target --exclude-dir=.git --exclude=Cargo.lock .); then
  echo "bad repo links (brianmatheson -> briankmatheson):" >&2
  echo "$hits" >&2
  exit 1
fi
echo "lint-links: ok"
