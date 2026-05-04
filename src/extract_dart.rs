use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Structured representation of a single Dart public declaration.
struct DartItem {
    kind: &'static str, // "class", "mixin", "extension", "enum", "typedef", "function", "const", "var"
    declaration: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract the public API surface from a Flutter/Dart package directory.
pub fn extract_dart_api(package_dir: &Path) -> Result<String> {
    match find_dart_entry(package_dir) {
        Some(entry) => extract_dart_file_api(&entry),
        None => extract_dart_dir_api(package_dir),
    }
}

/// Extract the public API surface from a Dart module file or directory.
pub fn extract_dart_module_api(module_path: &Path) -> Result<String> {
    if module_path.is_file() {
        extract_dart_file_api(module_path)
    } else if module_path.is_dir() {
        extract_dart_dir_api(module_path)
    } else {
        Ok(String::new())
    }
}

/// Extract the public API surface from a single Dart file.
pub fn extract_dart_file_api(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let items = parse_dart_exports(&content);
    Ok(format_dart_surface(&items))
}

// ---------------------------------------------------------------------------
// Entry-point discovery
// ---------------------------------------------------------------------------

fn find_dart_entry(dir: &Path) -> Option<PathBuf> {
    // Prefer lib/<name>.dart, then lib/main.dart, then lib/<anything>.dart
    let lib_dir = dir.join("lib");
    if lib_dir.is_dir() {
        // Try lib/<package_name>.dart via pubspec.yaml
        if let Some(name) = package_name_from_pubspec(dir) {
            let entry = lib_dir.join(format!("{}.dart", name));
            if entry.exists() {
                return Some(entry);
            }
        }
        // Fallback to lib/main.dart
        let main_dart = lib_dir.join("main.dart");
        if main_dart.exists() {
            return Some(main_dart);
        }
    }
    None
}

fn package_name_from_pubspec(dir: &Path) -> Option<String> {
    let content = fs::read_to_string(dir.join("pubspec.yaml")).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("name:") {
            let name = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

fn extract_dart_dir_api(dir: &Path) -> Result<String> {
    let search_root = {
        let lib = dir.join("lib");
        if lib.is_dir() { lib } else { dir.to_path_buf() }
    };

    let mut files: Vec<PathBuf> = WalkDir::new(&search_root)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path().extension().and_then(|s| s.to_str()) == Some("dart")
        })
        .filter(|e| {
            let name = e.file_name().to_string_lossy();
            !name.contains(".test.") && !name.contains("_test.dart")
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    files.sort();

    let mut parts = Vec::new();
    for file in &files {
        match extract_dart_file_api(file) {
            Ok(s) if !s.is_empty() => parts.push(s),
            _ => {}
        }
    }
    Ok(parts.join("\n\n"))
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

fn is_private(name: &str) -> bool {
    name.trim_start().starts_with('_')
}

/// Collect a multi-line declaration that ends with `{` or `;`.
/// Returns (declaration_text, lines_consumed).
fn collect_declaration(lines: &[&str], start: usize) -> (String, usize) {
    let mut collected = Vec::new();
    let mut i = start;
    loop {
        if i >= lines.len() {
            break;
        }
        let line = lines[i];
        collected.push(line.trim_end().to_string());
        i += 1;
        let joined = collected.join(" ");
        let trimmed = joined.trim();
        // Stop at opening brace (block start) or semicolon (typedef / simple decl)
        if trimmed.ends_with('{') || trimmed.ends_with(';') || trimmed.ends_with("=>") {
            break;
        }
        // Safety: don't consume too many lines
        if collected.len() > 15 {
            break;
        }
    }
    let text = collected.join("\n");
    (text, i - start)
}

fn parse_dart_exports(content: &str) -> Vec<DartItem> {
    let lines: Vec<&str> = content.lines().collect();
    let mut items = Vec::new();
    let mut i = 0;
    let mut brace_depth: i32 = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Track brace depth for lines we're skipping over
        if brace_depth > 0 {
            for ch in line.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        brace_depth -= 1;
                        if brace_depth < 0 {
                            brace_depth = 0;
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
            continue;
        }

        // Skip blank lines, comments
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with("*") {
            i += 1;
            continue;
        }

        // Skip imports, exports, library, part
        if trimmed.starts_with("import ")
            || trimmed.starts_with("export ")
            || trimmed.starts_with("library ")
            || trimmed.starts_with("part ")
        {
            i += 1;
            continue;
        }

        // Annotations (like @override, @deprecated) — skip and continue
        if trimmed.starts_with('@') {
            i += 1;
            continue;
        }

        // --- Detect top-level declarations ---

        // typedef
        if trimmed.starts_with("typedef ") {
            let (decl, consumed) = collect_declaration(&lines, i);
            let name = extract_identifier_after("typedef", &decl);
            if !is_private(&name) {
                items.push(DartItem { kind: "typedef", declaration: decl });
            }
            i += consumed;
            continue;
        }

        // enum
        if let Some(name) = try_extract_class_keyword(trimmed, "enum") {
            if !is_private(&name) {
                let (decl, consumed, _) = collect_block(&lines, i);
                items.push(DartItem { kind: "enum", declaration: decl });
                i += consumed;
            } else {
                i += 1;
            }
            continue;
        }

        // mixin
        if let Some(name) = try_extract_class_keyword(trimmed, "mixin") {
            if !is_private(&name) {
                let (decl, consumed, _) = collect_block(&lines, i);
                items.push(DartItem { kind: "mixin", declaration: extract_header(&decl) });
                // advance past the block body (consumed includes body)
                i += consumed;
            } else {
                i += 1;
            }
            continue;
        }

        // extension
        if trimmed.starts_with("extension ") {
            let name_after = trimmed["extension ".len()..].trim();
            let name = name_after.split_whitespace().next().unwrap_or("");
            // anonymous extensions (extension on ...) are still public
            if !is_private(name) {
                let (decl, consumed, _) = collect_block(&lines, i);
                items.push(DartItem { kind: "extension", declaration: extract_header(&decl) });
                i += consumed;
            } else {
                i += 1;
            }
            continue;
        }

        // class variants: class, abstract class, final class, sealed class, base class,
        // interface class, mixin class, abstract final class, abstract base class, etc.
        if is_class_declaration(trimmed) {
            let name = extract_class_name(trimmed);
            if !is_private(&name) {
                let (decl, consumed, _) = collect_block(&lines, i);
                items.push(DartItem { kind: "class", declaration: extract_header(&decl) });
                i += consumed;
            } else {
                i += 1;
            }
            continue;
        }

        // top-level const / final variable
        if trimmed.starts_with("const ") || trimmed.starts_with("final ") {
            let name = extract_var_name(trimmed);
            if !is_private(&name) {
                let (decl, consumed) = collect_declaration(&lines, i);
                items.push(DartItem { kind: "const", declaration: decl });
                i += consumed;
            } else {
                i += 1;
            }
            continue;
        }

        // top-level function: heuristic — non-indented line containing `(` that
        // looks like a function signature.
        if !line.starts_with(' ') && !line.starts_with('\t') && trimmed.contains('(') {
            let name = extract_function_name(trimmed);
            if !name.is_empty() && !is_private(&name) && is_likely_function(trimmed) {
                let (decl, consumed) = collect_declaration(&lines, i);
                // Only include declarations ending with `;` or `{` or `=>`
                let joined = decl.trim().to_string();
                if joined.ends_with('{') || joined.ends_with(';') || joined.ends_with("=>") {
                    items.push(DartItem {
                        kind: "function",
                        declaration: format_function_sig(&decl),
                    });
                }
                i += consumed;
            } else {
                // Might be a block opener at top level (e.g. main body) — track depth
                let opens: i32 = line.chars().filter(|&c| c == '{').count() as i32;
                let closes: i32 = line.chars().filter(|&c| c == '}').count() as i32;
                brace_depth += opens - closes;
                if brace_depth < 0 { brace_depth = 0; }
                i += 1;
            }
            continue;
        }

        // Unrecognised line — track brace depth and move on
        let opens: i32 = line.chars().filter(|&c| c == '{').count() as i32;
        let closes: i32 = line.chars().filter(|&c| c == '}').count() as i32;
        brace_depth += opens - closes;
        if brace_depth < 0 { brace_depth = 0; }
        i += 1;
    }

    items
}

// ---------------------------------------------------------------------------
// Declaration-shape helpers
// ---------------------------------------------------------------------------

fn is_class_declaration(trimmed: &str) -> bool {
    // Check for 'class' keyword (possibly preceded by modifiers)
    let modifiers = ["abstract", "final", "sealed", "base", "interface", "mixin"];
    let mut rest = trimmed;

    // Strip up to two leading modifiers
    for _ in 0..3 {
        let mut stripped = false;
        for m in &modifiers {
            if let Some(r) = rest.strip_prefix(m) {
                let r = r.trim_start();
                rest = r;
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }

    rest.starts_with("class ") || rest == "class"
}

fn extract_class_name(trimmed: &str) -> String {
    // Strip modifiers and 'class' keyword, then take first word
    let mut rest = trimmed;
    let modifiers = ["abstract", "final", "sealed", "base", "interface", "mixin"];
    for _ in 0..3 {
        let mut stripped = false;
        for m in &modifiers {
            if let Some(r) = rest.strip_prefix(m) {
                rest = r.trim_start();
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    if let Some(r) = rest.strip_prefix("class") {
        r.trim_start()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches('{')
            .to_string()
    } else {
        String::new()
    }
}

fn try_extract_class_keyword(trimmed: &str, keyword: &str) -> Option<String> {
    let prefix = format!("{} ", keyword);
    if trimmed.starts_with(&prefix) || trimmed == keyword {
        let rest = if trimmed.starts_with(&prefix) {
            &trimmed[prefix.len()..]
        } else {
            ""
        };
        let name = rest
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches('{')
            .to_string();
        Some(name)
    } else {
        None
    }
}

fn extract_identifier_after(keyword: &str, text: &str) -> String {
    let prefix = format!("{} ", keyword);
    if let Some(rest) = text.trim().strip_prefix(&prefix) {
        rest.split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(';')
            .to_string()
    } else {
        String::new()
    }
}

fn extract_var_name(trimmed: &str) -> String {
    // "const int foo = ..." or "final foo = ..."
    trimmed
        .split_whitespace()
        .find(|w| {
            !matches!(
                *w,
                "const" | "final" | "static" | "late"
                    | "int" | "double" | "String" | "bool" | "dynamic" | "var" | "void"
                    | "Object" | "num"
            ) && !w.ends_with('?')
                && !w.starts_with('<')
        })
        .unwrap_or("")
        .to_string()
}

fn extract_function_name(trimmed: &str) -> String {
    // Take the word immediately before the first `(`
    if let Some(paren_pos) = trimmed.find('(') {
        let before = &trimmed[..paren_pos];
        before.split_whitespace().last().unwrap_or("").to_string()
    } else {
        String::new()
    }
}

fn is_likely_function(trimmed: &str) -> bool {
    // Must contain `(` and not look like a function call (which would have a trailing `;`
    // on the same line after `)` with no `{`).
    // Simple heuristic: has `(` and is not starting with a literal / keyword that
    // indicates it's not a declaration.
    if trimmed.starts_with("if ") || trimmed.starts_with("for ") || trimmed.starts_with("while ")
        || trimmed.starts_with("switch ") || trimmed.starts_with("return ")
        || trimmed.starts_with("throw ") || trimmed.starts_with("print(")
    {
        return false;
    }
    // If the line looks like `identifier(...)` with a return type before it, it's a function
    true
}

fn format_function_sig(decl: &str) -> String {
    // Replace block body with `;`
    let text = decl.trim();
    if text.ends_with('{') {
        let without_brace = text[..text.len() - 1].trim_end();
        format!("{};", without_brace)
    } else if text.ends_with("=>") {
        format!("{} ...;", text)
    } else {
        text.to_string()
    }
}

/// Collect a full block (balanced braces) starting from line `start`.
/// Returns (full_text, lines_consumed, inner_brace_depth_when_done).
fn collect_block(lines: &[&str], start: usize) -> (String, usize, i32) {
    let mut collected = Vec::new();
    let mut depth: i32 = 0;
    let mut i = start;
    let mut block_started = false;

    loop {
        if i >= lines.len() {
            break;
        }
        let line = lines[i];
        collected.push(line.to_string());
        i += 1;

        for ch in line.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    block_started = true;
                }
                '}' => {
                    depth -= 1;
                }
                _ => {}
            }
        }

        if block_started && depth <= 0 {
            break;
        }

        // Handle declarations ending with `;` before a `{` is seen (like abstract methods)
        if !block_started && line.trim().ends_with(';') {
            break;
        }
    }

    let text = collected.join("\n");
    (text, i - start, depth)
}

/// Extract just the header line(s) of a block declaration (up to and including the `{`).
fn extract_header(full_block: &str) -> String {
    let lines: Vec<&str> = full_block.lines().collect();
    let mut header_lines = Vec::new();
    for line in &lines {
        header_lines.push(line.trim_end().to_string());
        if line.contains('{') {
            break;
        }
    }
    header_lines.join("\n")
}

// ---------------------------------------------------------------------------
// Formatter
// ---------------------------------------------------------------------------

fn format_dart_surface(items: &[DartItem]) -> String {
    let mut by_kind: std::collections::BTreeMap<&'static str, Vec<&str>> = Default::default();
    for item in items {
        by_kind.entry(item.kind).or_default().push(&item.declaration);
    }

    let order: &[&str] = &["class", "mixin", "extension", "enum", "typedef", "function", "const", "var"];
    let labels: std::collections::HashMap<&str, &str> = [
        ("class", "// Classes"),
        ("mixin", "// Mixins"),
        ("extension", "// Extensions"),
        ("enum", "// Enums"),
        ("typedef", "// Typedefs"),
        ("function", "// Functions"),
        ("const", "// Constants & Variables"),
        ("var", "// Constants & Variables"),
    ]
    .iter()
    .cloned()
    .collect();

    let mut parts = Vec::new();
    for kind in order {
        if let Some(decls) = by_kind.get(kind) {
            if !decls.is_empty() {
                let header = labels.get(kind).copied().unwrap_or("// Other");
                parts.push(format!("{}\n{}", header, decls.join("\n\n")));
            }
        }
    }

    parts.join("\n\n")
}

// ---------------------------------------------------------------------------
// Stub source generation (called from stub.rs)
// ---------------------------------------------------------------------------

/// Generate a compilable Dart stub from an api_surface string.
pub fn make_dart_stub(api_surface: &str) -> String {
    if api_surface.is_empty() {
        return "// Empty stub — no public API surface extracted\n".to_string();
    }

    let mut result = String::new();
    let lines: Vec<&str> = api_surface.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Section comments — pass through
        if line.starts_with("// ") {
            result.push_str(line);
            result.push('\n');
            i += 1;
            continue;
        }

        // Blank lines
        if line.trim().is_empty() {
            result.push('\n');
            i += 1;
            continue;
        }

        // Class / mixin / extension / enum header ending with `{`
        if line.trim_end().ends_with('{') {
            result.push_str(line);
            result.push('\n');
            result.push_str("}\n");
            i += 1;
            continue;
        }

        // Function signatures ending with `;`
        if line.trim_end().ends_with(';') && line.contains('(') && !line.trim_start().starts_with("//") {
            let sig = line.trim_end_matches(';').trim_end();
            // Make it a stub body
            result.push_str(&format!("{} {{\n  throw UnimplementedError();\n}}\n", sig));
            i += 1;
            continue;
        }

        // typedef, const, enum values — pass through as-is
        result.push_str(line);
        result.push('\n');
        i += 1;
    }

    result
}
