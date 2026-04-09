#!/bin/bash
set -e

cd /Users/ludwigjonsson/Projects/swell

echo "Loading SWELL settings from .swell/..."
if [ -f ".swell/settings.json" ]; then
    echo "  - Found settings.json"
fi
if [ -f ".swell/policies/default.yaml" ]; then
    echo "  - Found policy rules"
fi
if [ -f ".swell/models.json" ]; then
    echo "  - Found model configuration"
fi

echo "Installing Rust dependencies..."
cargo fetch

echo "Building workspace..."
cargo build --workspace

echo "Running tests..."
cargo test --workspace

echo "Setup complete."
