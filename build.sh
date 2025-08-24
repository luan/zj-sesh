#!/bin/bash

# Build the WASM plugin
cargo build --release

# Create the plugins directory if it doesn't exist
mkdir -p ~/.config/zellij/plugins

# Copy the built WASM file to the plugins directory
cp target/wasm32-wasip1/release/zj-sesh.wasm ~/.config/zellij/plugins/

echo "✅ Built and installed zj-sesh.wasm to ~/.config/zellij/plugins/"