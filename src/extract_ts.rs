use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Structured representation of a single TypeScript export item.
struct TsItem {
    kind: &'static str, // "function", "class", "interface", "type", "enum", "const", "reexport", "default"
    jsdoc: String,
    declaration: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract the public API surface from a TypeScript package directory.
pub fn extract_ts_api(package_dir: &Path) -> Result<String> {
    match find_ts_entry(package_dir) {
        Some(entry) => extract_ts_file_api(&entry),
        None => extract_ts_dir_api(package_dir),
    }
}

/// Extract the public API surface from a TypeScript module file or directory.
pub fn extract_ts_module_api(module_path: &Path) -> Result<String> {
    if module_path.is_file() {
        extract_ts_file_api(module_path)
    } else if module_path.is_dir() {
        extract_ts_dir_api(module_path)
    } else {
        Ok(String::new())
    }
}

/// Extract the public API surface from a single TypeScript file.
pub fn extract_ts_file_api(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let items = parse_ts_exports(&content);
    Ok(format_ts_surface(&items))
}

// ---------------------------------------------------------------------------
// Entry-point discovery
// ---------------------------------------------------------------------------

fn find_ts_entry(dir: &Path) -> Option<PathBuf> {
    if let Some(entry) = entry_from_package_json(dir) {
        return Some(entry);
    }
    for rel in &[
        "index.ts",
        "index.tsx",
        "src/index.ts",
        "src/index.tsx",
        "lib/index.ts",
    ] {
        let p = dir.join(rel);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn entry_from_package_json(dir: &Path) -> Option<PathBuf> {
    let pkg_path = dir.join("package.json");
    let content = fs::read_to_string(&pkg_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    // Prefer "types" / "typings" over "main" (which might point to .js)
    for field in &["types", "typings"] {
        if let Some(entry) = json.get(field).and_then(|v| v.as_str()) {
            let p = dir.join(entry);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn extract_ts_dir_api(dir: &Path) -> Result<String> {
    let search_root = {
        let src = dir.join("src");
        if src.is_dir() { src } else { dir.to_path_buf() }
    };

    let mut files: Vec<PathBuf> = WalkDir::new(&search_root)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let ext = e.path().extension().and_then(|s| s.to_str());
            matches!(ext, Some("ts") | Some("tsx"))
        })
        .filter(|e| {
            let name = e.file_name().to_string_lossy();
            !name.ends_with(".d.ts")
                && !name.contains(".test.")
                && !name.contains(".spec.")
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    files.sort();

    let mut parts = Vec::new();
    for file in &files {
        match extract_ts_file_api(file) {
            Ok(s) if !s.is_empty() => parts.push(s),
            _ => {}
        }
    }
    Ok(parts.join("\n\n"))
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

fn parse_ts_exports(content: &str) -> Vec<TsItem> {
    let lines: Vec<&str> = content.lines().collect();
    let mut items = Vec::new();
    let mut i = 0;
    let mut pending_jsdoc = String::new();
    let mut in_jsdoc = false;
    let mut jsdoc_acc: Vec<String> = Vec::new();

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // --- JSDoc tracking ---
        if !in_jsdoc && trimmed.starts_with("/**") {
            in_jsdoc = true;
            jsdoc_acc.clear();
            jsdoc_acc.push(trimmed.to_string());
            if trimmed.contains("*/") {
                in_jsdoc = false;
                pending_jsdoc = jsdoc_acc.join(" ");
            }
            i += 1;
            continue;
        }
        if in_jsdoc {
            jsdoc_acc.push(trimmed.to_string());
            if trimmed.contains("*/") {
                in_jsdoc = false;
                pending_jsdoc = jsdoc_acc.join(" ");
            }
            i += 1;
            continue;
        }

        // Skip blank lines (preserve pending jsdoc across blank lines)
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Skip single-line and block comments (non-JSDoc)
        if trimmed.starts_with("//")
            || (trimmed.starts_with("/*") && !trimmed.starts_with("/**"))
        {
            pending_jsdoc.clear();
            i += 1;
            continue;
        }

        // Non-export line: clear accumulated jsdoc
        if !trimmed.starts_with("export") {
            pending_jsdoc.clear();
            i += 1;
            continue;
        }

        let jsdoc = std::mem::take(&mut pending_jsdoc);

        // Parse after "export"
        let rest = trimmed["export".len()..].trim_start();

        // Skip `export type { ... }` (type-only re-exports)
        if rest.starts_with("type {") || rest.starts_with("type{") {
            i += 1;
            continue;
        }

        let keyword = leading_keyword(rest);

        match keyword {
            "interface" => {
                let (decl, consumed) = collect_full_block(&lines, i, 150);
                items.push(TsItem { kind: "interface", jsdoc, declaration: decl });
                i += consumed;
            }
            "enum" | "const enum" => {
                let (decl, consumed) = collect_full_block(&lines, i, 80);
                items.push(TsItem { kind: "enum", jsdoc, declaration: decl });
                i += consumed;
            }
            "class" | "abstract class" => {
                let (decl, consumed) = collect_class_api(&lines, i);
                items.push(TsItem { kind: "class", jsdoc, declaration: decl });
                i += consumed;
            }
            "function" | "async function" => {
                let (sig, consumed) = collect_function_sig(&lines, i);
                items.push(TsItem { kind: "function", jsdoc, declaration: sig });
                i += consumed;
            }
            "type" => {
                let (decl, consumed) = collect_type_decl(&lines, i);
                items.push(TsItem { kind: "type", jsdoc, declaration: decl });
                i += consumed;
            }
            "const" | "let" | "var" => {
                let (decl, consumed) = collect_const_decl(&lines, i);
                items.push(TsItem { kind: "const", jsdoc, declaration: decl });
                i += consumed;
            }
            "default" => {
                let after_default = rest["default".len()..].trim_start();
                let kw2 = leading_keyword(after_default);
                match kw2 {
                    "function" | "async function" => {
                        let (sig, consumed) = collect_function_sig(&lines, i);
                        items.push(TsItem { kind: "function", jsdoc, declaration: sig });
                        i += consumed;
                    }
                    "class" | "abstract class" => {
                        let (decl, consumed) = collect_class_api(&lines, i);
                        items.push(TsItem { kind: "class", jsdoc, declaration: decl });
                        i += consumed;
                    }
                    _ => {
                        items.push(TsItem {
                            kind: "default",
                            jsdoc,
                            declaration: trimmed.to_string(),
                        });
                        i += 1;
                    }
                }
            }
            _ => {
                // Re-exports: `export { ... }`, `export * from '...'`
                let mut line_acc = trimmed.to_string();
                let mut j = i + 1;
                while !line_acc.contains(';') && j < lines.len() && j - i < 8 {
                    line_acc.push(' ');
                    line_acc.push_str(lines[j].trim());
                    j += 1;
                }
                items.push(TsItem { kind: "reexport", jsdoc, declaration: line_acc });
                i = j;
            }
        }
    }

    items
}

/// Determine the leading TypeScript keyword from a string that follows `export`.
/// Strips optional modifiers (`declare`, `async`, `abstract`) and returns the
/// canonical keyword, or `"_other"` if nothing matches.
fn leading_keyword(s: &str) -> &'static str {
    let mut rest = s.trim();
    loop {
        if let Some(r) = rest.strip_prefix("declare ") {
            rest = r.trim_start();
        } else if let Some(r) = rest.strip_prefix("async ") {
            rest = r.trim_start();
            if rest.starts_with("function") {
                return "async function";
            }
            return "_other";
        } else if let Some(r) = rest.strip_prefix("abstract ") {
            rest = r.trim_start();
            if rest.starts_with("class") {
                return "abstract class";
            }
            return "_other";
        } else {
            break;
        }
    }
    if rest.starts_with("default") {
        "default"
    } else if rest.starts_with("interface") {
        "interface"
    } else if rest.starts_with("const enum") || rest.starts_with("enum") {
        "enum"
    } else if rest.starts_with("class") {
        "class"
    } else if rest.starts_with("function") {
        "function"
    } else if rest.starts_with("type ") || rest.starts_with("type\n") {
        "type"
    } else if rest.starts_with("const ") {
        "const"
    } else if rest.starts_with("let ") {
        "let"
    } else if rest.starts_with("var ") {
        "var"
    } else {
        "_other"
    }
}

// ---------------------------------------------------------------------------
// Block collectors
// ---------------------------------------------------------------------------

/// Collect a complete braced block (interface, enum body, etc.) verbatim.
fn collect_full_block(lines: &[&str], start: usize, max_lines: usize) -> (String, usize) {
    let mut collected = Vec::new();
    let mut depth = 0i32;
    let mut found_open = false;
    let mut i = start;

    while i < lines.len() && i - start < max_lines {
        let line = lines[i];
        collected.push(line);
        for ch in line.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    found_open = true;
                }
                '}' => {
                    depth -= 1;
                }
                _ => {}
            }
        }
        i += 1;
        if found_open && depth == 0 {
            break;
        }
    }

    (collected.join("\n"), i - start)
}

/// Collect a class declaration as a compact API summary:
/// header + public member signatures (bodies stripped).
fn collect_class_api(lines: &[&str], start: usize) -> (String, usize) {
    let mut i = start;
    let mut outer_depth = 0i32;
    let mut header_done = false;
    let mut header_parts: Vec<String> = Vec::new();
    let mut public_members: Vec<String> = Vec::new();
    let mut member_jsdoc = String::new();
    let mut in_member_jsdoc = false;

    while i < lines.len() && i - start < 500 {
        let trimmed = lines[i].trim();

        if !header_done {
            // Accumulate header lines until we encounter the opening `{`
            if let Some(brace_pos) = find_unquoted_char(trimmed, '{') {
                let header_part = trimmed[..brace_pos].trim_end().to_string();
                if !header_part.is_empty() {
                    header_parts.push(header_part);
                }
                outer_depth = 1;
                header_done = true;
            } else if !trimmed.is_empty() {
                header_parts.push(trimmed.to_string());
            }
            i += 1;
            continue;
        }

        // --- Inside class body ---

        // Track JSDoc for members
        if !in_member_jsdoc && trimmed.starts_with("/**") {
            in_member_jsdoc = true;
            member_jsdoc = trimmed.to_string();
            if trimmed.contains("*/") {
                in_member_jsdoc = false;
            }
            i += 1;
            continue;
        }
        if in_member_jsdoc {
            member_jsdoc.push(' ');
            member_jsdoc.push_str(trimmed);
            if trimmed.contains("*/") {
                in_member_jsdoc = false;
            }
            i += 1;
            continue;
        }

        // Count how brace depth changes on this line
        let brace_delta: i32 = trimmed
            .chars()
            .map(|c| match c {
                '{' => 1,
                '}' => -1,
                _ => 0,
            })
            .sum();

        if outer_depth == 1 && is_public_class_member(trimmed) {
            let sig = extract_member_sig(trimmed);
            let jdoc_prefix = if !member_jsdoc.is_empty() {
                format!("  /** {} */\n", member_jsdoc.trim())
            } else {
                String::new()
            };
            public_members.push(format!("{}  {};", jdoc_prefix, sig));
            member_jsdoc.clear();

            // If this line opens a method body, skip past the body
            if brace_delta > 0 {
                outer_depth += brace_delta;
                i += 1;
                while i < lines.len() && outer_depth > 1 {
                    let inner = lines[i].trim();
                    for ch in inner.chars() {
                        match ch {
                            '{' => outer_depth += 1,
                            '}' => outer_depth -= 1,
                            _ => {}
                        }
                    }
                    i += 1;
                }
                continue;
            }
        } else {
            member_jsdoc.clear();
            outer_depth += brace_delta;
        }

        if outer_depth == 0 {
            i += 1;
            break;
        }

        i += 1;
    }

    let header = header_parts.join(" ").trim().to_string();
    let decl = if public_members.is_empty() {
        format!("{} {{}}", header)
    } else {
        format!("{} {{\n{}\n}}", header, public_members.join("\n"))
    };

    (decl, i - start)
}

/// Collect a function signature, stopping before the opening `{`.
fn collect_function_sig(lines: &[&str], start: usize) -> (String, usize) {
    let mut parts: Vec<&str> = Vec::new();
    let mut i = start;
    let mut paren_depth = 0i32;

    while i < lines.len() && i - start < 20 {
        let trimmed = lines[i].trim();
        parts.push(trimmed);

        for ch in trimmed.chars() {
            match ch {
                '(' => paren_depth += 1,
                ')' => paren_depth -= 1,
                _ => {}
            }
        }

        // When parens are balanced, the next `{` or `;` terminates the signature
        if paren_depth <= 0 {
            if trimmed.contains('{') || trimmed.ends_with(';') {
                i += 1;
                break;
            }
        }
        i += 1;
    }

    let full = parts.join(" ");
    // Strip body (everything from `{` onwards)
    let sig = if let Some(pos) = find_unquoted_char(&full, '{') {
        full[..pos].trim().to_string()
    } else {
        full
    };
    // Ensure trailing semicolon
    let sig = if sig.ends_with(';') {
        sig
    } else {
        format!("{};", sig)
    };

    (sig, i - start)
}

/// Collect a type alias declaration (until `;`).
fn collect_type_decl(lines: &[&str], start: usize) -> (String, usize) {
    let mut parts: Vec<&str> = Vec::new();
    let mut i = start;
    let mut depth = 0i32;

    while i < lines.len() && i - start < 30 {
        let trimmed = lines[i].trim();
        parts.push(trimmed);
        for ch in trimmed.chars() {
            match ch {
                '{' | '(' | '<' => depth += 1,
                '}' | ')' | '>' => depth -= 1,
                _ => {}
            }
        }
        i += 1;
        if depth <= 0 && parts.last().map(|l| l.ends_with(';')).unwrap_or(false) {
            break;
        }
    }

    (parts.join(" "), i - start)
}

/// Collect a `const`/`let`/`var` declaration, stripping the initialiser so
/// only the name and type annotation remain.
fn collect_const_decl(lines: &[&str], start: usize) -> (String, usize) {
    let line = lines[start].trim();

    // Strip the initialiser: keep everything before the first unquoted `=`
    // that is not `==` or `=>`.
    let decl = if let Some(eq_pos) = find_assignment_eq(line) {
        let lhs = line[..eq_pos].trim_end().to_string();
        if lhs.ends_with(';') {
            lhs
        } else {
            format!("{};", lhs)
        }
    } else if line.ends_with(';') {
        line.to_string()
    } else {
        // Multi-line or no initialiser
        let without_body = if let Some(pos) = find_unquoted_char(line, '{') {
            line[..pos].trim().to_string()
        } else {
            line.to_string()
        };
        if without_body.ends_with(';') {
            without_body
        } else {
            format!("{};", without_body)
        }
    };

    (decl, 1)
}

/// Find the index of a simple `=` sign that is not part of `==`, `===`, `=>`,
/// `<=`, `>=`, `!=`.
fn find_assignment_eq(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut in_str = false;
    let mut str_byte = b' ';
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if b == str_byte {
                in_str = false;
            }
        } else {
            match b {
                b'"' | b'\'' | b'`' => {
                    in_str = true;
                    str_byte = b;
                }
                b'=' => {
                    // Skip ==, ===, =>, <=, >= before the `=`, !=
                    let prev = if i > 0 { bytes[i - 1] } else { b' ' };
                    let next = if i + 1 < bytes.len() { bytes[i + 1] } else { b' ' };
                    if prev == b'!' || prev == b'<' || prev == b'>' || prev == b'=' {
                        // !=, <=, >=, == – skip
                    } else if next == b'=' || next == b'>' {
                        // ==, => – skip
                        i += 1;
                    } else {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Member helpers
// ---------------------------------------------------------------------------

fn is_public_class_member(line: &str) -> bool {
    if line.is_empty()
        || line == "}"
        || line.starts_with("//")
        || line.starts_with('*')
        || line.starts_with("/*")
    {
        return false;
    }
    // Explicitly non-public
    if line.starts_with("private ")
        || line.starts_with("protected ")
        || line.starts_with('#')
    {
        return false;
    }
    // Explicitly public or well-known member keywords
    if line.starts_with("constructor(")
        || line.starts_with("public ")
        || line.starts_with("static ")
        || line.starts_with("async ")
        || line.starts_with("abstract ")
        || line.starts_with("override ")
        || line.starts_with("readonly ")
        || line.starts_with("get ")
        || line.starts_with("set ")
    {
        return true;
    }
    // Method shorthand: `name(` or `name<` or `name:` at the start of the line
    // (not starting with a bracket – those are array/object literals)
    if let Some(first) = line.chars().next() {
        if first.is_alphabetic() || first == '_' || first == '$' {
            let rest = &line[first.len_utf8()..];
            let after_ident = rest
                .trim_start_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '$');
            if after_ident.starts_with('(')
                || after_ident.starts_with('<')
                || after_ident.starts_with(':')
                || after_ident.starts_with('?')
            {
                return true;
            }
        }
    }
    false
}

/// Extract the signature portion of a class member (everything up to `{`).
fn extract_member_sig(line: &str) -> String {
    if let Some(pos) = find_unquoted_char(line, '{') {
        line[..pos].trim_end().to_string()
    } else if line.ends_with(';') {
        line[..line.len() - 1].to_string()
    } else {
        line.to_string()
    }
}

/// Find the byte position of a character in a string, skipping quoted regions.
fn find_unquoted_char(s: &str, target: char) -> Option<usize> {
    let mut in_str = false;
    let mut str_char = '\0';
    for (pos, ch) in s.char_indices() {
        if in_str {
            if ch == str_char {
                in_str = false;
            }
        } else {
            match ch {
                '"' | '\'' | '`' => {
                    in_str = true;
                    str_char = ch;
                }
                c if c == target => return Some(pos),
                _ => {}
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Formatter
// ---------------------------------------------------------------------------

fn format_ts_surface(items: &[TsItem]) -> String {
    if items.is_empty() {
        return String::new();
    }

    let mut functions = Vec::new();
    let mut classes = Vec::new();
    let mut interfaces = Vec::new();
    let mut types = Vec::new();
    let mut enums = Vec::new();
    let mut consts = Vec::new();
    let mut reexports = Vec::new();
    let mut defaults = Vec::new();

    for item in items {
        let text = if item.jsdoc.is_empty() {
            item.declaration.clone()
        } else {
            format!("{}\n{}", item.jsdoc, item.declaration)
        };
        match item.kind {
            "function" => functions.push(text),
            "class" => classes.push(text),
            "interface" => interfaces.push(text),
            "type" => types.push(text),
            "enum" => enums.push(text),
            "const" | "let" | "var" => consts.push(text),
            "reexport" => reexports.push(text),
            "default" => defaults.push(text),
            _ => {}
        }
    }

    let mut parts = Vec::new();
    if !functions.is_empty() {
        parts.push(format!("// Functions\n{}", functions.join("\n\n")));
    }
    if !classes.is_empty() {
        parts.push(format!("// Classes\n{}", classes.join("\n\n")));
    }
    if !interfaces.is_empty() {
        parts.push(format!("// Interfaces\n{}", interfaces.join("\n\n")));
    }
    if !types.is_empty() {
        parts.push(format!("// Types\n{}", types.join("\n\n")));
    }
    if !enums.is_empty() {
        parts.push(format!("// Enums\n{}", enums.join("\n\n")));
    }
    if !consts.is_empty() {
        parts.push(format!("// Constants\n{}", consts.join("\n\n")));
    }
    if !reexports.is_empty() {
        parts.push(format!("// Re-exports\n{}", reexports.join("\n")));
    }
    if !defaults.is_empty() {
        parts.push(format!("// Default exports\n{}", defaults.join("\n")));
    }

    parts.join("\n\n")
}

// ---------------------------------------------------------------------------
// Stub generation
// ---------------------------------------------------------------------------

/// Transform api_surface text into compilable TypeScript source code.
pub fn make_ts_compilable(api_surface: &str) -> String {
    let mut result = String::new();

    for line in api_surface.lines() {
        let trimmed = line.trim();

        // Section headers → pass through as comments
        if trimmed.starts_with("// ") {
            result.push_str(line);
            result.push('\n');
            continue;
        }

        // Function signatures ending with `;` → add stub body
        if is_function_decl_line(trimmed) && trimmed.ends_with(';') {
            let without_semi = trimmed[..trimmed.len() - 1].trim_end();
            result.push_str(&format!(
                "{} {{ throw new Error('not implemented'); }}\n",
                without_semi
            ));
            continue;
        }

        // Class method signatures inside a class block (indented, ends with `;`)
        if trimmed.ends_with(';')
            && !trimmed.starts_with("export ")
            && !trimmed.starts_with("import ")
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("*")
            && trimmed.contains('(')
            && trimmed.contains(')')
            && is_method_sig_line(trimmed)
        {
            let indent = leading_spaces(line);
            let without_semi = trimmed[..trimmed.len() - 1].trim_end();
            result.push_str(&format!(
                "{}{} {{ throw new Error('not implemented'); }}\n",
                indent, without_semi
            ));
            continue;
        }

        // `export const foo: T;` without initialiser → add `= null as any`
        if (trimmed.starts_with("export const ")
            || trimmed.starts_with("export let ")
            || trimmed.starts_with("export var "))
            && trimmed.ends_with(';')
            && !trimmed.contains('=')
        {
            let without_semi = trimmed[..trimmed.len() - 1].trim_end();
            result.push_str(&format!("{} = null as any;\n", without_semi));
            continue;
        }

        result.push_str(line);
        result.push('\n');
    }

    result
}

fn is_function_decl_line(line: &str) -> bool {
    (line.starts_with("export function ")
        || line.starts_with("export async function ")
        || line.starts_with("export default function "))
        && !line.contains('{')
        && !line.contains('}')
}

fn is_method_sig_line(line: &str) -> bool {
    // Heuristic: looks like a method if it has parens and is not an interface property
    line.contains('(')
        && line.contains(')')
        && !line.starts_with('[')
        && !line.starts_with('{')
}

fn leading_spaces(line: &str) -> &str {
    let non_space = line.find(|c: char| !c.is_whitespace()).unwrap_or(0);
    &line[..non_space]
}
