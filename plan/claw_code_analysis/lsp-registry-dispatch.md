# LSP Registry-Based Dispatch

## What It Is

LSP integration in ClawCode is organized around an in-memory `LspRegistry` that maps language names to server state and capabilities. Rather than embedding hardcoded paths to language server binaries or relying on auto-discovery at runtime, the registry is populated by explicit `register()` calls and queried via `find_server_for_path()`. This gives the system a deterministic, inspectable dispatch surface for all LSP operations.

**Evidence:** `references/claw-code/rust/crates/runtime/src/lsp_client.rs` — `LspRegistry` struct and all associated methods.

---

## Registry Structure

The registry is a `HashMap<String, LspServerState>` protected by an `Arc<Mutex<>>` wrapper so it can be shared across async tasks without a lifetime headache.

```rust
// lsp_client.rs
pub struct LspRegistry {
    inner: Arc<Mutex<RegistryInner>>,
}

struct RegistryInner {
    servers: HashMap<String, LspServerState>,
}
```

`LspServerState` holds everything the runtime needs to know about a language server at dispatch time:

```rust
pub struct LspServerState {
    pub language: String,
    pub status: LspServerStatus,
    pub root_path: Option<String>,
    pub capabilities: Vec<String>,
    pub diagnostics: Vec<LspDiagnostic>,
}
```

The `capabilities` field stores the list of server capabilities advertised during the LSP initialization handshake — things like `hover`, `completion`, `definition`, `references`, `symbols`, `format`, and `diagnostics`. When a tool dispatches an LSP action, the registry's `dispatch()` method checks whether the target server is connected and rejects requests against disconnected servers.

**Evidence:** `LspServerState.capabilities` field and the `dispatch()` method's status check in `lsp_client.rs`.

---

## Explicit Registration, Not Auto-Discovery

The registry is populated through an explicit `register()` call, not by scanning for language servers on the filesystem or calling out to an external discovery mechanism:

```rust
pub fn register(
    &self,
    language: &str,
    status: LspServerStatus,
    root_path: Option<&str>,
    capabilities: Vec<String>,
)
```

Callers pass the language identifier, initial connection status, optional root path, and the capabilities list from the server's `initialize` response. The registry does not reach out to LSP servers to ask what languages they support — that handshake happens elsewhere and the results are handed to the registry.

This design keeps the registry lightweight and predictable. It also means the runtime does not need to manage server lifecycle events like `initialize` and `shutdown` inside the registry itself; those belong to the caller that invoked `register()`.

**Evidence:** `LspRegistry::register()` in `lsp_client.rs`.

---

## Extension-to-Language Matching: The Static Map

When a tool needs to find the right server for a file path, it calls `find_server_for_path()`:

```rust
pub fn find_server_for_path(&self, path: &str) -> Option<LspServerState> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let language = match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" => "cpp",
        "rb" => "ruby",
        "lua" => "lua",
        _ => return None,
    };

    self.get(language)
}
```

The mapping is a static match — no heuristics, no fuzzy matching, no config file lookup. Ten language families are covered:

| Extension(s) | Language key |
|---|---|
| `rs` | `rust` |
| `ts`, `tsx` | `typescript` |
| `js`, `jsx` | `javascript` |
| `py` | `python` |
| `go` | `go` |
| `java` | `java` |
| `c`, `h` | `c` |
| `cpp`, `hpp`, `cc` | `cpp` |
| `rb` | `ruby` |
| `lua` | `lua` |

Files with no recognized extension (e.g., `Makefile`, `Dockerfile`) return `None` — there is no fallback to a generic LSP client. This is intentional; the behavior is explicit rather than guessable.

**Evidence:** `find_server_for_path()` in `lsp_client.rs` and the `find_server_for_all_extensions` test that covers all ten language families.

---

## Capabilities Storage Per Language

When a server is registered, its capabilities are stored in `LspServerState.capabilities` as a `Vec<String>`. This captures what the server announced during LSP initialization — not just that it is connected, but what operations it can handle.

The `dispatch()` method uses this at action-routing time:

```rust
pub fn dispatch(
    &self,
    action: &str,
    path: Option<&str>,
    line: Option<u32>,
    character: Option<u32>,
    _query: Option<&str>,
) -> Result<serde_json::Value, String>
```

Actions supported are: `diagnostics`, `hover`, `definition`, `references`, `completion`, `symbols`, `format`. Each action has aliases so that `goto_definition` and `definition` both resolve to `LspAction::Definition`, and `completions` resolves to `LspAction::Completion`.

The `LspAction::from_str()` method handles the alias resolution:

```rust
pub fn from_str(s: &str) -> Option<Self> {
    match s {
        "diagnostics" => Some(Self::Diagnostics),
        "hover" => Some(Self::Hover),
        "definition" | "goto_definition" => Some(Self::Definition),
        "references" | "find_references" => Some(Self::References),
        "completion" | "completions" => Some(Self::Completion),
        "symbols" | "document_symbols" => Some(Self::Symbols),
        "format" | "formatting" => Some(Self::Format),
        _ => None,
    }
}
```

This means callers can use natural aliases rather than a single canonical name.

**Evidence:** `LspAction`, `from_str()`, and `dispatch()` in `lsp_client.rs`.

---

## Diagnostics as Server-State Part

Diagnostics are cached directly in `LspServerState.diagnostics` per language server. The registry exposes `add_diagnostics()`, `get_diagnostics()`, and `clear_diagnostics()` for managing this cache. When `dispatch()` is called with the `diagnostics` action, it reads from the cached diagnostics rather than triggering a new LSP `textDocument/publishDiagnostics` flow:

```rust
if lsp_action == LspAction::Diagnostics {
    if let Some(path) = path {
        let diags = self.get_diagnostics(path);
        return Ok(serde_json::json!({
            "action": "diagnostics",
            "path": path,
            "diagnostics": diags,
            "count": diags.len()
        }));
    }
    // All diagnostics across all servers
    ...
}
```

This makes diagnostics a pure state read rather than a live RPC call, which fits the tool dispatch model where a tool should return a result without managing long-running connections.

**Evidence:** `add_diagnostics()`, `get_diagnostics()`, `clear_diagnostics()`, and the `dispatch_diagnostics_without_path_aggregates` test in `lsp_client.rs`.

---

## Why Registry-Based Dispatch Beats Hardcoded Server Paths

A naive alternative to the registry would be embedding binary paths like `/usr/local/bin/rust-analyzer` or `~/.cargo/bin/typescript-language-server` directly in the tool layer. That approach has several problems the registry avoids:

**1. Path instability.** Language servers move between machines, get updated, or live under version-manager prefixes (asdf, nix, mise). A hardcoded path breaks on any machine that does not have the server in exactly that location.

**2. No per-language state.** A hardcoded path gives you one server; the registry gives you `Connected`, `Disconnected`, `Starting`, and `Error` status per language, plus a capabilities list, plus a diagnostics cache. Status matters because `dispatch()` explicitly rejects requests against disconnected servers:

```rust
if server.status != LspServerStatus::Connected {
    return Err(format!(
        "LSP server for '{}' is not connected (status: {})",
        server.language, server.status
    ));
}
```

A bare path has no concept of connection state.

**3. Single-responsibility dispatch.** `find_server_for_path()` handles the extension-to-language routing so the caller does not need to know about file extensions at all. The caller says "find me the right server for this path" and gets back a `LspServerState` or `None`. The routing logic lives in one place rather than being scattered across tool handlers.

**4. Testability.** The registry's `HashMap<String, LspServerState>` is straightforward to seed with known state in tests. The `find_server_for_all_extensions` test exercises all ten languages with real registry operations rather than mocking filesystem paths. A hardcoded path approach would require either mocking the filesystem or running real language servers in tests.

**5. Extension to new languages.** Adding Python support means calling `register("python", ...)` with the appropriate capabilities and root path — no code changes to the dispatch logic. The static match in `find_server_for_path()` is the only place extension mapping lives, and it too is designed to be extended without touching dispatch.

---

## Key Files

- `references/claw-code/rust/crates/runtime/src/lsp_client.rs` — `LspRegistry`, `LspServerState`, `register()`, `find_server_for_path()`, `dispatch()`, `LspAction`, and all diagnostics methods.

---

## Builder Lessons

1. **Explicit registration > auto-discovery for deterministic dispatch.** Auto-discovery sounds convenient but introduces nondeterminism. An explicit `register()` call makes the system predictable and testable.

2. **State accumulated at registration survives query operations.** Storing capabilities and connection status at registration means `dispatch()` does not need to re-establish or re-query the server. The state was captured once and reused.

3. **Extension maps belong in a single function, not inline.** `find_server_for_path()` centralizes all extension-to-language routing. Adding a new language or changing a mapping is a one-line change in one place.

4. **Aliases on the action layer make the API friendlier.** `definition` and `goto_definition` both work because `from_str()` maps aliases to canonical variants. Callers get natural naming without the dispatch layer having to care.

5. **Disconnected servers are a first-class error, not a silent skip.** `dispatch()` returns an explicit error when the server is not `Connected`. This keeps the caller aware of the real connection state rather than silently succeeding against a dead server.
