#!/bin/bash
# NOTE: This file should be executable (chmod +x .factory/init.sh)
set -euo pipefail

# Ensure Rust toolchain is available
if ! command -v cargo &> /dev/null; then
    echo "ERROR: cargo not found. Install Rust via https://rustup.rs"
    exit 1
fi

# Verify workspace compiles
echo "Checking workspace compilation..."
cargo check --workspace 2>&1 || {
    echo "ERROR: Workspace compilation failed"
    exit 1
}

echo "Swell workspace ready."
