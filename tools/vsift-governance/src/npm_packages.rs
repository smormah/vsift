//! The npm package rule (P13 PR 9, ADR 0023 section 2; the launcher boundary
//! of `implementation-work-packets.md`).
//!
//! No `VSift` npm package may run code when it is installed: a launcher that
//! fetched or built its executable in an install script would install cleanly
//! where scripts are disabled (Bun by default, pnpm and Yarn when configured,
//! many organisations) and then fail at first use, and installing would
//! download something the package does not hold (ADR 0007, ADR 0014). No
//! package manifest names a person either: the packages are published from
//! CI, and a name or address in a manifest would be published with them.
//!
//! Every `package.json` under [`NPM_DIRECTORY`] (the launcher's sources and
//! the qualification driver) must be a JSON object without `scripts`,
//! `gypfile`, `author`, `contributors` or `maintainers`; no `binding.gyp` may
//! exist there, because npm infers `"install": "node-gyp rebuild"` from it;
//! and the launcher manifest must pin each optional dependency to its own
//! exact version. The platform packages' manifests are written by
//! `tools/vsift-release`, which checks the packed tarballs the same way.

use std::{fs, path::Path};

use serde_json::Value;

/// The directory holding the npm packages' sources.
pub(crate) const NPM_DIRECTORY: &str = "npm";

/// The launcher's manifest, which must exist so the rule is never vacuous.
const LAUNCHER_MANIFEST: &str = "npm/vsift-cli/package.json";

/// The launcher package's npm name; the command it installs stays `vsift`.
const LAUNCHER_PACKAGE: &str = "vsift-cli";

/// Manifest fields no `VSift` package may declare.
const FORBIDDEN_FIELDS: [&str; 5] = [
    "scripts",
    "gypfile",
    "author",
    "contributors",
    "maintainers",
];

/// The most entries the walk visits; the directory holds a dozen.
const MAXIMUM_ENTRIES: usize = 1_000;

/// Checks every manifest under [`NPM_DIRECTORY`] and appends one message per
/// finding.
pub(crate) fn validate_npm_packages(messages: &mut Vec<String>, root: &Path) {
    let mut pending = vec![String::from(NPM_DIRECTORY)];
    let mut visited = 0_usize;
    let mut saw_launcher = false;
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
                    "{NPM_DIRECTORY} holds more than {MAXIMUM_ENTRIES} entries"
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
            if kind.is_dir() {
                // A developer's local install is not a source of the packages.
                if name != "node_modules" {
                    pending.push(path);
                }
            } else if name == "binding.gyp" {
                messages.push(format!(
                    "{path}: npm infers an install script (`node-gyp rebuild`) from a \
                     binding.gyp; no VSift package may run code on install (ADR 0023)"
                ));
            } else if name == "package.json" {
                saw_launcher |= path == LAUNCHER_MANIFEST;
                match fs::read_to_string(root.join(&path)) {
                    Ok(text) => check_manifest(messages, &path, &text),
                    Err(error) => messages.push(format!("{path} could not be read: {error}")),
                }
            }
        }
    }
    if !saw_launcher {
        messages.push(format!(
            "{LAUNCHER_MANIFEST} is missing; the npm package rule has nothing to hold"
        ));
    }
}

/// Checks one manifest's text; `relative_path` names it in messages.
pub(crate) fn check_manifest(messages: &mut Vec<String>, relative_path: &str, text: &str) {
    let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(text) else {
        messages.push(format!("{relative_path} is not a JSON object"));
        return;
    };
    for field in FORBIDDEN_FIELDS {
        if fields.contains_key(field) {
            messages.push(format!(
                "{relative_path} declares `{field}`; a VSift npm package runs no install, \
                 pack or publish script and names no person (ADR 0023)"
            ));
        }
    }
    if relative_path != LAUNCHER_MANIFEST {
        return;
    }
    // ADR 0023 decision A as amended on 2026-09-30: npm refused the unscoped
    // `vsift`, and the maintainer holds `vsift-cli`.
    if fields.get("name").and_then(Value::as_str) != Some(LAUNCHER_PACKAGE) {
        messages.push(format!(
            "{relative_path} must name the package `{LAUNCHER_PACKAGE}` (ADR 0023 decision A)"
        ));
    }
    let version = fields.get("version").and_then(Value::as_str);
    match fields.get("optionalDependencies") {
        Some(Value::Object(dependencies)) if !dependencies.is_empty() => {
            for (name, range) in dependencies {
                if range.as_str() != version || version.is_none() {
                    messages.push(format!(
                        "{relative_path} must pin `{name}` to the launcher's own exact version"
                    ));
                }
            }
        }
        _ => messages.push(format!(
            "{relative_path} must list the platform packages as `optionalDependencies`"
        )),
    }
    for field in [
        "dependencies",
        "peerDependencies",
        "bundleDependencies",
        "bundledDependencies",
    ] {
        if fields.contains_key(field) {
            messages.push(format!(
                "{relative_path} declares `{field}`; the launcher depends only on its platform \
                 packages"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LAUNCHER_MANIFEST, check_manifest};

    fn findings(path: &str, text: &str) -> Vec<String> {
        let mut messages = Vec::new();
        check_manifest(&mut messages, path, text);
        messages
    }

    const LAUNCHER: &str = r#"{"name": "vsift-cli", "version": "0.1.0",
        "optionalDependencies": {"@vsift/linux-x64": "0.1.0", "@vsift/win32-x64": "0.1.0"}}"#;

    #[test]
    fn the_reviewed_launcher_manifest_is_accepted() {
        assert_eq!(findings(LAUNCHER_MANIFEST, LAUNCHER), Vec::<String>::new());
    }

    #[test]
    fn install_scripts_and_people_are_refused_in_any_manifest() {
        for (field, value) in [
            ("scripts", r#"{"postinstall": "node install.js"}"#),
            ("scripts", r#"{"preinstall": "curl"}"#),
            ("scripts", r#"{"install": "node-gyp rebuild"}"#),
            ("scripts", r#"{"test": "node --test"}"#),
            ("gypfile", "true"),
            ("author", r#""A Person <person@example.com>""#),
            ("contributors", "[]"),
            ("maintainers", "[]"),
        ] {
            let text = LAUNCHER.replacen('{', &format!("{{\"{field}\": {value}, "), 1);
            for path in [LAUNCHER_MANIFEST, "npm/qualification/package.json"] {
                let messages = findings(path, &text);
                assert!(
                    messages.iter().any(|message| message.contains(field)),
                    "{path} {field}: {messages:?}"
                );
            }
        }
    }

    #[test]
    fn the_launcher_pins_its_platform_packages_exactly() {
        for manifest in [
            LAUNCHER.replacen(
                r#""@vsift/linux-x64": "0.1.0""#,
                r#""@vsift/linux-x64": "^0.1.0""#,
                1,
            ),
            LAUNCHER.replacen(
                r#""@vsift/linux-x64": "0.1.0""#,
                r#""@vsift/linux-x64": "0.1.1""#,
                1,
            ),
            LAUNCHER.replacen("optionalDependencies", "dependencies", 1),
            String::from(r#"{"name": "vsift-cli", "version": "0.1.0"}"#),
            LAUNCHER.replacen(r#""name": "vsift-cli""#, r#""name": "vsift""#, 1),
            String::from("[]"),
        ] {
            assert!(
                !findings(LAUNCHER_MANIFEST, &manifest).is_empty(),
                "{manifest}"
            );
        }
    }
}
