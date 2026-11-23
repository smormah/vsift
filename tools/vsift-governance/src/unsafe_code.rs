//! The `unsafe` rule (AGENTS.md, ADR 0026): only
//! [`UNSAFE_CRATE`] may contain `unsafe` code, and only as ADR 0026 allows it.
//!
//! - The workspace forbids `unsafe_code`; every other package takes the
//!   workspace's lints (`[lints] workspace = true`, which no target can relax),
//!   and each crate under `crates/` also carries `#![forbid(unsafe_code)]` in its
//!   `src/lib.rs` and `src/main.rs`.
//! - No Rust source outside [`UNSAFE_CRATE`] holds the `unsafe` keyword, in
//!   `crates/`, `tools/` or `fuzz/` (comments and string literals are not
//!   code and do not count).
//! - [`UNSAFE_CRATE`] repeats the workspace's lints with `unsafe_code`
//!   denied (allowed only at each block), `unsafe_op_in_unsafe_fn` denied, and
//!   Clippy's `undocumented_unsafe_blocks` and `multiple_unsafe_ops_per_block`
//!   denied; every `unsafe` in it follows a `// SAFETY:` comment; and it holds at
//!   most [`MAX_UNSAFE_BLOCKS`], so a new one is a deliberate change of this
//!   bound and of ADR 0026, reviewed as the ADR says.

use std::{fs, path::Path};

/// The one crate allowed `unsafe` code (ADR 0026).
pub(crate) const UNSAFE_CRATE: &str = "crates/vsift-windows-security";
/// How many `unsafe` blocks ADR 0026 accepted in [`UNSAFE_CRATE`].
///
/// - Three for its decision (2026-10-19): building the security descriptor,
///   creating the directory, and freeing the descriptor.
/// - Ten for its amendment of 2026-10-27 (L-229), starting the background
///   runner with only its null handle inherited: marking that handle
///   inheritable, sizing and initialising the attribute list, naming the
///   handle in it, creating the process, deleting the list, owning the
///   returned handles, waiting for the process, reading its exit code, and
///   ending it.
const MAX_UNSAFE_BLOCKS: usize = 13;
/// The folders whose Rust sources are checked.
const SOURCE_DIRECTORIES: [&str; 3] = ["crates", "tools", "fuzz"];
/// The folders whose packages must take the workspace's lints.
const PACKAGE_DIRECTORIES: [&str; 2] = ["crates", "tools"];
/// A bound on the folders and files walked, so a link loop or a huge tree
/// cannot stall the check.
const MAXIMUM_ENTRIES: usize = 20_000;
/// Lints [`UNSAFE_CRATE`] must set beyond the workspace's Clippy table.
const REQUIRED_RUST_LINTS: [&str; 3] = [
    "missing_docs = \"warn\"",
    "unsafe_code = \"deny\"",
    "unsafe_op_in_unsafe_fn = \"deny\"",
];
const REQUIRED_CLIPPY_LINTS: [&str; 2] = [
    "undocumented_unsafe_blocks = \"deny\"",
    "multiple_unsafe_ops_per_block = \"deny\"",
];
/// Attributes [`UNSAFE_CRATE`]'s root must carry, so the rules hold even if
/// its manifest's lint table is edited.
const REQUIRED_ROOT_ATTRIBUTES: [&str; 2] = [
    "#![deny(unsafe_op_in_unsafe_fn)]",
    "#![deny(clippy::undocumented_unsafe_blocks)]",
];
const FORBID_ATTRIBUTE: &str = "#![forbid(unsafe_code)]";

/// Runs every part of the rule over the repository at `root`.
pub(crate) fn validate_unsafe_code(messages: &mut Vec<String>, root: &Path) {
    let workspace = match fs::read_to_string(root.join("Cargo.toml")) {
        Ok(text) => text,
        Err(error) => {
            messages.push(format!("Cargo.toml could not be read: {error}"));
            return;
        }
    };
    check_workspace_manifest(messages, &workspace);
    for directory in PACKAGE_DIRECTORIES {
        let Some(names) = list(messages, root, directory) else {
            continue;
        };
        for name in names {
            let package = format!("{directory}/{name}");
            let manifest = format!("{package}/Cargo.toml");
            let Ok(text) = fs::read_to_string(root.join(&manifest)) else {
                continue;
            };
            if package == UNSAFE_CRATE {
                check_unsafe_crate_manifest(messages, &manifest, &text, &workspace);
                let lib = format!("{package}/src/lib.rs");
                match fs::read_to_string(root.join(&lib)) {
                    Ok(source) => check_unsafe_crate_root(messages, &lib, &source),
                    Err(error) => messages.push(format!("{lib} could not be read: {error}")),
                }
            } else {
                check_package_manifest(messages, &manifest, &text);
                // The shipped crates also say it in their roots. The tools rely
                // on the workspace lint alone: the agent-trial harness's
                // sources are frozen with each trial batch.
                let roots: &[&str] = if directory == "crates" {
                    &["src/lib.rs", "src/main.rs"]
                } else {
                    &[]
                };
                for target in roots {
                    let path = format!("{package}/{target}");
                    if let Ok(source) = fs::read_to_string(root.join(&path)) {
                        check_crate_root(messages, &path, &source);
                    }
                }
            }
        }
    }
    let mut unsafe_blocks = 0_usize;
    let mut walked = 0_usize;
    for directory in SOURCE_DIRECTORIES {
        let mut pending = vec![directory.to_owned()];
        while let Some(relative) = pending.pop() {
            let Ok(entries) = fs::read_dir(root.join(&relative)) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                walked += 1;
                if walked > MAXIMUM_ENTRIES {
                    messages.push(format!(
                        "more than {MAXIMUM_ENTRIES} entries under the source folders; the unsafe \
                         rule stopped walking"
                    ));
                    return;
                }
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                let path = format!("{relative}/{name}");
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    if !(name.starts_with('.') || name == "target" || name == "node_modules") {
                        pending.push(path);
                    }
                } else if kind.is_file()
                    && Path::new(&name)
                        .extension()
                        .is_some_and(|extension| extension == "rs")
                {
                    match fs::read_to_string(root.join(&path)) {
                        Ok(source) => {
                            unsafe_blocks += check_source(messages, &path, &source);
                        }
                        Err(error) => messages.push(format!("{path} could not be read: {error}")),
                    }
                }
            }
        }
    }
    check_unsafe_block_count(messages, unsafe_blocks);
}

fn list(messages: &mut Vec<String>, root: &Path, directory: &str) -> Option<Vec<String>> {
    match fs::read_dir(root.join(directory)) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect();
            names.sort();
            Some(names)
        }
        Err(error) => {
            messages.push(format!("{directory} could not be listed: {error}"));
            None
        }
    }
}

/// The lines of the TOML table `[name]`, trimmed, without comments or blanks.
fn table<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("[{name}]");
    let mut inside = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == header;
            continue;
        }
        if inside && !trimmed.is_empty() && !trimmed.starts_with('#') {
            lines.push(trimmed);
        }
    }
    lines
}

pub(crate) fn check_workspace_manifest(messages: &mut Vec<String>, text: &str) {
    if !table(text, "workspace.lints.rust").contains(&"unsafe_code = \"forbid\"") {
        messages.push(String::from(
            "Cargo.toml must keep `unsafe_code = \"forbid\"` in [workspace.lints.rust] (AGENTS.md, \
             ADR 0026)",
        ));
    }
}

pub(crate) fn check_package_manifest(messages: &mut Vec<String>, manifest: &str, text: &str) {
    if !table(text, "lints").contains(&"workspace = true") {
        messages.push(format!(
            "{manifest} must take the workspace's lints (`[lints] workspace = true`), which forbid \
             `unsafe_code`; only {UNSAFE_CRATE} may differ (ADR 0026)"
        ));
    }
}

pub(crate) fn check_crate_root(messages: &mut Vec<String>, path: &str, source: &str) {
    if !source.lines().any(|line| line.trim() == FORBID_ATTRIBUTE) {
        messages.push(format!(
            "{path} must carry `{FORBID_ATTRIBUTE}`; only {UNSAFE_CRATE} may contain unsafe code \
             (ADR 0026)"
        ));
    }
}

pub(crate) fn check_unsafe_crate_manifest(
    messages: &mut Vec<String>,
    manifest: &str,
    text: &str,
    workspace: &str,
) {
    if table(text, "lints").contains(&"workspace = true") {
        messages.push(format!(
            "{manifest} cannot take the workspace's lints: they forbid the unsafe code ADR 0026 \
             allows here, so it repeats them"
        ));
    }
    let rust = table(text, "lints.rust");
    for required in REQUIRED_RUST_LINTS {
        if !rust.contains(&required) {
            messages.push(format!(
                "{manifest} must set `{required}` in [lints.rust] (ADR 0026)"
            ));
        }
    }
    let clippy = table(text, "lints.clippy");
    let workspace_clippy = table(workspace, "workspace.lints.clippy");
    for required in workspace_clippy
        .iter()
        .copied()
        .chain(REQUIRED_CLIPPY_LINTS)
    {
        if !clippy.contains(&required) {
            messages.push(format!(
                "{manifest} must set `{required}` in [lints.clippy] (ADR 0026)"
            ));
        }
    }
}

pub(crate) fn check_unsafe_crate_root(messages: &mut Vec<String>, path: &str, source: &str) {
    for required in REQUIRED_ROOT_ATTRIBUTES {
        if !source.lines().any(|line| line.trim() == required) {
            messages.push(format!("{path} must carry `{required}` (ADR 0026)"));
        }
    }
}

/// Checks one Rust source and returns how many `unsafe` it holds when it is
/// in [`UNSAFE_CRATE`] (anywhere else every one is a finding).
pub(crate) fn check_source(messages: &mut Vec<String>, path: &str, source: &str) -> usize {
    let code = code_only(source);
    let original: Vec<&str> = source.lines().collect();
    let allowed = path.starts_with(&format!("{UNSAFE_CRATE}/"));
    let mut count = 0_usize;
    for (index, line) in code.lines().enumerate() {
        for _ in 0..unsafe_keywords(line) {
            if !allowed {
                messages.push(format!(
                    "{path} line {} uses `unsafe`; only {UNSAFE_CRATE} may (AGENTS.md, ADR 0026)",
                    index + 1
                ));
                continue;
            }
            count += 1;
            if !has_safety_comment(&original, index) {
                messages.push(format!(
                    "{path} line {}: an `unsafe` without a `// SAFETY:` comment before it \
                     (ADR 0026)",
                    index + 1
                ));
            }
        }
    }
    count
}

pub(crate) fn check_unsafe_block_count(messages: &mut Vec<String>, count: usize) {
    if count > MAX_UNSAFE_BLOCKS {
        messages.push(format!(
            "{UNSAFE_CRATE} holds {count} `unsafe`, more than the {MAX_UNSAFE_BLOCKS} ADR 0026 \
             accepted; a new one needs the ADR's review and this bound raised with it"
        ));
    }
}

/// Whether the statement holding line `index` is preceded by a `// SAFETY:`
/// comment, with only attributes, other comment lines and the statement's own
/// earlier lines between them.
fn has_safety_comment(lines: &[&str], index: usize) -> bool {
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        let line = lines.get(cursor).map_or("", |line| line.trim());
        if line.starts_with("// SAFETY:") {
            return true;
        }
        if line.starts_with("//") || line.starts_with("#[") {
            continue;
        }
        return false;
    }
    false
}

/// How many times the keyword `unsafe` stands as a whole identifier in `line`.
fn unsafe_keywords(line: &str) -> usize {
    let bytes = line.as_bytes();
    let is_identifier = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    line.match_indices("unsafe")
        .filter(|(start, word)| {
            let before = start
                .checked_sub(1)
                .and_then(|position| bytes.get(position))
                .is_some_and(|byte| is_identifier(*byte));
            let after = bytes
                .get(start + word.len())
                .is_some_and(|byte| is_identifier(*byte));
            !before && !after
        })
        .count()
}

/// `source` with every comment and string or character literal replaced by
/// spaces (line breaks kept, so line numbers stay), leaving only code.
pub(crate) fn code_only(source: &str) -> String {
    let characters: Vec<char> = source.chars().collect();
    let mut code = String::with_capacity(source.len());
    let mut index = 0_usize;
    let blank = |character: char| if character == '\n' { '\n' } else { ' ' };
    while let Some(&current) = characters.get(index) {
        let next = characters.get(index + 1).copied();
        if current == '/' && next == Some('/') {
            while let Some(&character) = characters.get(index) {
                if character == '\n' {
                    break;
                }
                code.push(' ');
                index += 1;
            }
        } else if current == '/' && next == Some('*') {
            let mut depth = 0_usize;
            while let Some(&character) = characters.get(index) {
                let following = characters.get(index + 1).copied();
                if character == '/' && following == Some('*') {
                    depth += 1;
                    code.push_str("  ");
                    index += 2;
                } else if character == '*' && following == Some('/') {
                    depth = depth.saturating_sub(1);
                    code.push_str("  ");
                    index += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    code.push(blank(character));
                    index += 1;
                }
            }
        } else if let Some(hashes) = raw_string_start(&characters, index) {
            // r"…", r#"…"#, br"…": no escapes; ends at a quote and the hashes.
            let opening = characters
                .get(index..)
                .and_then(|rest| rest.iter().position(|character| *character == '"'))
                .map_or(characters.len(), |offset| index + offset);
            while index <= opening && index < characters.len() {
                code.push(' ');
                index += 1;
            }
            while let Some(&character) = characters.get(index) {
                let closes = character == '"'
                    && (1..=hashes).all(|offset| characters.get(index + offset) == Some(&'#'));
                if closes {
                    for _ in 0..=hashes {
                        code.push(' ');
                    }
                    index += hashes + 1;
                    break;
                }
                code.push(blank(character));
                index += 1;
            }
        } else if current == '"' {
            code.push(' ');
            index += 1;
            while let Some(&character) = characters.get(index) {
                if character == '\\' {
                    code.push(' ');
                    if let Some(&escaped) = characters.get(index + 1) {
                        code.push(blank(escaped));
                    }
                    index += 2;
                    continue;
                }
                code.push(blank(character));
                index += 1;
                if character == '"' {
                    break;
                }
            }
        } else if current == '\'' {
            let length = character_literal_length(&characters, index);
            for offset in 0..length.max(1) {
                let character = characters.get(index + offset).copied().unwrap_or(' ');
                code.push(if length == 0 {
                    character
                } else {
                    blank(character)
                });
            }
            index += length.max(1);
        } else {
            code.push(current);
            index += 1;
        }
    }
    code
}

/// The number of `#` of a raw string literal starting at `index`, or `None`.
fn raw_string_start(characters: &[char], index: usize) -> Option<usize> {
    let previous_is_identifier = index
        .checked_sub(1)
        .and_then(|position| characters.get(position))
        .is_some_and(|character| character.is_alphanumeric() || *character == '_');
    if previous_is_identifier {
        return None;
    }
    let mut cursor = index;
    if characters.get(cursor) == Some(&'b') || characters.get(cursor) == Some(&'c') {
        cursor += 1;
    }
    if characters.get(cursor) != Some(&'r') {
        return None;
    }
    cursor += 1;
    let mut hashes = 0_usize;
    while characters.get(cursor) == Some(&'#') {
        hashes += 1;
        cursor += 1;
    }
    (characters.get(cursor) == Some(&'"')).then_some(hashes)
}

/// The length of a character literal starting at `index` (`'x'`, `'\n'`,
/// `'\u{1F600}'`), or 0 for a lifetime or label (`'a`).
fn character_literal_length(characters: &[char], index: usize) -> usize {
    match characters.get(index + 1) {
        // The escaped character itself may be a quote (`'\''`), so the search
        // for the closing quote starts after it.
        Some('\\') => characters
            .get(index + 3..)
            .and_then(|rest| {
                rest.iter()
                    .take(12)
                    .position(|character| *character == '\'')
            })
            .map_or(0, |offset| offset + 4),
        Some(_) if characters.get(index + 2) == Some(&'\'') => 3,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, path::PathBuf};

    use super::{
        UNSAFE_CRATE, check_crate_root, check_package_manifest, check_source,
        check_unsafe_block_count, check_unsafe_crate_manifest, check_unsafe_crate_root,
        check_workspace_manifest, code_only, validate_unsafe_code,
    };

    fn repository() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn read(path: &str) -> Result<String, Box<dyn Error>> {
        Ok(fs::read_to_string(repository().join(path))?)
    }

    /// Replaces exactly one occurrence of `from` in `text`.
    fn mutate(text: &str, from: &str, to: &str) -> Result<String, Box<dyn Error>> {
        if text.matches(from).count() != 1 {
            return Err(format!("expected one {from:?}").into());
        }
        Ok(text.replacen(from, to, 1))
    }

    #[test]
    fn the_repository_follows_the_rule() {
        let mut messages = Vec::new();
        validate_unsafe_code(&mut messages, &repository());
        assert!(messages.is_empty(), "{messages:#?}");
    }

    /// Comments, strings, raw strings, characters and lifetimes are not code:
    /// the word in them is not the keyword, and line numbers are kept.
    #[test]
    fn only_code_is_searched() {
        let source = concat!(
            "// unsafe { }\n",
            "/* unsafe /* nested unsafe */ unsafe */\n",
            "let a = \"unsafe { \\\" unsafe\";\n",
            "let b = r#\"unsafe \"# ;\n",
            "let c = br\"unsafe\";\n",
            "let d = '\"'; let e = '\\''; fn f<'a>(x: &'a str) {}\n",
            "#![forbid(unsafe_code)] let unsafe_name = 1;\n",
            "let g = unsafe { h() };\n",
        );
        let code = code_only(source);
        assert_eq!(code.lines().count(), source.lines().count());
        let mut messages = Vec::new();
        check_source(&mut messages, "crates/vsift/src/lib.rs", source);
        assert_eq!(messages.len(), 1, "{messages:#?}");
        assert!(messages[0].contains("line 8"), "{messages:#?}");
    }

    /// Each part of the rule refuses a deliberately broken copy of the real
    /// file it reads.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one auditable list: each part of the rule and the broken copy it must refuse"
    )]
    fn each_part_refuses_a_broken_copy_of_the_real_files() -> Result<(), Box<dyn Error>> {
        let workspace = read("Cargo.toml")?;
        let infrastructure_manifest = read("crates/vsift-infrastructure/Cargo.toml")?;
        let infrastructure_root = read("crates/vsift-infrastructure/src/lib.rs")?;
        let exception_manifest = read(&format!("{UNSAFE_CRATE}/Cargo.toml"))?;
        let exception_root = read(&format!("{UNSAFE_CRATE}/src/lib.rs"))?;
        let exception_create = read(&format!("{UNSAFE_CRATE}/src/create.rs"))?;
        let create_path = format!("{UNSAFE_CRATE}/src/create.rs");
        let exception_detached = read(&format!("{UNSAFE_CRATE}/src/detached.rs"))?;
        let detached_path = format!("{UNSAFE_CRATE}/src/detached.rs");

        let mut messages = Vec::new();
        check_workspace_manifest(&mut messages, &workspace);
        check_package_manifest(&mut messages, "m", &infrastructure_manifest);
        check_crate_root(&mut messages, "r", &infrastructure_root);
        check_unsafe_crate_manifest(&mut messages, "e", &exception_manifest, &workspace);
        check_unsafe_crate_root(&mut messages, "e", &exception_root);
        let create_count = check_source(&mut messages, &create_path, &exception_create);
        let detached_count = check_source(&mut messages, &detached_path, &exception_detached);
        check_unsafe_block_count(&mut messages, create_count + detached_count);
        assert!(messages.is_empty(), "the real files: {messages:#?}");
        assert_eq!((create_count, detached_count), (3, 10));

        let cases: Vec<(&str, Vec<String>)> = vec![
            ("an unsafe block in another crate", {
                let mut found = Vec::new();
                let broken = format!(
                    "{infrastructure_root}\n// SAFETY: none.\nfn f() {{ unsafe {{ g() }} }}\n"
                );
                check_source(
                    &mut found,
                    "crates/vsift-infrastructure/src/lib.rs",
                    &broken,
                );
                found
            }),
            ("a package that does not take the workspace's lints", {
                let mut found = Vec::new();
                let broken = mutate(&infrastructure_manifest, "[lints]\nworkspace = true", "")?;
                check_package_manifest(&mut found, "m", &broken);
                found
            }),
            ("a crate root without its forbid", {
                let mut found = Vec::new();
                let broken = mutate(&infrastructure_root, "#![forbid(unsafe_code)]", "")?;
                check_crate_root(&mut found, "r", &broken);
                found
            }),
            ("a workspace that only denies unsafe code", {
                let mut found = Vec::new();
                let broken = mutate(
                    &workspace,
                    "unsafe_code = \"forbid\"",
                    "unsafe_code = \"deny\"",
                )?;
                check_workspace_manifest(&mut found, &broken);
                found
            }),
            ("the exception without its SAFETY comment", {
                let mut found = Vec::new();
                let broken = mutate(
                    &exception_create,
                    "    // SAFETY: `wide_path`",
                    "    // `wide_path`",
                )?;
                check_source(&mut found, &create_path, &broken);
                found
            }),
            ("a fourteenth unsafe block in the exception", {
                let mut found = Vec::new();
                let broken = format!(
                    "{exception_detached}\n// SAFETY: a new block.\nfn extra() {{ unsafe {{ g() }} }}\n"
                );
                let count = check_source(&mut found, &create_path, &exception_create)
                    + check_source(&mut found, &detached_path, &broken);
                check_unsafe_block_count(&mut found, count);
                found
            }),
            ("the runner's launch without its SAFETY comment", {
                let mut found = Vec::new();
                let broken = mutate(
                    &exception_detached,
                    "    // SAFETY: `application` and `directory`",
                    "    // `application` and `directory`",
                )?;
                check_source(&mut found, &detached_path, &broken);
                found
            }),
            ("the exception without the undocumented-block lint", {
                let mut found = Vec::new();
                let broken = mutate(
                    &exception_manifest,
                    "undocumented_unsafe_blocks = \"deny\"\n",
                    "",
                )?;
                check_unsafe_crate_manifest(&mut found, "e", &broken, &workspace);
                found
            }),
            ("the exception allowing unsafe code everywhere", {
                let mut found = Vec::new();
                let broken = mutate(
                    &exception_manifest,
                    "unsafe_code = \"deny\"",
                    "unsafe_code = \"allow\"",
                )?;
                check_unsafe_crate_manifest(&mut found, "e", &broken, &workspace);
                found
            }),
            ("the exception dropping a workspace lint", {
                let mut found = Vec::new();
                let broken = mutate(&exception_manifest, "unwrap_used = \"deny\"\n", "")?;
                check_unsafe_crate_manifest(&mut found, "e", &broken, &workspace);
                found
            }),
            ("the exception's root without its deny", {
                let mut found = Vec::new();
                let broken = mutate(&exception_root, "#![deny(unsafe_op_in_unsafe_fn)]", "")?;
                check_unsafe_crate_root(&mut found, "e", &broken);
                found
            }),
        ];
        for (case, found) in cases {
            assert!(!found.is_empty(), "{case}: the broken copy passed");
        }
        Ok(())
    }
}
