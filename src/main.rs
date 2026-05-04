use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

mod agent;
mod extract;
mod index;
mod stub;

/// Specbuilt - Context window management for AI agents working with Rust
#[derive(Parser)]
#[command(name = "specbuild")]
#[command(about = "Open and close Rust packages via .sb spec files")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Open a package: replace .sb file with full source
    Open {
        /// Package name (the .sb file without extension)
        package: String,
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Close a package: replace source with .sb file
    Close {
        /// Package name (the directory to close)
        package: String,
        /// Regenerate the spec from source before closing
        #[arg(long)]
        regenerate: bool,
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// List all packages and their status
    List {
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Build the project (ensures all packages are available for compilation)
    Build {
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Open all closed packages
    OpenAll {
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Close all opened packages
    CloseAll {
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Base the current directory: auto-generate .sb files from sibling -sb/specbuilt-source
    Base {
        /// Path to application-rs directory (default: current directory)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Check specs against implementation (like unit tests for specs)
    Check {
        /// Package name to check (checks all if omitted)
        package: Option<String>,
        /// Force re-check even if nothing changed
        #[arg(short, long)]
        force: bool,
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Fix a package: update spec, open, run agent, check, close
    Fix {
        /// Package name to fix
        package: String,
        /// Prompt describing what to fix or add
        #[arg(short, long)]
        prompt: Option<String>,
        /// Max check retries after agent runs
        #[arg(short, long, default_value = "3")]
        retries: u32,
        /// Leave package open after fixing
        #[arg(long)]
        no_close: bool,
        /// AI agent command (falls back to SPECBUILD_AI_AGENT env var)
        #[arg(long)]
        agent: Option<String>,
        /// LLM provider (openai, anthropic; falls back to SPECBUILD_AI_PROVIDER)
        #[arg(long)]
        provider: Option<String>,
        /// LLM model name (falls back to SPECBUILD_AI_MODEL)
        #[arg(long)]
        model: Option<String>,
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Doover a package: clear source and reimplement from spec
    Doover {
        /// Package name to rebuild
        package: String,
        /// Additional prompt for the rebuild
        #[arg(short, long)]
        prompt: Option<String>,
        /// Max check retries after agent runs
        #[arg(short, long, default_value = "3")]
        retries: u32,
        /// Leave package open after rebuild
        #[arg(long)]
        no_close: bool,
        /// AI agent command (falls back to SPECBUILD_AI_AGENT env var)
        #[arg(long)]
        agent: Option<String>,
        /// LLM provider (openai, anthropic; falls back to SPECBUILD_AI_PROVIDER)
        #[arg(long)]
        provider: Option<String>,
        /// LLM model name (falls back to SPECBUILD_AI_MODEL)
        #[arg(long)]
        model: Option<String>,
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Build the symbol index from specbuilt-source
    Index {
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Search the symbol index
    Search {
        /// Query string (symbol name, kind, or doc text)
        query: String,
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Generate a compilable stub crate from a .sb spec
    Stub {
        /// Package name (the .sb file without extension)
        package: String,
        /// Output directory for the stub (default: .specbuild-stubs)
        #[arg(short, long)]
        out_dir: Option<PathBuf>,
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Generate stub crates for all closed packages
    StubAll {
        /// Output directory for stubs (default: .specbuild-stubs)
        #[arg(short, long)]
        out_dir: Option<PathBuf>,
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Open a module within an opened crate: replace .sb with full source
    OpenModule {
        /// Module path (e.g. auth::models)
        path: String,
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Close a module within an opened crate: replace source with .sb + stub
    CloseModule {
        /// Module path (e.g. auth::models)
        path: String,
        /// Regenerate the spec from source before closing
        #[arg(long)]
        regenerate: bool,
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
    /// Write the Specbuilt agent guide (AGENTS.md) into the application-rs directory
    AgentGuide {
        /// Path to application-rs directory (default: ./application-rs)
        #[arg(short, long)]
        app_dir: Option<PathBuf>,
    },
}

/// The .sb spec file format
#[derive(Debug, Serialize, Deserialize)]
struct SpecFile {
    package: PackageSpec,
    #[serde(default)]
    interface: InterfaceSpec,
    source: SourceSpec,
    #[serde(default)]
    test: TestSpec,
    #[serde(default)]
    spec: SpecSection,
}

#[derive(Debug, Serialize, Deserialize)]
struct PackageSpec {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct InterfaceSpec {
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    outputs: Vec<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    /// Auto-extracted public API surface of the package
    #[serde(default)]
    api_surface: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SourceSpec {
    /// Path relative to application-rs where source lives
    path: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TestSpec {
    #[serde(default)]
    command: String,
}

/// Behavioral specification section
#[derive(Debug, Default, Serialize, Deserialize)]
struct SpecSection {
    #[serde(default)]
    description: String,
    #[serde(default)]
    invariants: Vec<String>,
    #[serde(default)]
    verify_command: Option<String>,
}

/// Minimal Cargo.toml parsing for base command
#[derive(Debug, Deserialize)]
struct CargoToml {
    package: CargoPackage,
    #[serde(default)]
    dependencies: toml::Table,
    #[serde(default)]
    dev_dependencies: toml::Table,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
}

// ---------------------------------------------------------------------------
// Check database types
// ---------------------------------------------------------------------------

const CHECKS_DB: &str = "specbuilt-checks.json";

#[derive(Debug, Serialize, Deserialize, Default)]
struct CheckDatabase {
    #[serde(default)]
    records: HashMap<String, CheckRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum CheckResult {
    Passed,
    Failed,
    Bugged,
}

impl std::fmt::Display for CheckResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckResult::Passed => write!(f, "PASSED"),
            CheckResult::Failed => write!(f, "FAILED"),
            CheckResult::Bugged => write!(f, "BUGGED"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckRecord {
    spec_hash: String,
    source_hash: String,
    spec_modified: u64,
    source_modified: u64,
    last_checked: u64,
    result: CheckResult,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct AiCheckResult {
    result: CheckResult,
    reason: String,
}

// ---------------------------------------------------------------------------
// Constants & helpers
// ---------------------------------------------------------------------------

const SB_EXT: &str = ".sb";
const SPEC_HIDDEN: &str = ".specbuilt.sb";

fn get_app_dir(app_dir: Option<PathBuf>) -> Result<PathBuf> {
    let dir = match app_dir {
        Some(d) => d,
        None => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            if cwd
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with("-rs"))
                .unwrap_or(false)
            {
                PathBuf::from(".")
            } else {
                PathBuf::from("application-rs")
            }
        }
    };
    let canonical = dir
        .canonicalize()
        .with_context(|| format!("Cannot find application directory: {}", dir.display()))?;
    Ok(canonical)
}

fn get_app_dir_or_cwd(app_dir: Option<PathBuf>) -> Result<PathBuf> {
    let dir = app_dir.unwrap_or_else(|| PathBuf::from("."));
    let canonical = dir
        .canonicalize()
        .with_context(|| format!("Cannot find application directory: {}", dir.display()))?;
    Ok(canonical)
}

fn read_spec(path: &Path) -> Result<SpecFile> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read spec file: {}", path.display()))?;
    let spec: SpecFile = toml::from_str(&content)
        .with_context(|| format!("Failed to parse spec file: {}", path.display()))?;
    Ok(spec)
}

fn write_spec(path: &Path, spec: &SpecFile) -> Result<()> {
    let content =
        toml::to_string_pretty(spec).with_context(|| "Failed to serialize spec file")?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write spec file: {}", path.display()))?;
    Ok(())
}

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn remove_dir_all_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .with_context(|| format!("Failed to remove directory: {}", path.display()))?;
    }
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("Failed to remove file: {}", path.display()))?;
    }
    Ok(())
}

fn relative_path(from: &Path, to: &Path) -> Result<String> {
    let from = from
        .canonicalize()
        .with_context(|| format!("Cannot canonicalize: {}", from.display()))?;
    let to = to
        .canonicalize()
        .with_context(|| format!("Cannot canonicalize: {}", to.display()))?;

    let from_components: Vec<_> = from.components().collect();
    let to_components: Vec<_> = to.components().collect();

    let mut common = 0;
    while common < from_components.len()
        && common < to_components.len()
        && from_components[common] == to_components[common]
    {
        common += 1;
    }

    if common == 0 {
        return Ok(to.display().to_string());
    }

    let mut result = Vec::new();
    for _ in common..from_components.len() {
        result.push("..".to_string());
    }

    for comp in &to_components[common..] {
        result.push(comp.as_os_str().to_string_lossy().to_string());
    }

    if result.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(result.join("/"))
    }
}

// ---------------------------------------------------------------------------
// Hashing & timestamps
// ---------------------------------------------------------------------------

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    to_hex(&hasher.finalize())
}

fn hash_source_dir(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut entries: Vec<_> = walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"))
        .map(|e| e.path().to_path_buf())
        .collect();
    entries.sort();

    for entry in entries {
        let content = fs::read(&entry)?;
        hasher.update(&content);
        hasher.update(b"\0");
    }

    Ok(to_hex(&hasher.finalize()))
}

fn file_mtime(path: &Path) -> Result<u64> {
    let meta = fs::metadata(path)?;
    let mtime = meta.modified()?;
    Ok(mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs())
}

fn max_mtime_source(path: &Path) -> Result<u64> {
    let mut max = 0u64;
    for entry in walkdir::WalkDir::new(path) {
        let entry = entry?;
        if entry.file_type().is_file()
            && entry.path().extension().and_then(|s| s.to_str()) == Some("rs")
        {
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    let secs = mtime
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if secs > max {
                        max = secs;
                    }
                }
            }
        }
    }
    Ok(max)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// Check database
// ---------------------------------------------------------------------------

fn get_checks_db_path(app_dir: &Path) -> Result<PathBuf> {
    let sb_dir = discover_sb_dir(app_dir)?;
    Ok(sb_dir.join(CHECKS_DB))
}

fn load_checks_db(path: &Path) -> Result<CheckDatabase> {
    if !path.exists() {
        return Ok(CheckDatabase::default());
    }
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read checks db: {}", path.display()))?;
    let db: CheckDatabase = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse checks db: {}", path.display()))?;
    Ok(db)
}

fn save_checks_db(path: &Path, db: &CheckDatabase) -> Result<()> {
    let content =
        serde_json::to_string_pretty(db).with_context(|| "Failed to serialize checks db")?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write checks db: {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Module path helpers
// ---------------------------------------------------------------------------

fn parse_module_path(path: &str) -> Result<(&str, Vec<&str>)> {
    let parts: Vec<&str> = path.split("::").collect();
    if parts.len() < 2 {
        anyhow::bail!(
            "Invalid module path '{}'. Expected format: crate::module or crate::module::submodule",
            path
        );
    }
    let crate_name = parts[0];
    let module_segments = parts[1..].to_vec();
    Ok((crate_name, module_segments))
}

#[derive(Debug)]
enum ModuleLocation {
    File(PathBuf),
    Directory(PathBuf),
}

/// Resolve a module path like ["models", "db"] within a crate to its file location.
fn resolve_module_file(crate_dir: &Path, module_segments: &[&str]) -> Result<ModuleLocation> {
    let src_dir = crate_dir.join("src");
    let entry = if src_dir.join("lib.rs").exists() {
        src_dir.join("lib.rs")
    } else if src_dir.join("main.rs").exists() {
        src_dir.join("main.rs")
    } else {
        anyhow::bail!("No src/lib.rs or src/main.rs in {}", crate_dir.display());
    };

    let mut current_file = entry;
    let mut current_dir = src_dir.clone();

    for (i, segment) in module_segments.iter().enumerate() {
        let content = fs::read_to_string(&current_file)
            .with_context(|| format!("Failed to read {}", current_file.display()))?;
        let file = syn::parse_file(&content)
            .with_context(|| format!("Failed to parse {}", current_file.display()))?;

        let mut found = false;
        for item in &file.items {
            if let syn::Item::Mod(item_mod) = item {
                if item_mod.ident == segment {
                    if item_mod.content.is_some() {
                        anyhow::bail!(
                            "Module '{}' is inline (declared with `mod {} {{ ... }}`) — cannot open/close inline modules",
                            segment,
                            segment
                        );
                    }
                    found = true;
                    break;
                }
            }
        }

        if !found {
            anyhow::bail!(
                "Module '{}' not found in {}",
                segment,
                current_file.display()
            );
        }

        // Check if this is the last segment
        let is_last = i == module_segments.len() - 1;

        // Try file module first: dir/segment.rs
        let file_mod = current_dir.join(format!("{}.rs", segment));
        if file_mod.exists() {
            if is_last {
                return Ok(ModuleLocation::File(file_mod));
            }
            current_file = file_mod;
            current_dir = current_dir.join(segment);
            continue;
        }

        // Try directory module: dir/segment/mod.rs
        let dir_mod = current_dir.join(segment).join("mod.rs");
        if dir_mod.exists() {
            if is_last {
                return Ok(ModuleLocation::Directory(dir_mod));
            }
            current_file = dir_mod;
            current_dir = current_dir.join(segment);
            continue;
        }

        anyhow::bail!(
            "Module '{}' declared in {} but no source file found (tried {} and {})",
            segment,
            current_file.display(),
            file_mod.display(),
            dir_mod.display()
        );
    }

    unreachable!()
}

/// Build the full dotted name for a module .sb file inside a crate.
fn module_sb_name(crate_name: &str, sb_path: &Path, crate_src_dir: &Path) -> Result<String> {
    let rel = sb_path
        .strip_prefix(crate_src_dir)
        .with_context(|| format!("Cannot get relative path for {}", sb_path.display()))?;
    // rel is like "models.sb" or "models/db.sb"
    let mut parts: Vec<String> = Vec::new();
    parts.push(crate_name.to_string());

    for component in rel.parent().iter().flat_map(|p| p.components()) {
        if let std::path::Component::Normal(name) = component {
            parts.push(name.to_string_lossy().to_string());
        }
    }

    let stem = rel
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !stem.is_empty() {
        parts.push(stem.to_string());
    }

    Ok(parts.join("::"))
}

/// Find all module .sb files recursively inside an opened crate directory.
fn find_module_specs(crate_dir: &Path, crate_name: &str) -> Result<Vec<(String, PathBuf)>> {
    let src_dir = crate_dir.join("src");
    if !src_dir.exists() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    for entry in walkdir::WalkDir::new(&src_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("sb"))
    {
        let name = module_sb_name(crate_name, entry.path(), &src_dir)?;
        results.push((name, entry.path().to_path_buf()));
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Package discovery
// ---------------------------------------------------------------------------

fn get_top_level_closed_packages(app_dir: &Path) -> Result<Vec<String>> {
    let mut packages = Vec::new();
    for entry in fs::read_dir(app_dir)?.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with(SB_EXT) {
            let pkg_name = name_str[..name_str.len() - SB_EXT.len()].to_string();
            packages.push(pkg_name);
        }
    }
    Ok(packages)
}

fn _get_closed_packages(app_dir: &Path) -> Result<Vec<String>> {
    let mut packages = get_top_level_closed_packages(app_dir)?;
    for entry in fs::read_dir(app_dir)?.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let hidden = entry.path().join(SPEC_HIDDEN);
            if hidden.exists() {
                // Opened crate — scan for nested module .sb files
                let crate_name = entry.file_name().to_string_lossy().to_string();
                match find_module_specs(&entry.path(), &crate_name) {
                    Ok(mods) => {
                        for (name, _) in mods {
                            packages.push(name);
                        }
                    }
                    Err(e) => {
                        eprintln!("  [discover] Warning: failed to scan {}: {}", entry.path().display(), e);
                    }
                }
            }
        }
    }
    Ok(packages)
}

fn get_opened_packages(app_dir: &Path) -> Result<Vec<String>> {
    let mut packages = Vec::new();
    for entry in fs::read_dir(app_dir)?.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let hidden = entry.path().join(SPEC_HIDDEN);
            if hidden.exists() {
                packages.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    Ok(packages)
}

fn get_all_packages(app_dir: &Path) -> Result<Vec<String>> {
    let mut set = std::collections::HashSet::new();
    for pkg in get_top_level_closed_packages(app_dir)? {
        set.insert(pkg);
    }
    for pkg in get_opened_packages(app_dir)? {
        set.insert(pkg);
    }
    let mut packages: Vec<_> = set.into_iter().collect();
    packages.sort();
    Ok(packages)
}

// ---------------------------------------------------------------------------
// Temp workspace (shared by build and check)
// ---------------------------------------------------------------------------

fn setup_temp_workspace(app_dir: &Path) -> Result<(PathBuf, Vec<String>)> {
    let mut packages: Vec<(String, SpecFile, bool)> = Vec::new();

    for entry in fs::read_dir(app_dir)?.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.ends_with(SB_EXT) {
            let pkg_name = name_str[..name_str.len() - SB_EXT.len()].to_string();
            let spec = read_spec(&entry.path())?;
            packages.push((pkg_name, spec, false));
        } else if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let hidden = entry.path().join(SPEC_HIDDEN);
            if hidden.exists() {
                let spec = read_spec(&hidden)?;
                packages.push((name_str.to_string(), spec, true));
            }
        }
    }

    if packages.is_empty() {
        anyhow::bail!("No packages found.");
    }

    let temp_dir = app_dir.join(".specbuild-tmp");
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }
    fs::create_dir_all(&temp_dir)?;

    let mut member_names = Vec::new();
    for (name, spec, is_open) in &packages {
        let src = if *is_open {
            app_dir.join(name)
        } else {
            app_dir.join(&spec.source.path)
        };
        let dst = temp_dir.join(name);
        copy_dir_all(&src, &dst)?;
        let hidden = dst.join(SPEC_HIDDEN);
        remove_file_if_exists(&hidden)?;
        member_names.push(name.clone());
    }

    let members_toml = member_names
        .iter()
        .map(|n| format!("\"{}\"", n))
        .collect::<Vec<_>>()
        .join(", ");
    let workspace_toml = format!(
        "[workspace]\nmembers = [{}]\nresolver = \"2\"\n",
        members_toml
    );
    fs::write(temp_dir.join("Cargo.toml"), workspace_toml)?;

    Ok((temp_dir, member_names))
}

// ---------------------------------------------------------------------------
// Check logic
// ---------------------------------------------------------------------------

fn run_checks(
    package: &str,
    _app_dir: &Path,
    temp_dir: &Path,
    spec: &SpecFile,
    _source_path: &Path,
) -> Result<(CheckResult, String)> {
    // 1. Spec validity check
    if spec.spec.invariants.is_empty() && spec.spec.verify_command.is_none() {
        return Ok((
            CheckResult::Bugged,
            "No invariants or verify_command defined in spec".to_string(),
        ));
    }

    // Check for empty or whitespace-only invariants
    for inv in &spec.spec.invariants {
        if inv.trim().is_empty() {
            return Ok((
                CheckResult::Bugged,
                "Empty invariant found in spec".to_string(),
            ));
        }
    }

    // 2. Run cargo test in temp workspace
    let output = std::process::Command::new("cargo")
        .arg("test")
        .arg("-p")
        .arg(package)
        .current_dir(temp_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .with_context(|| "Failed to run cargo test")?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut reason = format!("cargo test failed for package '{}'\n", package);
        if !stdout.is_empty() {
            reason.push_str(&format!("\nSTDOUT:\n{}", stdout));
        }
        if !stderr.is_empty() {
            reason.push_str(&format!("\nSTDERR:\n{}", stderr));
        }
        return Ok((CheckResult::Failed, reason));
    }

    // 3. Run spec verify_command if present
    if let Some(cmd_str) = &spec.spec.verify_command {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd_str)
            .current_dir(_source_path)
            .output()
            .with_context(|| format!("Failed to run verify_command: {}", cmd_str))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let reason = if !stderr.is_empty() {
                format!("verify_command failed: {}", stderr.trim())
            } else {
                format!("verify_command failed: {}", stdout.trim())
            };
            return Ok((CheckResult::Failed, reason));
        }
    }

    // 4. Run external AI checker if configured
    if let Ok(ai_checker) = std::env::var("SPECBUILD_AI_CHECKER") {
        let output = std::process::Command::new(&ai_checker)
            .arg(package)
            .current_dir(_app_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .with_context(|| format!("Failed to run AI checker: {}", ai_checker))?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Try to parse as JSON
        if let Ok(ai_result) = serde_json::from_str::<AiCheckResult>(&stdout) {
            return Ok((ai_result.result, ai_result.reason));
        }

        // Fallback: if exit code is 0, passed; non-zero, failed
        if output.status.success() {
            return Ok((
                CheckResult::Passed,
                format!("AI check passed: {}", stdout.trim()),
            ));
        } else {
            return Ok((
                CheckResult::Failed,
                format!("AI check failed: {}", stdout.trim()),
            ));
        }
    }

    Ok((CheckResult::Passed, "All checks passed".to_string()))
}

fn check_package(
    package: &str,
    app_dir: &Path,
    temp_dir: &Path,
    db: &mut CheckDatabase,
    db_path: &Path,
    force: bool,
) -> Result<(CheckResult, String)> {
    let sb_path = app_dir.join(format!("{}{}", package, SB_EXT));
    let pkg_dir = app_dir.join(package);

    // Determine spec and source path (open vs closed)
    let (spec, source_path, spec_path_for_hash) = if sb_path.exists() {
        let spec = read_spec(&sb_path)?;
        let source = app_dir.join(&spec.source.path);
        (spec, source, sb_path.clone())
    } else if pkg_dir.join(SPEC_HIDDEN).exists() {
        let spec = read_spec(&pkg_dir.join(SPEC_HIDDEN))?;
        (spec, pkg_dir.clone(), pkg_dir.join(SPEC_HIDDEN))
    } else {
        anyhow::bail!("Package '{}' not found", package);
    };

    // Compute hashes (use canonical toml for spec to ensure open/closed consistency)
    let spec_toml = toml::to_string_pretty(&spec)?;
    let spec_hash = hash_bytes(spec_toml.as_bytes());
    let source_hash = hash_source_dir(&source_path)?;
    let spec_modified = file_mtime(&spec_path_for_hash)?;
    let source_modified = max_mtime_source(&source_path)?;

    // Check cache
    if !force {
        if let Some(record) = db.records.get(package) {
            if record.spec_hash == spec_hash && record.source_hash == source_hash {
                return Ok((record.result.clone(), record.reason.clone()));
            }
        }
    }

    // Run checks
    let (result, reason) = run_checks(package, app_dir, temp_dir, &spec, &source_path)?;

    // Save to db
    db.records.insert(
        package.to_string(),
        CheckRecord {
            spec_hash,
            source_hash,
            spec_modified,
            source_modified,
            last_checked: now_unix(),
            result: result.clone(),
            reason: reason.clone(),
        },
    );
    save_checks_db(db_path, db)?;

    Ok((result, reason))
}

// ---------------------------------------------------------------------------
// Open / Close
// ---------------------------------------------------------------------------

fn open_package(package: &str, app_dir: &Path, auto_deps: bool) -> Result<()> {
    let sb_path = app_dir.join(format!("{}{}", package, SB_EXT));
    if !sb_path.exists() {
        anyhow::bail!(
            "Package '{}' is not closed (no .sb file found at {})",
            package,
            sb_path.display()
        );
    }

    let spec = read_spec(&sb_path)?;
    let source_path = app_dir.join(&spec.source.path);
    let target_path = app_dir.join(package);

    if !source_path.exists() {
        anyhow::bail!("Source directory does not exist: {}", source_path.display());
    }

    if auto_deps && !spec.interface.dependencies.is_empty() {
        for dep in &spec.interface.dependencies {
            let dep_sb = app_dir.join(format!("{}{}", dep, SB_EXT));
            if dep_sb.exists() {
                println!("  [dep] Auto-opening dependency '{}'...", dep);
                open_package(dep, app_dir, true)?;
            }
        }
    }

    copy_dir_all(&source_path, &target_path).with_context(|| {
        format!(
            "Failed to copy source from {} to {}",
            source_path.display(),
            target_path.display()
        )
    })?;

    let hidden_spec_path = target_path.join(SPEC_HIDDEN);
    write_spec(&hidden_spec_path, &spec)?;

    fs::remove_file(&sb_path)
        .with_context(|| format!("Failed to remove .sb file: {}", sb_path.display()))?;

    println!("Opened package '{}' -> {}", package, target_path.display());
    println!("  Source: {}", source_path.display());
    println!("  Inputs: {:?}", spec.interface.inputs);
    println!("  Outputs: {:?}", spec.interface.outputs);
    if !spec.interface.dependencies.is_empty() {
        println!("  Dependencies: {:?}", spec.interface.dependencies);
    }
    Ok(())
}

fn close_package(package: &str, app_dir: &Path, regenerate: bool) -> Result<()> {
    let pkg_dir = app_dir.join(package);
    if !pkg_dir.exists() || !pkg_dir.is_dir() {
        anyhow::bail!(
            "Package '{}' is not opened (no directory found at {})",
            package,
            pkg_dir.display()
        );
    }

    let hidden_spec_path = pkg_dir.join(SPEC_HIDDEN);
    if !hidden_spec_path.exists() {
        anyhow::bail!(
            "Opened package '{}' is missing its .specbuilt.sb metadata; cannot close safely.",
            package
        );
    }

    let mut spec = read_spec(&hidden_spec_path)?;
    let source_path = app_dir.join(&spec.source.path);

    remove_dir_all_if_exists(&source_path)?;
    copy_dir_all(&pkg_dir, &source_path).with_context(|| {
        format!(
            "Failed to sync source back to {}",
            source_path.display()
        )
    })?;

    let archived_hidden = source_path.join(SPEC_HIDDEN);
    remove_file_if_exists(&archived_hidden)?;

    // Optionally regenerate spec from updated source
    if regenerate {
        println!("  [close] Regenerating spec from source...");
        match extract::extract_crate_api(&source_path) {
            Ok(api_surface) => {
                spec.interface.api_surface = api_surface;
                println!("  [close] API surface regenerated");
            }
            Err(e) => {
                eprintln!("  [close] Warning: failed to regenerate API surface: {}", e);
            }
        }
    }

    let sb_path = app_dir.join(format!("{}{}", package, SB_EXT));
    write_spec(&sb_path, &spec)?;

    fs::remove_dir_all(&pkg_dir)
        .with_context(|| format!("Failed to remove opened package directory: {}", pkg_dir.display()))?;

    println!("Closed package '{}' -> {}", package, sb_path.display());
    println!("  Source synced back to: {}", source_path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Open / Close Module
// ---------------------------------------------------------------------------

fn open_module(path: &str, app_dir: &Path) -> Result<()> {
    let (crate_name, module_segments) = parse_module_path(path)?;
    let crate_dir = app_dir.join(crate_name);

    // Verify crate is open
    if !crate_dir.join(SPEC_HIDDEN).exists() {
        anyhow::bail!(
            "Crate '{}' is not opened. Open it first with `specbuild open {}`",
            crate_name,
            crate_name
        );
    }

    let sb_file_name = format!("{}{}", module_segments.last().unwrap(), SB_EXT);
    let sb_path = match resolve_module_file(&crate_dir, &module_segments)? {
        ModuleLocation::File(ref mod_path) => mod_path.with_file_name(&sb_file_name),
        ModuleLocation::Directory(ref mod_path) => mod_path.parent().unwrap().with_file_name(&sb_file_name),
    };

    if !sb_path.exists() {
        anyhow::bail!("Module '{}' is not closed (no .sb file at {})", path, sb_path.display());
    }

    let spec = read_spec(&sb_path)?;
    let source_path = app_dir.join(&spec.source.path);

    if !source_path.exists() {
        anyhow::bail!("Source does not exist: {}", source_path.display());
    }

    // Restore source based on whether the archived source is a file or directory
    let mod_path = resolve_module_file(&crate_dir, &module_segments)?;
    if source_path.is_file() {
        let target_path = match &mod_path {
            ModuleLocation::File(p) => p.clone(),
            ModuleLocation::Directory(p) => p.parent().unwrap().with_extension("rs"),
        };
        fs::copy(&source_path, &target_path).with_context(|| {
            format!(
                "Failed to restore source from {} to {}",
                source_path.display(),
                target_path.display()
            )
        })?;
    } else if source_path.is_dir() {
        let target_dir = match &mod_path {
            ModuleLocation::File(p) => p.parent().unwrap().join(p.file_stem().unwrap()),
            ModuleLocation::Directory(p) => p.parent().unwrap().to_path_buf(),
        };
        // Remove the stub file if it exists (e.g. utils.rs when restoring utils/ directory)
        let stub_file = target_dir.with_extension("rs");
        remove_file_if_exists(&stub_file)?;
        remove_dir_all_if_exists(&target_dir)?;
        copy_dir_all(&source_path, &target_dir).with_context(|| {
            format!(
                "Failed to restore source from {} to {}",
                source_path.display(),
                target_dir.display()
            )
        })?;
    } else {
        anyhow::bail!("Source path is neither a file nor a directory: {}", source_path.display());
    }

    fs::remove_file(&sb_path)
        .with_context(|| format!("Failed to remove module .sb file: {}", sb_path.display()))?;

    println!("Opened module '{}' -> restored from {}", path, source_path.display());
    Ok(())
}

fn close_module(path: &str, app_dir: &Path, _regenerate: bool) -> Result<()> {
    let (crate_name, module_segments) = parse_module_path(path)?;
    let crate_dir = app_dir.join(crate_name);

    // Verify crate is open
    if !crate_dir.join(SPEC_HIDDEN).exists() {
        anyhow::bail!(
            "Crate '{}' is not opened. Open it first with `specbuild open {}`",
            crate_name,
            crate_name
        );
    }

    let location = resolve_module_file(&crate_dir, &module_segments)?;
    let (mod_source_path, sb_path, is_directory) = match &location {
        ModuleLocation::File(mod_path) => {
            let sb = mod_path.with_file_name(format!("{}{}", module_segments.last().unwrap(), SB_EXT));
            (mod_path.clone(), sb, false)
        }
        ModuleLocation::Directory(mod_path) => {
            let parent = mod_path.parent().unwrap();
            let sb = parent.with_file_name(format!("{}{}", module_segments.last().unwrap(), SB_EXT));
            (parent.to_path_buf(), sb, true)
        }
    };

    if sb_path.exists() {
        anyhow::bail!("Module '{}' is already closed ({} exists)", path, sb_path.display());
    }

    // Extract API surface
    let api_surface = match &location {
        ModuleLocation::File(mod_path) => extract::extract_module_api(mod_path)?,
        ModuleLocation::Directory(mod_path) => extract::extract_module_api(mod_path)?,
    };

    // Determine canonical source path in specbuilt-source
    let crate_spec = read_spec(&crate_dir.join(SPEC_HIDDEN))?;
    let canonical_crate_source = app_dir.join(&crate_spec.source.path);
    let rel_src_path = relative_path(&crate_dir, &mod_source_path)?;
    let canonical_module_source = canonical_crate_source.join(&rel_src_path);

    // Archive source
    if is_directory {
        remove_dir_all_if_exists(&canonical_module_source)?;
        copy_dir_all(&mod_source_path, &canonical_module_source).with_context(|| {
            format!(
                "Failed to archive module source to {}",
                canonical_module_source.display()
            )
        })?;
    } else {
        if let Some(parent) = canonical_module_source.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&mod_source_path, &canonical_module_source).with_context(|| {
            format!(
                "Failed to archive module source to {}",
                canonical_module_source.display()
            )
        })?;
    }

    // Build spec
    let source_rel = relative_path(app_dir, &canonical_module_source)?;
    let spec = SpecFile {
        package: PackageSpec {
            name: module_segments.last().unwrap().to_string(),
            version: crate_spec.package.version.clone(),
            description: String::new(),
        },
        interface: InterfaceSpec {
            inputs: Vec::new(),
            outputs: Vec::new(),
            dependencies: Vec::new(),
            api_surface,
        },
        source: SourceSpec { path: source_rel },
        test: TestSpec::default(),
        spec: SpecSection::default(),
    };

    write_spec(&sb_path, &spec)?;

    // Generate stub at original location
    match &location {
        ModuleLocation::File(mod_path) => {
            stub::generate_module_stub(&sb_path, mod_path)?;
        }
        ModuleLocation::Directory(mod_path) => {
            let mod_dir = mod_path.parent().unwrap();
            let stub_path = mod_dir.with_extension("rs");
            remove_dir_all_if_exists(mod_dir)?;
            stub::generate_module_stub(&sb_path, &stub_path)?;
        }
    }

    println!("Closed module '{}' -> {}", path, sb_path.display());
    println!("  Source archived to: {}", canonical_module_source.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

fn format_timestamp(ts: u64) -> String {
    if ts == 0 {
        "never".to_string()
    } else {
        let now = now_unix();
        let diff = now.saturating_sub(ts);
        if diff < 60 {
            format!("{}s ago", diff)
        } else if diff < 3600 {
            format!("{}m ago", diff / 60)
        } else if diff < 86400 {
            format!("{}h ago", diff / 3600)
        } else {
            format!("{}d ago", diff / 86400)
        }
    }
}

fn list_packages(app_dir: &Path) -> Result<()> {
    // Try to load check db for display
    let check_db = get_checks_db_path(app_dir)
        .ok()
        .and_then(|p| load_checks_db(&p).ok())
        .unwrap_or_default();

    println!("Packages in {}", app_dir.display());
    println!(
        "{:<30} {:<10} {:<10} {}",
        "NAME", "STATUS", "CHECK", "DETAILS"
    );
    println!("{}", "-".repeat(80));

    let mut found = false;
    if let Ok(entries) = fs::read_dir(app_dir) {
        let mut all_entries: Vec<_> = entries.flatten().collect();
        all_entries.sort_by_key(|e| e.file_name());

        for entry in all_entries {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if name_str.ends_with(SB_EXT) {
                found = true;
                let pkg_name = &name_str[..name_str.len() - SB_EXT.len()];
                let spec = read_spec(&entry.path()).ok();
                let details = spec
                    .map(|s| {
                        let mut d = String::new();
                        if !s.interface.inputs.is_empty() {
                            d.push_str(&format!("inputs:{} ", s.interface.inputs.len()));
                        }
                        if !s.interface.outputs.is_empty() {
                            d.push_str(&format!("outputs:{} ", s.interface.outputs.len()));
                        }
                        if !s.package.description.is_empty() {
                            d.push_str(&format!("| {}", s.package.description));
                        }
                        d
                    })
                    .unwrap_or_default();

                let check_status = check_db
                    .records
                    .get(pkg_name)
                    .map(|r| format!("{} ({})", r.result, format_timestamp(r.last_checked)))
                    .unwrap_or_else(|| "-".to_string());

                println!(
                    "{:<30} {:<10} {:<10} {}",
                    pkg_name,
                    "closed",
                    check_status,
                    details.trim()
                );
            } else if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let hidden = entry.path().join(SPEC_HIDDEN);
                if hidden.exists() {
                    found = true;
                    let spec = read_spec(&hidden).ok();
                    let details = spec
                        .map(|s| {
                            let mut d = String::new();
                            if !s.interface.inputs.is_empty() {
                                d.push_str(&format!("inputs:{} ", s.interface.inputs.len()));
                            }
                            if !s.interface.outputs.is_empty() {
                                d.push_str(&format!("outputs:{} ", s.interface.outputs.len()));
                            }
                            d
                        })
                        .unwrap_or_default();

                    let pkg_name = name_str.to_string();
                    let check_status = check_db
                        .records
                        .get(&pkg_name)
                        .map(|r| format!("{} ({})", r.result, format_timestamp(r.last_checked)))
                        .unwrap_or_else(|| "-".to_string());

                    println!(
                        "{:<30} {:<10} {:<10} {}",
                        name_str,
                        "OPEN",
                        check_status,
                        details.trim()
                    );

                    // List nested closed modules inside this opened crate
                    match find_module_specs(&entry.path(), &pkg_name) {
                        Ok(mods) => {
                            for (mod_name, mod_sb_path) in mods {
                                let mod_spec = read_spec(&mod_sb_path).ok();
                                let mod_details = mod_spec
                                    .map(|s| {
                                        if !s.package.description.is_empty() {
                                            format!("| {}", s.package.description)
                                        } else {
                                            String::new()
                                        }
                                    })
                                    .unwrap_or_default();
                                println!(
                                    "  {:<28} {:<10} {:<10} {}",
                                    mod_name,
                                    "closed",
                                    "-",
                                    mod_details.trim()
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("  [list] Warning: failed to scan {}: {}", entry.path().display(), e);
                        }
                    }
                }
            }
        }
    }

    if !found {
        println!("No packages found.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

fn build_project(app_dir: &Path) -> Result<()> {
    let (temp_dir, member_names) = setup_temp_workspace(app_dir)?;

    println!(
        "Building {} package(s) in {}...",
        member_names.len(),
        temp_dir.display()
    );
    let status = std::process::Command::new("cargo")
        .arg("build")
        .arg("--workspace")
        .current_dir(&temp_dir)
        .status()
        .with_context(|| "Failed to run cargo build. Is cargo installed?")?;

    if !status.success() {
        anyhow::bail!("cargo build failed");
    }
    println!("Build succeeded.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Base
// ---------------------------------------------------------------------------

pub fn discover_sb_dir(app_dir: &Path) -> Result<PathBuf> {
    let app_name = app_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("Cannot determine directory name"))?;

    if !app_name.ends_with("-rs") {
        anyhow::bail!(
            "Expected current directory to end with '-rs' (e.g. 'application-rs'), got: {}",
            app_name
        );
    }

    let base_name = &app_name[..app_name.len() - 3];
    let sb_name = format!("{}-sb", base_name);

    let parent = app_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot get parent of {}", app_dir.display()))?;

    let sibling_sb = parent.join(&sb_name);
    if sibling_sb.exists() && sibling_sb.is_dir() {
        return Ok(sibling_sb);
    }

    let parent_name = parent.file_name().and_then(|n| n.to_str());
    if parent_name == Some(&sb_name) {
        return Ok(parent.to_path_buf());
    }

    anyhow::bail!(
        "Could not find '{}' directory (looked as sibling of '{}' and as parent)",
        sb_name,
        app_name
    );
}

fn base_workspace(app_dir: &Path) -> Result<()> {
    let sb_dir = discover_sb_dir(app_dir)?;
    let source_dir = sb_dir.join("specbuilt-source");

    if !source_dir.exists() || !source_dir.is_dir() {
        anyhow::bail!(
            "specbuilt-source directory not found in {}",
            sb_dir.display()
        );
    }

    let mut crates: Vec<(String, CargoToml, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&source_dir)?.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let cargo_toml = entry.path().join("Cargo.toml");
            if cargo_toml.exists() {
                let content = fs::read_to_string(&cargo_toml)
                    .with_context(|| format!("Failed to read {}", cargo_toml.display()))?;
                let cargo: CargoToml = toml::from_str(&content)
                    .with_context(|| format!("Failed to parse {}", cargo_toml.display()))?;
                crates.push((cargo.package.name.clone(), cargo, entry.path()));
            }
        }
    }

    if crates.is_empty() {
        anyhow::bail!("No Rust crates found in {}", source_dir.display());
    }

    let internal_names: std::collections::HashSet<String> =
        crates.iter().map(|(_, cargo, _)| cargo.package.name.clone()).collect();

    let mut generated = 0;
    for (name, cargo, crate_path) in &crates {
        let sb_path = app_dir.join(format!("{}{}", name, SB_EXT));

        let mut deps = Vec::new();
        for (dep_name, _) in cargo.dependencies.iter() {
            if internal_names.contains(dep_name.as_str()) {
                deps.push(dep_name.clone());
            }
        }
        for (dep_name, _) in cargo.dev_dependencies.iter() {
            if internal_names.contains(dep_name.as_str()) && !deps.contains(dep_name) {
                deps.push(dep_name.clone());
            }
        }

        // Also infer dependencies from `use` statements in source
        match infer_use_deps(crate_path, &internal_names, name) {
            Ok(use_deps) => {
                for dep in use_deps {
                    if !deps.contains(&dep) {
                        deps.push(dep);
                    }
                }
            }
            Err(e) => {
                eprintln!("  [base] Warning: failed to infer use-deps for {}: {}", name, e);
            }
        }

        let rel_path = relative_path(app_dir, crate_path)?;

        println!("  [base] Extracting API for '{}'...", name);
        let api_surface = extract::extract_crate_api(crate_path).unwrap_or_else(|e| {
            eprintln!("  [base] Warning: failed to extract API for {}: {}", name, e);
            String::new()
        });

        let spec = SpecFile {
            package: PackageSpec {
                name: name.clone(),
                version: cargo.package.version.clone(),
                description: cargo.package.description.clone(),
            },
            interface: InterfaceSpec {
                inputs: Vec::new(),
                outputs: Vec::new(),
                dependencies: deps,
                api_surface,
            },
            source: SourceSpec { path: rel_path },
            test: TestSpec {
                command: format!("cargo test -p {}", name),
            },
            spec: SpecSection::default(),
        };

        write_spec(&sb_path, &spec)?;
        println!("Generated {}", sb_path.display());
        generated += 1;
    }

    println!("\nBased {} crate(s) from {}", generated, source_dir.display());
    println!("Run `specbuild list` to see packages.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Dependency inference from `use` statements
// ---------------------------------------------------------------------------

fn infer_use_deps(
    crate_dir: &Path,
    internal_names: &std::collections::HashSet<String>,
    self_name: &str,
) -> Result<Vec<String>> {
    let mut deps = std::collections::HashSet::new();
    let src_dir = crate_dir.join("src");
    if !src_dir.exists() {
        return Ok(Vec::new());
    }

    for entry in walkdir::WalkDir::new(&src_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"))
    {
        let content = fs::read_to_string(entry.path())?;
        let file = match syn::parse_file(&content) {
            Ok(f) => f,
            Err(_) => continue,
        };
        for item in &file.items {
            if let syn::Item::Use(item_use) = item {
                extract_use_deps(&item_use.tree, internal_names, self_name, &mut deps);
            }
        }
    }

    let mut result: Vec<_> = deps.into_iter().collect();
    result.sort();
    Ok(result)
}

fn extract_use_deps(
    tree: &syn::UseTree,
    internal_names: &std::collections::HashSet<String>,
    self_name: &str,
    deps: &mut std::collections::HashSet<String>,
) {
    match tree {
        syn::UseTree::Path(p) => {
            let ident = p.ident.to_string();
            if internal_names.contains(&ident) && ident != self_name {
                deps.insert(ident);
            } else {
                extract_use_deps(&p.tree, internal_names, self_name, deps);
            }
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                extract_use_deps(item, internal_names, self_name, deps);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// AI Agent helpers
// ---------------------------------------------------------------------------

fn get_ai_agent(agent_arg: Option<String>) -> Option<String> {
    agent_arg.or_else(|| std::env::var("SPECBUILD_AI_AGENT").ok())
}

fn run_ai_agent(
    action: &str,
    package: &str,
    app_dir: &Path,
    _sb_path: &Path,
    _source_path: &Path,
    prompt: Option<&str>,
    agent: Option<String>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<bool> {
    // Try Rig-based agent first (if provider or API keys are configured)
    if provider.is_some()
        || std::env::var("SPECBUILD_AI_PROVIDER").is_ok()
        || std::env::var("OPENAI_API_KEY").is_ok()
        || std::env::var("ANTHROPIC_API_KEY").is_ok()
    {
        match agent::run_agent(action, package, app_dir, prompt, provider, model) {
            Ok(true) => return Ok(true),
            Ok(false) => {
                println!("  [agent] Rig agent did not complete successfully; falling back...");
            }
            Err(e) => {
                println!("  [agent] Rig agent error: {}; falling back...", e);
            }
        }
    }

    // Fall back to external agent command
    let Some(agent_cmd) = get_ai_agent(agent) else {
        return Ok(false);
    };

    let mut cmd = std::process::Command::new(&agent_cmd);
    cmd.arg(action).arg(package).arg(_sb_path).arg(_source_path);

    if let Some(p) = prompt {
        cmd.arg(p);
    }

    let status = cmd
        .status()
        .with_context(|| format!("Failed to run AI agent: {}", agent_cmd))?;

    Ok(status.success())
}

// ---------------------------------------------------------------------------
// Backup & clear helpers
// ---------------------------------------------------------------------------

fn backup_package(pkg_dir: &Path, app_dir: &Path) -> Result<PathBuf> {
    let backup_dir = app_dir.join(".specbuild-backup");
    let pkg_name = pkg_dir.file_name().unwrap_or_default();
    let backup = backup_dir.join(pkg_name);

    if backup.exists() {
        fs::remove_dir_all(&backup)?;
    }
    fs::create_dir_all(&backup_dir)?;
    copy_dir_all(pkg_dir, &backup)?;

    println!("  [backup] Saved to {}", backup.display());
    Ok(backup)
}

fn clear_rs_files(dir: &Path) -> Result<usize> {
    let mut count = 0usize;
    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry?;
        if entry.file_type().is_file()
            && entry.path().extension().and_then(|s| s.to_str()) == Some("rs")
        {
            fs::remove_file(entry.path())?;
            count += 1;
        }
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// Fix / Doover
// ---------------------------------------------------------------------------

fn run_check_loop(
    package: &str,
    app_dir: &Path,
    db: &mut CheckDatabase,
    db_path: &Path,
    retries: u32,
    provider: Option<String>,
    model: Option<String>,
) -> Result<(CheckResult, String)> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let (temp_dir, _) = setup_temp_workspace(app_dir)?;

        match check_package(package, app_dir, &temp_dir, db, db_path, true) {
            Ok((CheckResult::Passed, reason)) => {
                let _ = fs::remove_dir_all(&temp_dir);
                return Ok((CheckResult::Passed, reason));
            }
            Ok((result, reason)) => {
                let _ = fs::remove_dir_all(&temp_dir);
                if attempt > retries {
                    return Ok((result, reason));
                }
                println!("  [check] {} (attempt {}/{}): {}", result, attempt, retries + 1, reason);

                // If a rig provider is available, ask the agent to diagnose and fix
                if provider.is_some()
                    || std::env::var("SPECBUILD_AI_PROVIDER").is_ok()
                    || std::env::var("OPENAI_API_KEY").is_ok()
                    || std::env::var("ANTHROPIC_API_KEY").is_ok()
                {
                    println!("  [check] Invoking agent to fix failure...");
                    let fix_prompt = format!(
                        "The checks for package '{}' failed. Here is the error output:\n\n{}\n\n\
                        Please diagnose the issue, fix it in the source files, and verify with cargo test.",
                        package, reason
                    );
                    match agent::run_agent(
                        "fix-check-failure",
                        package,
                        app_dir,
                        Some(&fix_prompt),
                        provider.clone(),
                        model.clone(),
                    ) {
                        Ok(true) => println!("  [check] Agent completed a fix attempt"),
                        Ok(false) => println!("  [check] Agent did not complete"),
                        Err(e) => println!("  [check] Agent error: {}", e),
                    }
                }
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&temp_dir);
                if attempt > retries {
                    return Err(e);
                }
                println!("  [check] Error (attempt {}/{}): {}", attempt, retries + 1, e);
            }
        }
    }
}

fn fix_package(
    package: &str,
    app_dir: &Path,
    prompt: Option<&str>,
    retries: u32,
    no_close: bool,
    agent: Option<String>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let sb_path = app_dir.join(format!("{}{}", package, SB_EXT));
    let pkg_dir = app_dir.join(package);

    // Read current spec
    let mut spec = if sb_path.exists() {
        read_spec(&sb_path)?
    } else if pkg_dir.join(SPEC_HIDDEN).exists() {
        read_spec(&pkg_dir.join(SPEC_HIDDEN))?
    } else {
        anyhow::bail!("Package '{}' not found", package);
    };

    // Update spec with prompt
    if let Some(p) = prompt {
        println!("[fix] Updating spec for '{}' with prompt: {}", package, p);

        // If an AI agent is configured, let it rewrite the spec first
        if let Some(ref agent_cmd) = get_ai_agent(agent.clone()) {
            println!("  [fix] Asking AI agent to rewrite spec...");
            let status = std::process::Command::new(agent_cmd)
                .arg("rewrite-spec")
                .arg(package)
                .arg(&sb_path)
                .arg(app_dir.join(&spec.source.path))
                .arg(p)
                .status()
                .with_context(|| format!("Failed to run AI agent: {}", agent_cmd))?;

            if status.success() {
                // Re-read the potentially updated spec
                spec = if sb_path.exists() {
                    read_spec(&sb_path)?
                } else if pkg_dir.join(SPEC_HIDDEN).exists() {
                    read_spec(&pkg_dir.join(SPEC_HIDDEN))?
                } else {
                    spec
                };
                println!("  [fix] Spec rewritten by agent");
            }
        } else {
            // Manual update: append prompt to invariants and description
            if !spec.spec.invariants.contains(&p.to_string()) {
                spec.spec.invariants.push(p.to_string());
            }
            if spec.spec.description.is_empty() {
                spec.spec.description = p.to_string();
            }
            println!("  [fix] Added prompt to spec invariants");
        }
    }

    // Ensure package is open
    let was_already_open = pkg_dir.join(SPEC_HIDDEN).exists();
    if !was_already_open {
        if sb_path.exists() {
            open_package(package, app_dir, true)?;
        } else {
            anyhow::bail!("Package '{}' cannot be opened — no .sb file found", package);
        }
    }

    // Write updated spec to hidden spec
    let hidden = pkg_dir.join(SPEC_HIDDEN);
    write_spec(&hidden, &spec)?;

    // Also write to .sb if it exists (for closed packages that we might not close)
    if sb_path.exists() {
        write_spec(&sb_path, &spec)?;
    }

    // Run AI agent to implement changes
    let source_path = app_dir.join(package);
    let agent_ran = run_ai_agent(
        "fix",
        package,
        app_dir,
        &sb_path,
        &source_path,
        prompt,
        agent.clone(),
        provider.clone(),
        model.clone(),
    )?;

    if !agent_ran {
        println!(
            "  [fix] No AI agent configured (set SPECBUILD_AI_AGENT or use --agent)"
        );
        println!(
            "  [fix] Package is open at {} — implement changes manually",
            source_path.display()
        );
    } else {
        println!("  [fix] AI agent completed");

        // Run check loop
        let db_path = get_checks_db_path(app_dir)?;
        let mut db = load_checks_db(&db_path)?;

        println!("  [fix] Running check loop (max {} retries)...", retries + 1);
        match run_check_loop(package, app_dir, &mut db, &db_path, retries, provider.clone(), model.clone()) {
            Ok((CheckResult::Passed, _)) => {
                println!("  [fix] Check passed!");
            }
            Ok((result, reason)) => {
                println!("  [fix] Check final result: {} — {}", result, reason);
            }
            Err(e) => {
                println!("  [fix] Check failed permanently: {}", e);
            }
        }
    }

    if !no_close {
        close_package(package, app_dir, true)?;
    } else {
        println!("  [fix] Package left open at {}", pkg_dir.display());
    }

    Ok(())
}

fn doover_package(
    package: &str,
    app_dir: &Path,
    prompt: Option<&str>,
    retries: u32,
    no_close: bool,
    agent: Option<String>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let sb_path = app_dir.join(format!("{}{}", package, SB_EXT));
    let pkg_dir = app_dir.join(package);

    // Read spec
    let spec = if sb_path.exists() {
        read_spec(&sb_path)?
    } else if pkg_dir.join(SPEC_HIDDEN).exists() {
        read_spec(&pkg_dir.join(SPEC_HIDDEN))?
    } else {
        anyhow::bail!("Package '{}' not found", package);
    };

    // Ensure package is open
    let was_already_open = pkg_dir.join(SPEC_HIDDEN).exists();
    if !was_already_open {
        if sb_path.exists() {
            open_package(package, app_dir, true)?;
        } else {
            anyhow::bail!("Package '{}' cannot be opened — no .sb file found", package);
        }
    }

    // Backup current source
    println!("[doover] Backing up '{}'...", package);
    backup_package(&pkg_dir, app_dir)?;

    // Clear .rs files (keep Cargo.toml, config, etc.)
    println!("[doover] Clearing source files...");
    let cleared = clear_rs_files(&pkg_dir)?;
    println!("  [doover] Removed {} .rs file(s)", cleared);

    // Preserve hidden spec
    let hidden = pkg_dir.join(SPEC_HIDDEN);
    write_spec(&hidden, &spec)?;

    // Run AI agent to rebuild from spec
    let source_path = app_dir.join(package);
    let agent_ran = run_ai_agent(
        "doover",
        package,
        app_dir,
        &sb_path,
        &source_path,
        prompt,
        agent.clone(),
        provider.clone(),
        model.clone(),
    )?;

    if !agent_ran {
        println!(
            "  [doover] No AI agent configured (set SPECBUILD_AI_AGENT or use --agent)"
        );
        println!(
            "  [doover] Package is open and cleared at {} — implement manually",
            source_path.display()
        );
    } else {
        println!("  [doover] AI agent completed");

        // Run check loop
        let db_path = get_checks_db_path(app_dir)?;
        let mut db = load_checks_db(&db_path)?;

        println!("  [doover] Running check loop (max {} retries)...", retries + 1);
        match run_check_loop(package, app_dir, &mut db, &db_path, retries, provider.clone(), model.clone()) {
            Ok((CheckResult::Passed, _)) => {
                println!("  [doover] Check passed!");
            }
            Ok((result, reason)) => {
                println!("  [doover] Check final result: {} — {}", result, reason);
            }
            Err(e) => {
                println!("  [doover] Check failed permanently: {}", e);
            }
        }
    }

    if !no_close {
        close_package(package, app_dir, true)?;
    } else {
        println!("  [doover] Package left open at {}", pkg_dir.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Open { package, app_dir } => {
            let app_dir = get_app_dir(app_dir)?;
            open_package(&package, &app_dir, true)?;
        }
        Commands::Close {
            package,
            regenerate,
            app_dir,
        } => {
            let app_dir = get_app_dir(app_dir)?;
            close_package(&package, &app_dir, regenerate)?;
        }
        Commands::List { app_dir } => {
            let app_dir = get_app_dir(app_dir)?;
            list_packages(&app_dir)?;
        }
        Commands::Build { app_dir } => {
            let app_dir = get_app_dir(app_dir)?;
            build_project(&app_dir)?;
        }
        Commands::OpenAll { app_dir } => {
            let app_dir = get_app_dir(app_dir)?;
            let closed = get_top_level_closed_packages(&app_dir)?;
            if closed.is_empty() {
                println!("All packages are already open.");
            } else {
                for pkg in &closed {
                    open_package(pkg, &app_dir, true)?;
                }
            }
        }
        Commands::CloseAll { app_dir } => {
            let app_dir = get_app_dir(app_dir)?;
            let opened = get_opened_packages(&app_dir)?;
            if opened.is_empty() {
                println!("All packages are already closed.");
            } else {
                for pkg in &opened {
                    close_package(pkg, &app_dir, false)?;
                }
            }
        }
        Commands::Base { app_dir } => {
            let app_dir = get_app_dir_or_cwd(app_dir)?;
            base_workspace(&app_dir)?;
        }
        Commands::Check {
            package,
            force,
            app_dir,
        } => {
            let app_dir = get_app_dir(app_dir)?;
            let db_path = get_checks_db_path(&app_dir)?;
            let mut db = load_checks_db(&db_path)?;

            let packages = match package {
                Some(p) => vec![p],
                None => get_all_packages(&app_dir)?,
            };

            if packages.is_empty() {
                println!("No packages found.");
                return Ok(());
            }

            // Setup temp workspace once for all cargo test invocations
            let (temp_dir, _) = setup_temp_workspace(&app_dir)?;

            let mut passed = 0usize;
            let mut failed = 0usize;
            let mut bugged = 0usize;

            for pkg in &packages {
                print!("Checking {} ... ", pkg);
                std::io::Write::flush(&mut std::io::stdout())?;
                match check_package(pkg, &app_dir, &temp_dir, &mut db, &db_path, force) {
                    Ok((result, reason)) => {
                        println!("{}", result);
                        if !reason.is_empty() && result != CheckResult::Passed {
                            println!("  -> {}", reason);
                        }
                        match result {
                            CheckResult::Passed => passed += 1,
                            CheckResult::Failed => failed += 1,
                            CheckResult::Bugged => bugged += 1,
                        }
                    }
                    Err(e) => {
                        println!("ERROR");
                        println!("  -> {}", e);
                    }
                }
            }

            // Clean up temp workspace
            let _ = fs::remove_dir_all(&temp_dir);

            println!();
            println!(
                "Results: {} passed, {} failed, {} bugged",
                passed, failed, bugged
            );
        }
        Commands::Fix {
            package,
            prompt,
            retries,
            no_close,
            agent,
            provider,
            model,
            app_dir,
        } => {
            let app_dir = get_app_dir(app_dir)?;
            fix_package(
                &package,
                &app_dir,
                prompt.as_deref(),
                retries,
                no_close,
                agent,
                provider,
                model,
            )?;
        }
        Commands::Doover {
            package,
            prompt,
            retries,
            no_close,
            agent,
            provider,
            model,
            app_dir,
        } => {
            let app_dir = get_app_dir(app_dir)?;
            doover_package(
                &package,
                &app_dir,
                prompt.as_deref(),
                retries,
                no_close,
                agent,
                provider,
                model,
            )?;
        }
        Commands::Index { app_dir } => {
            let app_dir = get_app_dir(app_dir)?;
            let sb_dir = discover_sb_dir(&app_dir)?;
            let source_dir = sb_dir.join("specbuilt-source");
            println!("Indexing symbols from {}...", source_dir.display());
            let symbol_index = index::build_index(&source_dir)?;
            let index_path = index::get_index_path(&app_dir)?;
            index::save_index(&symbol_index, &index_path)?;
            let total: usize = symbol_index.crates.values().map(|v| v.len()).sum();
            println!(
                "Indexed {} symbol(s) across {} crate(s) -> {}",
                total,
                symbol_index.crates.len(),
                index_path.display()
            );
        }
        Commands::Search { query, app_dir } => {
            let app_dir = get_app_dir(app_dir)?;
            let index_path = index::get_index_path(&app_dir)?;
            if !index_path.exists() {
                anyhow::bail!(
                    "No symbol index found at {}. Run `specbuild index` first.",
                    index_path.display()
                );
            }
            let symbol_index = index::load_index(&index_path)?;
            let results = index::search_index(&symbol_index, &query);
            index::print_search_results(&symbol_index, &results);
        }
        Commands::Stub {
            package,
            out_dir,
            app_dir,
        } => {
            let app_dir = get_app_dir(app_dir)?;
            let sb_path = app_dir.join(format!("{}{}", package, SB_EXT));
            let out_dir = out_dir.unwrap_or_else(|| app_dir.join(".specbuild-stubs"));
            stub::generate_stub_crate(&sb_path, &out_dir)?;
        }
        Commands::StubAll { out_dir, app_dir } => {
            let app_dir = get_app_dir(app_dir)?;
            let out_dir = out_dir.unwrap_or_else(|| app_dir.join(".specbuild-stubs"));
            let closed = get_top_level_closed_packages(&app_dir)?;
            if closed.is_empty() {
                println!("No closed packages to stub.");
            } else {
                for pkg in &closed {
                    let sb_path = app_dir.join(format!("{}{}", pkg, SB_EXT));
                    stub::generate_stub_crate(&sb_path, &out_dir)?;
                }
            }
        }
        Commands::OpenModule { path, app_dir } => {
            let app_dir = get_app_dir(app_dir)?;
            open_module(&path, &app_dir)?;
        }
        Commands::CloseModule {
            path,
            regenerate,
            app_dir,
        } => {
            let app_dir = get_app_dir(app_dir)?;
            close_module(&path, &app_dir, regenerate)?;
        }
        Commands::AgentGuide { app_dir } => {
            let app_dir = get_app_dir(app_dir)?;
            let guide = include_str!("../AGENTS.md");
            let out_path = app_dir.join("AGENTS.md");
            fs::write(&out_path, guide)
                .with_context(|| format!("Failed to write {}", out_path.display()))?;
            println!("Wrote agent guide to {}", out_path.display());
        }
    }

    Ok(())
}
