# `.claw/settings.local.json` — Machine-Local Override Layer

`.claw/settings.local.json` is the highest-precedence, machine-local configuration file in ClawCode's config discovery chain. It sits above all shared project and user-level settings and is explicitly excluded from version control to prevent machine-specific overrides from leaking into shared repositories.

## Discovery Position

`ConfigLoader::discover()` in `runtime/src/config.rs` enumerates the full config file chain in load order:

```rust
vec![
    ConfigEntry { source: ConfigSource::User, path: user_legacy_path },      // ~/.claw.json
    ConfigEntry { source: ConfigSource::User, path: self.config_home.join("settings.json") },  // ~/.config/claw/settings.json
    ConfigEntry { source: ConfigSource::Project, path: self.cwd.join(".claw.json") },          // <repo>/.claw.json
    ConfigEntry { source: ConfigSource::Project, path: self.cwd.join(".claw").join("settings.json") },  // <repo>/.claw/settings.json
    ConfigEntry { source: ConfigSource::Local, path: self.cwd.join(".claw").join("settings.local.json") }, // <repo>/.claw/settings.local.json
]
```

`ConfigSource::Local` is the last and highest entry. The `discover()` method returns this list in ascending precedence order; `ConfigLoader::load()` merges them sequentially, so each successive file deep-merges into the accumulated result. Values from `settings.local.json` therefore override any matching keys from project or user config.

The `discover()` order maps directly to the precedence chain documented in `USAGE.md`:

> 1. `~/.claw.json`
> 2. `~/.config/claw/settings.json`
> 3. `<repo>/.claw.json`
> 4. `<repo>/.claw/settings.json`
> 5. `<repo>/.claw/settings.local.json`

## Machine-Local Semantics

`settings.local.json` is intentionally excluded from version control. The `initialize_repo()` bootstrap in `rusty-claude-cli/src/init.rs` adds it to `.gitignore` alongside sessions and agent state:

```rust
const GITIGNORE_ENTRIES: [&str; 3] = [".claw/settings.local.json", ".claw/sessions/", ".clawhip/"];
```

The `init` test `initialize_repo_creates_expected_files_and_gitignore_entries` asserts that the entry is present in the generated `.gitignore`, and `initialize_repo_is_idempotent_and_preserves_existing_files` asserts the entry appears exactly once even on re-initialization. This ensures the file is never committed.

The CLAUDE.md working agreement reinforces the distinction:

> Keep shared defaults in `.claw.json`; reserve `.claw/settings.local.json` for machine-local overrides.

## What Overrides Are Possible

Because `settings.local.json` participates in the same deep-merge as all other config files, any top-level keys can be overridden. The `loads_and_merges_claude_code_config_files_by_precedence` test in `runtime/src/config.rs` exercises the full chain and confirms that a local `model: "opus"` and `permissionMode: "acceptEdits"` win over all prior sources:

```rust
fs::write(
    cwd.join(".claw").join("settings.local.json"),
    r#"{"model":"opus","permissionMode":"acceptEdits"}"#,
).expect("write local settings");
```

The merged result has `model = "opus"` despite user and project configs setting `"sonnet"` and `"project-compat"`.

`parses_typed_mcp_and_oauth_config` demonstrates the same override behavior for `mcpServers`: a local entry with `ConfigSource::Local` replaces the user-level `http` transport for the same server name with a `ws` transport:

```rust
fs::write(
    cwd.join(".claw").join("settings.local.json"),
    r#"{
      "mcpServers": {
        "remote-server": {
          "type": "ws",
          "url": "wss://override.test/mcp",
          "headers": {"X-Env": "local"}
        }
      }
    }"#,
).expect("write local settings");
```

Loaded `mcp.get("remote-server").scope == ConfigSource::Local` and `transport() == McpTransport::Ws`.

Other overridable sections include:
- `aliases` — model nickname shortcuts
- `hooks` — pre/post-tool command lists
- `permissions` / `permissionMode` — default mode and allow/deny/ask rules
- `sandbox` — filesystem, namespace, and network isolation settings
- `providerFallbacks` — fallback chain for retryable errors
- `trustedRoots` — verified certificate anchor paths
- `oauth` — main runtime OAuth client config

## Builder Takeaways

**Override scope is the entire file.** Unlike some config systems that restrict override to specific fields, `settings.local.json` accepts any top-level key and merges recursively. A local file that only contains `{"model": "claude-opus-4-6"}` will override the model while leaving all other project settings (hooks, MCP servers, sandbox config) untouched. This makes machine-specific overrides surgical rather than all-or-nothing.

**Deep merge preserves nested structure.** `deep_merge_objects()` in `runtime/src/config.rs` merges nested maps rather than replacing them wholesale. In the `parses_user_defined_model_aliases_from_settings` test, the local file `{"aliases":{"smart":"claude-sonnet-4-6","cheap":"grok-3-mini"}}` adds `"cheap"` while overriding `"smart"`, leaving the user-level `"fast"` alias intact. A builder implementing similar override systems should prefer deep merge to avoid accidental data loss.

**The Local source tag enables scope-aware MCP server merging.** The `merge_mcp_servers()` function in `config.rs` inserts into a `BTreeMap<String, ScopedMcpServerConfig>`, keyed by server name. When a Local entry appears for the same server name as a User or Project entry, the Local config replaces it entirely (not merge-attempts at the transport level). This gives machine-local MCP endpoint overrides predictable semantics.

**Gitignore is part of the override contract.** Because `settings.local.json` is in `.gitignore`, teams that rely on shared `.claw/settings.json` for MCP server URLs, permission defaults, or hook commands will never accidentally pick up a collaborator's local overrides. When implementing similar "machine-only" config layers, treat the VCS exclusion as a load-bearing part of the design, not an afterthought.
