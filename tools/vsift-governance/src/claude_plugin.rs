//! The Claude Code plugin rule (0.3.0, ADR 0038 section 4, the planner's D6).
//!
//! The repository is a Claude Code plugin marketplace: `.claude-plugin/
//! marketplace.json` lists one plugin, `plugins/vsift/`, which carries the
//! skill. Installing a plugin may run its hooks, start its MCP and LSP
//! servers and put its `bin/` on the shell's path; `VSift`'s plugin does none
//! of that, so installing it runs nothing. Neither manifest names a person:
//! the marketplace is fetched by anyone who adds it, and a name or address in
//! it would be published with every copy.
//!
//! The rule holds the two manifests to the reviewed shape, refuses every
//! file or folder under [`PLUGINS_DIRECTORY`] that would run code or add a
//! server (`hooks/`, `bin/`, `monitors/`, `.mcp.json`, `.lsp.json`,
//! `package.json`, any link).
//!
//! `plugin.json` carries no `version`. Claude Code keeps a person on a
//! plugin's stated version until it changes, so a stated version would have
//! to be bumped with every release, and the release tools allow a stable
//! release to differ from its candidate only in the files they list. Without
//! it the plugin follows its marketplace source, which the docs pin to the
//! release tag (`/plugin marketplace add smormah/vsift#v<version>`). That the
//! plugin's skill equals `skills/vsift/` byte for byte is the skill guard's
//! test (`skill_contract.rs`), run by `cargo test`.

use std::{fs, path::Path};

use serde_json::{Map, Value};

/// The marketplace manifest, at the marketplace root.
const MARKETPLACE_MANIFEST: &str = ".claude-plugin/marketplace.json";

/// The directory holding the marketplace's plugins.
pub(crate) const PLUGINS_DIRECTORY: &str = "plugins";

/// The one plugin's manifest.
const PLUGIN_MANIFEST: &str = "plugins/vsift/.claude-plugin/plugin.json";

/// The marketplace's, the plugin's and the entry's name.
const NAME: &str = "vsift";

/// The marketplace owner: the project, never a person.
const OWNER: &str = "VSift project";

/// The plugin entry's source, relative to the marketplace root.
const SOURCE: &str = "./plugins/vsift";

/// Members of the marketplace's top level the rule admits.
const MARKETPLACE_MEMBERS: [&str; 4] = ["name", "description", "owner", "plugins"];

/// Members of the plugin entry the rule admits: no component, so the plugin
/// folder alone says what is installed.
const ENTRY_MEMBERS: [&str; 3] = ["name", "source", "description"];

/// Members of `plugin.json` the rule admits: metadata only, with no author,
/// no `version` (the module's documentation says why) and no component that
/// runs code or adds a server.
const PLUGIN_MEMBERS: [&str; 4] = ["name", "description", "license", "repository"];

/// Names no entry under [`PLUGINS_DIRECTORY`] may have: each would run code
/// on install or on use, or add a server.
const FORBIDDEN_ENTRIES: [&str; 6] = [
    "hooks",
    "bin",
    "monitors",
    ".mcp.json",
    ".lsp.json",
    "package.json",
];

/// The most entries the walk visits; the plugin holds a few dozen.
const MAXIMUM_ENTRIES: usize = 1_000;

/// Checks the marketplace, the plugin manifest and the plugin folder, and
/// appends one message per finding.
pub(crate) fn validate_claude_plugin(messages: &mut Vec<String>, root: &Path) {
    match fs::read_to_string(root.join(MARKETPLACE_MANIFEST)) {
        Ok(text) => check_marketplace(messages, &text),
        Err(error) => messages.push(format!("{MARKETPLACE_MANIFEST} could not be read: {error}")),
    }
    match fs::read_to_string(root.join(PLUGIN_MANIFEST)) {
        Ok(text) => check_plugin(messages, &text),
        Err(error) => messages.push(format!("{PLUGIN_MANIFEST} could not be read: {error}")),
    }
    check_plugin_entries(messages, root);
}

/// Appends a finding for every member of `fields` not in `allowed`.
fn only_members(
    messages: &mut Vec<String>,
    at: &str,
    fields: &Map<String, Value>,
    allowed: &[&str],
) {
    for member in fields.keys() {
        if !allowed.contains(&member.as_str()) {
            messages.push(format!(
                "{at} declares `{member}`; the VSift plugin names no person and installs no \
                 component that runs code or adds a server (ADR 0038 section 4)"
            ));
        }
    }
}

/// Checks the marketplace manifest's text.
pub(crate) fn check_marketplace(messages: &mut Vec<String>, text: &str) {
    let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(text) else {
        messages.push(format!("{MARKETPLACE_MANIFEST} is not a JSON object"));
        return;
    };
    only_members(
        messages,
        MARKETPLACE_MANIFEST,
        &fields,
        &MARKETPLACE_MEMBERS,
    );
    if fields.get("name").and_then(Value::as_str) != Some(NAME) {
        messages.push(format!(
            "{MARKETPLACE_MANIFEST} must name the marketplace `{NAME}`"
        ));
    }
    match fields.get("owner") {
        Some(Value::Object(owner))
            if owner.len() == 1 && owner.get("name").and_then(Value::as_str) == Some(OWNER) => {}
        _ => messages.push(format!(
            "{MARKETPLACE_MANIFEST} must give the owner only as {{\"name\": \"{OWNER}\"}}: no \
             person's name, address or link (ADR 0038 section 4)"
        )),
    }
    let entries = match fields.get("plugins") {
        Some(Value::Array(entries)) if entries.len() == 1 => entries,
        _ => {
            messages.push(format!(
                "{MARKETPLACE_MANIFEST} must list exactly one plugin, `{NAME}`"
            ));
            return;
        }
    };
    let Some(Value::Object(entry)) = entries.first() else {
        messages.push(format!(
            "{MARKETPLACE_MANIFEST} plugins[0] is not an object"
        ));
        return;
    };
    let at = format!("{MARKETPLACE_MANIFEST} plugins[0]");
    only_members(messages, &at, entry, &ENTRY_MEMBERS);
    if entry.get("name").and_then(Value::as_str) != Some(NAME) {
        messages.push(format!(
            "{at} must be named `{NAME}`, as its plugin.json is"
        ));
    }
    if entry.get("source").and_then(Value::as_str) != Some(SOURCE) {
        messages.push(format!("{at} must have the source `{SOURCE}`"));
    }
}

/// Checks the plugin manifest's text.
pub(crate) fn check_plugin(messages: &mut Vec<String>, text: &str) {
    let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(text) else {
        messages.push(format!("{PLUGIN_MANIFEST} is not a JSON object"));
        return;
    };
    only_members(messages, PLUGIN_MANIFEST, &fields, &PLUGIN_MEMBERS);
    if fields.get("name").and_then(Value::as_str) != Some(NAME) {
        messages.push(format!("{PLUGIN_MANIFEST} must name the plugin `{NAME}`"));
    }
}

/// Walks [`PLUGINS_DIRECTORY`] and refuses every [`FORBIDDEN_ENTRIES`] name
/// and every link.
fn check_plugin_entries(messages: &mut Vec<String>, root: &Path) {
    let mut pending = vec![String::from(PLUGINS_DIRECTORY)];
    let mut visited = 0_usize;
    while let Some(relative) = pending.pop() {
        let entries = match fs::read_dir(root.join(&relative)) {
            Ok(entries) => entries,
            Err(error) => {
                messages.push(format!("{relative} could not be listed: {error}"));
                continue;
            }
        };
        for entry in entries {
            visited += 1;
            if visited > MAXIMUM_ENTRIES {
                messages.push(format!(
                    "{PLUGINS_DIRECTORY} holds more than {MAXIMUM_ENTRIES} entries"
                ));
                return;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    messages.push(format!("{relative} could not be listed: {error}"));
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{relative}/{name}");
            let Ok(kind) = entry.file_type() else {
                messages.push(format!("{path} could not be inspected"));
                continue;
            };
            if let Some(finding) = entry_finding(&path, &name, kind.is_symlink()) {
                messages.push(finding);
            } else if kind.is_dir() {
                pending.push(path);
            }
        }
    }
}

/// The finding for one entry of the plugin folder, if it is refused.
pub(crate) fn entry_finding(path: &str, name: &str, is_link: bool) -> Option<String> {
    if is_link {
        // Claude Code copies a plugin into its cache, and a Windows checkout
        // without symbolic links turns a link into a text file.
        return Some(format!(
            "{path} is a link; the plugin holds real copies (ADR 0038 section 4)"
        ));
    }
    FORBIDDEN_ENTRIES.contains(&name).then(|| {
        format!(
            "{path}: the VSift plugin ships no hooks, executables, monitors, MCP or LSP servers \
             or npm manifest, so installing it runs nothing (ADR 0038 section 4)"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{check_marketplace, check_plugin, entry_finding};

    fn marketplace(text: &str) -> Vec<String> {
        let mut messages = Vec::new();
        check_marketplace(&mut messages, text);
        messages
    }

    fn plugin(text: &str) -> Vec<String> {
        let mut messages = Vec::new();
        check_plugin(&mut messages, text);
        messages
    }

    const MARKETPLACE: &str = r#"{"name": "vsift", "description": "d",
        "owner": {"name": "VSift project"},
        "plugins": [{"name": "vsift", "source": "./plugins/vsift", "description": "d"}]}"#;

    const PLUGIN: &str = r#"{"name": "vsift", "description": "d",
        "license": "MIT OR Apache-2.0", "repository": "https://github.com/smormah/vsift"}"#;

    #[test]
    fn the_reviewed_manifests_are_accepted() {
        assert_eq!(marketplace(MARKETPLACE), Vec::<String>::new());
        assert_eq!(plugin(PLUGIN), Vec::<String>::new());
    }

    #[test]
    fn a_person_or_a_component_is_refused_in_either_manifest() {
        for (from, to) in [
            (
                r#""owner": {"name": "VSift project"}"#,
                r#""owner": {"name": "VSift project", "email": "person@example.com"}"#,
            ),
            (
                r#""owner": {"name": "VSift project"}"#,
                r#""owner": {"name": "A Person"}"#,
            ),
            (
                r#""description": "d","#,
                r#""author": {"name": "A Person"},"#,
            ),
            (
                r#""source": "./plugins/vsift","#,
                r#""source": "./plugins/vsift", "hooks": {},"#,
            ),
            (
                r#""source": "./plugins/vsift","#,
                r#""source": "./plugins/vsift", "mcpServers": {},"#,
            ),
            (r#""source": "./plugins/vsift""#, r#""source": "./other""#),
            (
                r#""name": "vsift", "description""#,
                r#""name": "other", "description""#,
            ),
        ] {
            let text = MARKETPLACE.replacen(from, to, 1);
            assert_ne!(text, MARKETPLACE, "{from}");
            assert!(!marketplace(&text).is_empty(), "{text}");
        }
        for member in [
            r#""author": {"name": "A Person"}"#,
            r#""hooks": "./hooks.json""#,
            r#""mcpServers": {}"#,
            r#""lspServers": {}"#,
            r#""userConfig": {}"#,
            r#""experimental": {}"#,
            r#""version": "0.3.0""#,
        ] {
            let text = PLUGIN.replacen('{', &format!("{{{member}, "), 1);
            assert!(!plugin(&text).is_empty(), "{text}");
        }
    }

    #[test]
    fn code_servers_and_links_are_refused_in_the_plugin_folder() {
        for name in [
            "hooks",
            "bin",
            "monitors",
            ".mcp.json",
            ".lsp.json",
            "package.json",
        ] {
            assert!(
                entry_finding(&format!("plugins/vsift/{name}"), name, false).is_some(),
                "{name}"
            );
        }
        assert!(entry_finding("plugins/vsift/skills", "skills", true).is_some());
        assert_eq!(entry_finding("plugins/vsift/skills", "skills", false), None);
        assert_eq!(
            entry_finding("plugins/vsift/commands", "commands", false),
            None
        );
    }
}
