# MCP Server Tool Verification

When implementing MCP client features in `swell-tools`, verify actual tool names and argument formats by checking the MCP server's source code or capabilities response.

## mcp-language-server (github.com/isaacphi/mcp-language-server)

**Actual tools exposed:**
- `definition` - requires `{filePath, line, column}` (1-indexed)
- `references` - requires `{filePath, line, column}` (1-indexed)
- `hover` - requires `{filePath, line, column}` (1-indexed)
- `diagnostics` - requires `{filePath}`
- `rename_symbol` - requires `{filePath, line, column, newName}` (NOT `rename`)
- `edit_file` - requires `{filePath, oldText, newText}`

**Does NOT exist:** `workspace_diagnostics`, `document_symbols`

**Argument format:** Uses `filePath` (not `textDocumentUri`), `line` and `column` are 1-indexed.

## tree-sitter-server (github.com/wrale/mcp-server-tree-sitter)

**Tools:**
- `get_ast` - AST parsing for file
- `get_node_at_position` - find node at position
- `run_query` - execute tree-sitter queries
- `get_symbols` - extract symbols from AST
- `find_usage` - find symbol references
- `analyze_project` - project-wide analysis
- `get_dependencies` - crate dependency analysis
- `analyze_complexity` - code complexity metrics

All tree-sitter tools are read-only (readOnlyHint: true).

## Common Patterns

1. **Never assume tool names** - verify against actual server capabilities
2. **Argument format varies** - some servers use `textDocumentUri`, others use `filePath`
3. **Indexing differs** - LSP uses 0-indexed lines/columns in JSON-RPC, but some servers use 1-indexed
4. **Check capabilities** - run `tools/list` on the MCP server to discover available tools

## Debugging MCP Tool Calls

When a tool call fails with "tool not found":
1. Check the actual tool name in the server's `tools/list` response
2. Verify argument structure matches what the server expects
3. Check if the server supports the tool at all (some tools are server-specific)
