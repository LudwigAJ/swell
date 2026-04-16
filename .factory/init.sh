#!/bin/bash
set -euo pipefail

if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo not found. Install Rust via https://rustup.rs"
  exit 1
fi

echo "Checking workspace compilation..."
cargo check --workspace >/dev/null

echo "Checking audit wiring guardrail test target exists..."
cargo test -p swell-integration-tests --test full_cycle_wiring -- --list >/dev/null

echo "Swell audit-recovery mission workspace ready."
