## MCP Integration

To use the MCP tools in Cursor or other IDEs:

1. Build the project in release mode:
   ```sh
   cargo build --release
   ```
2. Ensure you are running your IDE from the project root (where `.cursor/mcp.json` is located).
3. The MCP server will be available at `./target/release/surfpool mcp`.
