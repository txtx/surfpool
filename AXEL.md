---
workspace: surfpool

skills:
  - path: ./skills
  - path: ~/.config/axel/skills

layouts:
  panes:
    - type: claude
      color: gray
      skills:
        - "*"

    - type: codex
      color: green
      skills:
        - "*"

    - type: custom
      name: shell
      notes:
        - "$ cargo build"
        - "$ cargo test"
        - "$ cargo run -- --help"

  grids:
    default:
      type: tmux
      claude:
        col: 0
        row: 0
        width: 50
      shell:
        col: 1
        row: 0
        color: yellow
---

# surfpool

Surfpool is a Solana development toolkit that provides a local validator environment, CLI tools, MCP server, GraphQL API, and a studio UI for building and testing Solana programs.

## Overview

Rust workspace with 9 crates: `cli` (main entry point), `core` (runtime), `mcp` (MCP server), `gql` (GraphQL API), `studio` (web UI), `subgraph` (indexing), `db` (storage), `types` (shared types), and `bench` (benchmarks).

## Getting Started

```bash
cargo build                    # Build all crates
cargo run -- --help            # Run the CLI
cargo test                     # Run tests
```

## Architecture

- **crates/cli** - Command-line interface (default member)
- **crates/core** - Core Solana runtime and validator logic
- **crates/mcp** - Model Context Protocol server
- **crates/gql** - GraphQL API (juniper + actix-web)
- **crates/studio** - Web-based studio UI
- **crates/subgraph** - Subgraph/indexing support
- **crates/db** - Database layer (diesel)
- **crates/types** - Shared type definitions
- **crates/bench** - Benchmarking

## Key Files

- `Cargo.toml` - Workspace root with all dependency versions
- `crates/cli/src/main.rs` - CLI entry point
- `crates/core/` - Core runtime logic
