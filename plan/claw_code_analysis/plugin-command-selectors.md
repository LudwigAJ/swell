# Plugin Command Selectors

Plugin management in ClawCode is exposed through the `/plugin` slash command family. This document explains the command aliases, the full action surface, and the critical distinction between **name-targeted** and **id-targeted** operations.

## Command Aliases

The plugin command is accessible under three names:

| Alias | Use Case |
|-------|----------|
| `/plugin` | Primary canonical form |
| `/plugins` | Plural alias (common CLI convention) |
| `/marketplace` | Marketplace-facing alias |

All three aliases resolve to the same handler. The alias surface is declared in `SlashCommandSpec`:

```rust
// references/claw-code/rust/crates/commands/src/lib.rs
SlashCommandSpec {
    name: "plugin",
    aliases: &["plugins", "marketplace"],
    summary: "Manage Claw Code plugins",
    argument_hint: Some(
        "[list|install <path>|enable <name>|disable <name>|uninstall <id>|update <id>]",
    ),
    resume_supported: false,
}
```

## Management Actions

The plugin command supports six actions:

| Action | Selector | Description |
|--------|----------|-------------|
| `list` | — | Lists all installed plugins with name, version, and enabled status |
| `install <path>` | path | Installs a plugin from a filesystem path |
| `enable <name>` | name | Enables a plugin by name (resolves via `resolve_plugin_target`) |
| `disable <name>` | name | Disables a plugin by name (resolves via `resolve_plugin_target`) |
| `uninstall <id>` | id | Uninstalls a plugin by stable plugin identifier |
| `update <id>` | id | Updates a plugin by stable plugin identifier |

## Name-Targeted vs Id-Targeted Operations

This is the most important selector distinction in the plugin system.

### Name-Targeted: `enable` and `disable`

The `enable` and `disable` actions accept a **display name** selector (`<name>`), but the selector is resolved through `resolve_plugin_target`, which matches against **either** `metadata.id` **or** `metadata.name`:

```rust
// references/claw-code/rust/crates/commands/src/lib.rs
fn resolve_plugin_target(
    manager: &PluginManager,
    target: &str,
) -> Result<PluginSummary, PluginError> {
    let mut matches = manager
        .list_installed_plugins()?
        .into_iter()
        .filter(|plugin| plugin.metadata.id == target || plugin.metadata.name == target)
        .collect::<Vec<_>>();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(PluginError::NotFound(...)),
        _ => Err(PluginError::InvalidManifest(
            "plugin name `{target}` is ambiguous; use the full plugin id"
        )),
    }
}
```

**Resolution behavior:**
- If the target string matches exactly one plugin's `id` OR `name` → operation proceeds on that plugin
- If zero matches → `PluginError::NotFound`
- If multiple plugins match (e.g., two plugins share the same display name) → `PluginError::InvalidManifest` with message "ambiguous; use the full plugin id"

This means you can type `/plugin enable my-plugin` and it will find the plugin whether `my-plugin` is the display name or the stable id.

### Id-Targeted: `uninstall` and `update`

The `uninstall` and `update` actions accept a **stable plugin identifier** (`<id>`). Unlike `enable`/`disable`, these operations do **not** use `resolve_plugin_target`. They use the provided string directly as the `plugin_id`:

```rust
// references/claw-code/rust/crates/commands/src/lib.rs
Some("uninstall") => {
    let Some(target) = target else {
        return Ok(PluginsCommandResult { message: "Usage: /plugins uninstall <plugin-id>".to_string(), ... });
    };
    manager.uninstall(target)?;
    Ok(PluginsCommandResult { message: format!("Plugins\n  Result           uninstalled {target}"), reload_runtime: true })
}
```

**Why the distinction?**
- The stable `id` is guaranteed to be unique across all plugins
- Display `name` is user-facing and may not be unique
- Destructive operations (`uninstall`, `update`) require unambiguous targeting
- Non-destructive operations (`enable`, `disable`) can tolerate ambiguity resolution

### Error Messages Reflect Selector Semantics

The usage strings in error messages make the selector expectation explicit:

```
/plugin enable <name>       # expects name or id (resolved)
/plugin disable <name>       # expects name or id (resolved)
/plugin uninstall <id>       # expects stable id directly
/plugin update <id>          # expects stable id directly
```

When `enable`/`disable` fails due to ambiguity, the error tells you to use the id:

```
plugin name `my-plugin` is ambiguous; use the full plugin id
```

## Plugin Lifecycle States

Plugin lifecycle is defined in `plugin_lifecycle.rs` and exposes health state through `PluginState`:

```rust
// references/claw-code/rust/crates/runtime/src/plugin_lifecycle.rs
pub enum PluginState {
    Unconfigured,
    Validated,
    Starting,
    Healthy,
    Degraded { healthy_servers: Vec<String>, failed_servers: Vec<ServerHealth> },
    Failed { reason: String },
    ShuttingDown,
    Stopped,
}
```

Key lifecycle transitions:
- **Startup**: `Unconfigured → Validated → Starting → Healthy|Degraded|Failed`
- **Degraded state** preserves partial tool availability even when some MCP servers fail
- **Shutdown**: `Healthy|Degraded|Failed → ShuttingDown → Stopped`

## Builder Lessons

1. **Selector ambiguity is a user experience hazard.** ClawCode's `resolve_plugin_target` explicitly detects ambiguous name matches and escalates to an error rather than silently picking one. For destructive operations, require unambiguous identifiers.

2. **Display names and stable identifiers serve different purposes.** Display names are ergonomic for interactive use; stable ids are required for operations that affect plugin identity (uninstall, update). Confusing these leads to errors.

3. **Plugin lifecycle state is multi-dimensional.** A plugin isn't simply "running" or "stopped" — it has transitional states (`Starting`, `ShuttingDown`) and health sub-states (`Degraded` with `healthy_servers` vs `failed_servers`). Tool availability can persist in degraded mode.

4. **Aliasing commands is low-cost but high-value.** The `/plugins` and `/marketplace` aliases cost nothing at the implementation level but significantly improve discoverability and user expectations.

## Evidence Sources

- Plugin command parsing and aliases: `references/claw-code/rust/crates/commands/src/lib.rs` (lines 231-233, 1681–1742, 2183–2292, 2704–2720)
- Plugin lifecycle state definitions: `references/claw-code/rust/crates/runtime/src/plugin_lifecycle.rs` (PluginState enum)
- Plugin management in README: `references/claw-code/rust/README.md`
