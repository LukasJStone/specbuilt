use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use crate::extract_ts;

/// Generate a compilable stub module file from a .sb spec file.
/// Writes a single `.rs` file (not a full crate with Cargo.toml).
pub fn generate_module_stub(spec_path: &Path, output_path: &Path) -> Result<()> {
    let content = fs::read_to_string(spec_path)
        .with_context(|| format!("Failed to read spec: {}", spec_path.display()))?;
    let spec: crate::SpecFile = toml::from_str(&content)
        .with_context(|| format!("Failed to parse spec: {}", spec_path.display()))?;

    let stub_src = if spec.interface.api_surface.is_empty() {
        "// Empty stub — no public API surface extracted\n".to_string()
    } else {
        make_compilable(&spec.interface.api_surface)
    };

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(output_path, stub_src)
        .with_context(|| format!("Failed to write stub: {}", output_path.display()))?;

    println!(
        "Generated module stub for '{}' -> {}",
        spec.package.name,
        output_path.display()
    );
    Ok(())
}

/// Generate a compilable stub crate from a .sb spec file.
pub fn generate_stub_crate(spec_path: &Path, output_dir: &Path) -> Result<()> {
    let content = fs::read_to_string(spec_path)
        .with_context(|| format!("Failed to read spec: {}", spec_path.display()))?;
    let spec: crate::SpecFile = toml::from_str(&content)
        .with_context(|| format!("Failed to parse spec: {}", spec_path.display()))?;

    let stub_src = if spec.interface.api_surface.is_empty() {
        String::new()
    } else {
        make_compilable(&spec.interface.api_surface)
    };

    let crate_dir = output_dir.join(&spec.package.name);
    let src_dir = crate_dir.join("src");
    fs::create_dir_all(&src_dir)?;

    let lib_rs = if stub_src.is_empty() {
        "// Empty stub — no public API surface extracted\n".to_string()
    } else {
        stub_src
    };

    fs::write(src_dir.join("lib.rs"), lib_rs)
        .with_context(|| format!("Failed to write lib.rs in {}", src_dir.display()))?;

    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "{}"
edition = "2021"
"#,
        spec.package.name, spec.package.version
    );
    fs::write(crate_dir.join("Cargo.toml"), cargo_toml)
        .with_context(|| format!("Failed to write Cargo.toml in {}", crate_dir.display()))?;

    println!(
        "Generated stub for '{}' -> {}",
        spec.package.name,
        crate_dir.display()
    );
    Ok(())
}

/// Transform api_surface text into compilable Rust code.
fn make_compilable(api_surface: &str) -> String {
    let mut result = String::new();
    let lines: Vec<&str> = api_surface.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Section headers become module-level comments
        if line.starts_with("// ") {
            result.push_str(line);
            result.push('\n');
            i += 1;
            continue;
        }

        // Collect a multi-line item
        let mut item_lines = Vec::new();
        while i < lines.len() && !lines[i].trim().is_empty() && !lines[i].starts_with("// ") {
            item_lines.push(lines[i]);
            i += 1;
        }

        if item_lines.is_empty() {
            i += 1;
            continue;
        }

        let item_text = item_lines.join("\n");
        result.push_str(&transform_item(&item_text));
        result.push('\n');
    }

    result
}

fn transform_item(text: &str) -> String {
    let trimmed = text.trim();

    // Trait block — keep as-is (methods with ; are valid in traits)
    if trimmed.contains("trait ") && trimmed.ends_with('}') {
        return text.to_string();
    }

    // Impl block — replace method signatures ending with ; with todo bodies
    if trimmed.starts_with("impl ") && trimmed.ends_with('}') {
        return transform_impl_block(text);
    }

    // Standalone function signature ending with ;
    if trimmed.contains("fn ") && trimmed.ends_with(';') {
        let without_semi = trimmed[..trimmed.len() - 1].trim();
        return format!("{} {{
    todo!()
}}", without_semi);
    }

    // Struct, enum, type, const, static — keep as-is
    text.to_string()
}

fn transform_impl_block(text: &str) -> String {
    let mut result = String::new();
    let lines: Vec<&str> = text.lines().collect();

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.ends_with(';') && trimmed.contains("fn ") {
            let without_semi = trimmed[..trimmed.len() - 1].trim();
            // Check if it's a doc comment or attribute
            if without_semi.starts_with("///") || without_semi.starts_with("#") {
                result.push_str(line);
                result.push('\n');
                continue;
            }
            result.push_str(&format!(
                "{} {{
        todo!()
    }}",
                without_semi
            ));
            result.push('\n');
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    result
}

// ---------------------------------------------------------------------------
// TypeScript stubs
// ---------------------------------------------------------------------------

/// Generate stub TypeScript source from an api_surface string.
fn ts_stub_source(spec: &crate::SpecFile) -> String {
    if spec.interface.api_surface.is_empty() {
        "// Empty stub — no public API surface extracted\nexport {};\n".to_string()
    } else {
        extract_ts::make_ts_compilable(&spec.interface.api_surface)
    }
}

/// Generate a compilable TypeScript stub module file from a .sb spec.
/// Writes a single `.ts` file.
pub fn generate_ts_module_stub(spec_path: &Path, output_path: &Path) -> Result<()> {
    let content = fs::read_to_string(spec_path)
        .with_context(|| format!("Failed to read spec: {}", spec_path.display()))?;
    let spec: crate::SpecFile = toml::from_str(&content)
        .with_context(|| format!("Failed to parse spec: {}", spec_path.display()))?;

    let stub_src = ts_stub_source(&spec);

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(output_path, stub_src)
        .with_context(|| format!("Failed to write TS stub: {}", output_path.display()))?;

    println!(
        "Generated TypeScript module stub for '{}' -> {}",
        spec.package.name,
        output_path.display()
    );
    Ok(())
}

/// Generate a compilable TypeScript stub package from a .sb spec.
/// Writes `output_dir/<name>/index.ts` and `output_dir/<name>/package.json`.
pub fn generate_ts_stub_crate(spec_path: &Path, output_dir: &Path) -> Result<()> {
    let content = fs::read_to_string(spec_path)
        .with_context(|| format!("Failed to read spec: {}", spec_path.display()))?;
    let spec: crate::SpecFile = toml::from_str(&content)
        .with_context(|| format!("Failed to parse spec: {}", spec_path.display()))?;

    let stub_src = ts_stub_source(&spec);

    let pkg_dir = output_dir.join(&spec.package.name);
    fs::create_dir_all(&pkg_dir)?;

    fs::write(pkg_dir.join("index.ts"), stub_src)
        .with_context(|| format!("Failed to write index.ts in {}", pkg_dir.display()))?;

    let version = if spec.package.version.is_empty() {
        "0.0.0".to_string()
    } else {
        spec.package.version.clone()
    };

    let pkg_json = format!(
        r#"{{
  "name": "{}",
  "version": "{}",
  "description": "{}",
  "main": "index.ts",
  "types": "index.ts"
}}
"#,
        spec.package.name,
        version,
        spec.package.description
    );
    fs::write(pkg_dir.join("package.json"), pkg_json)
        .with_context(|| format!("Failed to write package.json in {}", pkg_dir.display()))?;

    println!(
        "Generated TypeScript stub for '{}' -> {}",
        spec.package.name,
        pkg_dir.display()
    );
    Ok(())
}
