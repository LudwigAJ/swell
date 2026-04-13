# Config Merged Sections

The `ConfigLoader::load()` in `runtime/src/config.rs` does not stop at scalar keys. After discovering the five config files in precedence order, it performs a deep merge of the entire object tree and then feeds the flattened result into a rich `RuntimeFeatureConfig` struct that covers hooks, MCP servers, OAuth, aliases, permission rules, sandbox settings, provider fallbacks, and trusted roots. The merged result is what the runtime consumes — not just the selected model.

This document explains how each section is merged and what the design enables for a builder.

## What RuntimeFeatureConfig contains after merge

The `RuntimeFeatureConfig` struct (defined in `runtime/src/config.rs`) holds the parsed results of the merged configuration:

```rust
pub struct RuntimeFeatureConfig {
    hooks: RuntimeHookConfig,
    plugins: RuntimePluginConfig,
    mcp: McpConfigCollection,
    oauth: Option<OAuthConfig>,
    model: Option<String>,
    aliases: BTreeMap<String, String>,
    permission_mode: Option<ResolvedPermissionMode>,
    permission_rules: RuntimePermissionRuleConfig,
    sandbox: SandboxConfig,
    provider_fallbacks: ProviderFallbackConfig,
    trusted_roots: Vec<String>,
}
```

Each field is independently resolved from the merged JSON. The fields that are scalar values (`model`, `permission_mode`) follow plain override semantics — the last non-null value wins. The fields that are collections or nested objects use more nuanced merge strategies described below.

## Deep merge for object sections

`deep_merge_objects()` performs a recursive BTreeMap merge:

```rust
fn deep_merge_objects(target: &mut BTreeMap<String, JsonValue>, source: &BTreeMap<String, JsonValue>) {
    for (key, value) in source {
        match (target.get_mut(key), value) {
            (Some(JsonValue::Object(existing)), JsonValue::Object(incoming)) => {
                deep_merge_objects(existing, incoming); // recurse
            }
            _ => {
                target.insert(key.clone(), value.clone()); // override
            }
        }
    }
}
```

This means nested objects — `env`, `hooks`, `sandbox`, `permissions`, `plugins` — are merged field-by-field rather than wholesale-replaced. A later config file can add or override individual fields inside these objects while preserving sibling fields from earlier files.

The test `deep_merge_objects_merges_nested_maps` in `runtime/src/config.rs` demonstrates this with a concrete case: `{"env":{"A":"1","B":"2"},"model":"haiku"}` merged with `{"env":{"B":"override","C":"3"},"sandbox":{"enabled":true}}` yields an `env` object that retains `A`, overrides `B`, and adds `C` — without losing any field.

**Builder lesson:** This pattern of recursive deep merge is the idiomatic way to support partial overrides across configuration layers. A builder who naively replaced whole objects would break cases like "project sets a sandbox flag but inherits the user's allowed mounts." ClawCode's approach makes each layer additive where objects overlap.

## Hooks: unique-extension merge

Hook commands (`pre_tool_use`, `post_tool_use`, `post_tool_use_failure`) use a distinct strategy: **unique-extension** rather than override or deep-merge.

`RuntimeHookConfig::extend()` calls `extend_unique()` for each hook list:

```rust
fn extend_unique(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.iter().any(|existing| existing == &value) {
            target.push(value);
        }
    }
}
```

This means if the user config adds `["pre-check", "guard"]` and the project config adds `["guard", "audit-log"]`, the merged result is `["pre-check", "guard", "audit-log"]` — duplicates are dropped and order is preserved from earliest to latest source.

The test `hook_config_merge_preserves_uniques` confirms this with `base.merged(&overlay)` producing union-lists for each hook stage.

**Builder lesson:** Unique-extension is the right merge policy for command lists where every source legitimately wants to contribute and no source should silently silence another. An override policy would make it too easy for a project config to accidentally disable a user's safety hooks.

## MCP servers: scope-tracked replacement by name

MCP server definitions are handled by `merge_mcp_servers()`, which builds a `BTreeMap<String, ScopedMcpServerConfig>`:

```rust
fn merge_mcp_servers(
    target: &mut BTreeMap<String, ScopedMcpServerConfig>,
    source: ConfigSource,
    root: &BTreeMap<String, JsonValue>,
    path: &Path,
) -> Result<(), ConfigError> {
    let Some(mcp_servers) = root.get("mcpServers") else { return Ok(()); };
    let servers = expect_object(mcp_servers, ...)?;
    for (name, value) in servers {
        let parsed = parse_mcp_server_config(name, value, ...)?;
        target.insert(name.clone(), ScopedMcpServerConfig { scope: source, config: parsed });
    }
    Ok(())
}
```

Servers are keyed by name. If two config files both define `remote-server`, the later file's `ScopedMcpServerConfig` replaces the earlier one entirely (not deep-merged field-by-field), but the `scope` field on each entry records which config layer it came from. This lets downstream code — the MCP bridge in `runtime/src/mcp_tool_bridge.rs` — know whether a given server definition originated from user, project, or local config.

The test `parses_typed_mcp_and_oauth_config` demonstrates this: after loading both user settings (with `remote-server` as type `http`) and local settings (with `remote-server` as type `ws`), the merged collection has exactly one `remote-server` entry, with scope `Local` and transport `Ws`. The local entry fully replaced the user entry because the top-level key `mcpServers` was overwritten at the server-name level.

**Builder lesson:** When a config section contains named entries (servers, plugins, aliases), the merge key is the entry name, not the section name. This is the right granularity for per-entry override: project config can replace a user's `remote-server` transport type without touching other user-defined servers.

## Aliases: BTreeMap string-map merge

Model aliases are parsed via `parse_optional_aliases()` into a `BTreeMap<String, String>` via the deep merge of the top-level `aliases` key. The `deep_merge_objects` call on the root config means if user config has `{"aliases":{"fast":"haiku","smart":"opus"}}` and local config has `{"aliases":{"smart":"sonnet","cheap":"grok"}}`, the result is `{"fast":"haiku","smart":"sonnet","cheap":"grok"}` — the later file's `smart` replaces the earlier one's, but the earlier file's `fast` is preserved.

The test `parses_user_defined_model_aliases_from_settings` covers this case explicitly.

## Permission mode and rules

`permission_mode` (a top-level scalar) is resolved after merge by checking either `permissionMode` or `permissions.defaultMode` in the merged JSON. Later values override earlier ones — the test `loads_and_merges_claude_code_config_files_by_precedence` confirms that `settings.local.json`'s `"permissionMode":"acceptEdits"` wins over `settings.json`'s `"permissions":{"defaultMode":"plan"}`.

`permission_rules` (the `allow`, `deny`, `ask` arrays inside `permissions`) are extracted from the merged `permissions` object. Because `permissions` is an object that goes through deep merge, later arrays **replace** earlier arrays for the same key — unlike hooks, permission rule lists do not extend uniquely. The test confirms this: the project's `permissions.ask` of `["Edit"]` replaces whatever the user config may have set for `ask`.

**Builder lesson:** Permission rule lists use replacement semantics because allowing or denying specific operations is an intentional, ordered decision. Accumulating rules from all layers would make it easy to accidentally grant more access than intended. Unique-extension would also be dangerous here — a project would not want a user's deny list to silently shadow its own allow list.

## Sandbox configuration

Sandbox settings are parsed from `sandbox` inside the merged JSON via `parse_optional_sandbox_config()`. Because the `sandbox` object goes through deep merge, a later config file can set `sandbox.enabled` while an earlier file sets `sandbox.filesystemMode`, and both fields are preserved in the final result.

The test `parses_sandbox_config` demonstrates this with a local settings file that sets all five sandbox fields — each field is individually preserved in the parsed result, confirming deep-merge behavior.

The `SandboxConfig` struct captures `enabled`, `namespace_restrictions`, `network_isolation`, `filesystem_mode`, and `allowed_mounts` as independently optional fields.

## Provider fallbacks

`provider_fallbacks` is parsed into a `ProviderFallbackConfig` struct with a `primary` (optional string) and an ordered `fallbacks` (vec of strings). Both are extracted from the merged `provider_fallbacks` object. Because `provider_fallbacks` itself is a top-level key, later config files replace the entire `ProviderFallbackConfig` rather than extending the fallback list. The test `parses_provider_fallbacks_chain_with_primary_and_ordered_fallbacks` confirms the full chain `primary: claude-opus-4-6, fallbacks: [grok-3, grok-3-mini]` is parsed intact from a user settings file.

## Trusted roots

`trusted_roots` is a string array extracted from `trustedRoots` in the merged config. It is not accumulated via unique-extension; later arrays replace earlier ones. The test `parses_trusted_roots_from_settings` confirms the list is populated and `trusted_roots_default_is_empty_when_unset` confirms it starts empty when no config sets it.

## OAuth

The runtime-level `OAuthConfig` is parsed from `oauth` in the merged JSON. Because `oauth` is an object top-level key, later configs fully replace earlier ones rather than deep-merging individual OAuth fields. The test `parses_typed_mcp_and_oauth_config` covers this with a user settings file that defines a complete `OAuthConfig` with `clientId`, `authorizeUrl`, `tokenUrl`, `callbackPort`, `manualRedirectUrl`, and `scopes`.

## Plugins

Plugin configuration is more complex, with two possible config paths: `enabledPlugins` (a flat bool map) and the nested `plugins` object. Both go through deep merge on the root config, so `enabledPlugins` from a later file can override individual plugin bool values from an earlier file, and the `plugins` object fields (like `installRoot`, `registryPath`) are individually overridable.

The test `parses_plugin_config` confirms that `externalDirectories`, `installRoot`, `registryPath`, `bundledRoot`, and `maxOutputTokens` are all independently extracted from the merged `plugins` object after deep merge has been applied.

## Builder takeaways

| Section | Merge strategy | Key insight |
|---|---|---|
| Object sections (`env`, `sandbox`) | Deep recursive merge | Later files add/override fields; siblings are preserved |
| Hooks | Unique-extension on arrays | Every layer contributes; no silent disabling |
| MCP servers | Replace by server name | Per-server override granularity |
| Aliases | Deep merge on `aliases` object | Per-alias override granularity |
| Permission rules | Replace entire array | Deny/allow lists are intentionally ordered |
| Provider fallbacks | Replace entire struct | Fallback chain is a unit; no partial override |
| Trusted roots | Replace entire array | Trust is a binary decision per root |
| OAuth | Replace entire object | OAuth config is a unit |
| Plugins | Deep merge on `plugins` + bool-map on `enabledPlugins` | Per-plugin bool and per-field plugin paths |

The design principle is: **the right merge policy is chosen per section based on whether partial contribution or full replacement makes sense for that feature**. Scalar overrides are trivial. Collections of commands or operations (hooks) need additive semantics. Named-entry collections (servers, aliases) need per-name replacement. Policy lists (permissions, trusted roots) need full replacement to avoid ambiguous outcomes.

This table-driven per-section merge policy is more work than a single universal strategy, but it avoids the class of bug where a project-level config accidentally disables a user's safety hook or silently expands a deny list.
