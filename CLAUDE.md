# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Surfpool is a blazing-fast in-memory Solana testnet that can fork mainnet instantly. It's built on the Solana SVM (Solana Virtual Machine) using LiteSVM for fast boot times and provides a local development environment for Solana programs.

## Architecture

This is a Rust workspace with multiple crates:

- **`crates/cli/`** - Main CLI application (`surfpool` binary)
- **`crates/core/`** - Core Surfpool functionality including SVM integration and RPC services
- **`crates/db/`** - Database abstractions (SQLite/PostgreSQL)
- **`crates/gql/`** - GraphQL API implementation
- **`crates/mcp/`** - Model Context Protocol integration for AI agents
- **`crates/studio/`** - Studio UI components
- **`crates/subgraph/`** - Subgraph functionality
- **`crates/types/`** - Shared type definitions

### Key Components

- **SurfnetSvm** (`crates/core/src/surfnet/svm.rs`) - Core SVM wrapper that manages Solana program execution
- **RPC Layer** (`crates/core/src/rpc/`) - JSON-RPC API compatible with Solana's RPC interface
- **Geyser Integration** - For real-time account/transaction streaming
- **Remote Account Fetching** - Dynamically fetches missing mainnet accounts during execution

## Common Development Commands

### Building
```bash
# Build the project
cargo build

# Build with all features (including geyser plugin)
cargo build --features geyser-plugin

# Build release version
cargo build --release
```

### Testing
```bash
# Run all tests
cargo test --all --verbose

# Run tests with CI-specific features
cargo test --all --verbose --features "ignore_tests_ci geyser-plugin"
```

### Code Quality
```bash
# Format code (uses nightly rustfmt)
cargo +nightly fmt --all

# Check formatting
cargo +nightly fmt --all -- --check

# Run clippy linter
cargo clippy --all-targets

# Run clippy with all features
cargo clippy --all-targets --features geyser-plugin
```

### Running Surfpool
```bash
# Start local Solana network
surfpool start

# Show available options
surfpool start --help

# Install from source
cargo install --path crates/cli
```

## Development Notes

- Uses Rust 1.86.0 toolchain with specific components (`llvm-tools`, `rustc-dev`)
- Requires `libudev-dev` on Linux systems
- Solana SDK version 2.2.1 is used throughout
- The project uses workspace dependencies to ensure version consistency
- Formatting configuration in `rustfmt.toml` enforces import organization and style

## Key Features

- **Anchor Integration** - Automatically detects and deploys Anchor projects
- **Dynamic Account Fetching** - Retrieves mainnet accounts on-demand during transaction execution
- **Infrastructure as Code** - Generates deployment configurations similar to Terraform
- **MCP Support** - AI agent integration for automated testing and development workflows