use anyhow::{Context, Result};
use rig::completion::{Prompt, ToolDefinition};
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::Once;

static INIT_TRACING: Once = Once::new();

fn ensure_tracing() {
    INIT_TRACING.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .try_init();
    });
}

// ---------------------------------------------------------------------------
// Tool error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ToolError(String);

// ---------------------------------------------------------------------------
// read_file
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ReadFileTool {
    pub app_dir: PathBuf,
}

impl Tool for ReadFileTool {
    const NAME: &'static str = "read_file";
    type Error = ToolError;
    type Args = ReadFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read the contents of a file relative to the application directory".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to the file (e.g. 'auth/src/lib.rs')"
                    }
                },
                "required": ["path"],
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = self.app_dir.join(&args.path);
        fs::read_to_string(&path)
            .map_err(|e| ToolError(format!("Failed to read {}: {}", path.display(), e)))
    }
}

// ---------------------------------------------------------------------------
// write_file
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct WriteFileTool {
    pub app_dir: PathBuf,
}

impl Tool for WriteFileTool {
    const NAME: &'static str = "write_file";
    type Error = ToolError;
    type Args = WriteFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Write content to a file. Creates parent directories if needed.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to the file"
                    },
                    "content": {
                        "type": "string",
                        "description": "Full content to write"
                    }
                },
                "required": ["path", "content"],
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = self.app_dir.join(&args.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                ToolError(format!("Failed to create dirs for {}: {}", path.display(), e))
            })?;
        }
        fs::write(&path, &args.content).map_err(|e| {
            ToolError(format!("Failed to write {}: {}", path.display(), e))
        })?;
        Ok(format!("Wrote {} bytes to {}", args.content.len(), path.display()))
    }
}

// ---------------------------------------------------------------------------
// list_directory
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ListDirectoryArgs {
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ListDirectoryTool {
    pub app_dir: PathBuf,
}

impl Tool for ListDirectoryTool {
    const NAME: &'static str = "list_directory";
    type Error = ToolError;
    type Args = ListDirectoryArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_directory".to_string(),
            description: "List files and directories at a given path".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative path to the directory (use '.' for root)"
                    }
                },
                "required": ["path"],
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = self.app_dir.join(&args.path);
        let mut entries = Vec::new();
        for entry in fs::read_dir(&path)
            .map_err(|e| ToolError(format!("Failed to read dir {}: {}", path.display(), e)))?
        {
            let entry = entry.map_err(|e| ToolError(format!("Dir entry error: {}", e)))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let ty = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                "dir"
            } else {
                "file"
            };
            entries.push(format!("[{}] {}", ty, name));
        }
        entries.sort();
        Ok(entries.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// list_packages
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ListPackagesArgs {}

#[derive(Serialize, Deserialize, Clone)]
pub struct ListPackagesTool {
    pub app_dir: PathBuf,
}

impl Tool for ListPackagesTool {
    const NAME: &'static str = "list_packages";
    type Error = ToolError;
    type Args = ListPackagesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_packages".to_string(),
            description: "List all packages in the workspace with their open/closed status".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let mut packages = Vec::new();

        for entry in fs::read_dir(&self.app_dir)
            .map_err(|e| ToolError(format!("Failed to read app dir: {}", e)))?
            .flatten()
        {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if name_str.ends_with(".sb") {
                let pkg_name = &name_str[..name_str.len() - 3];
                packages.push(format!("{} [CLOSED]", pkg_name));
            } else if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let hidden = entry.path().join(".specbuilt.sb");
                if hidden.exists() {
                    packages.push(format!("{} [OPEN]", name_str));
                }
            }
        }

        packages.sort();
        Ok(packages.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// read_spec
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ReadSpecArgs {
    pub package: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ReadSpecTool {
    pub app_dir: PathBuf,
}

impl Tool for ReadSpecTool {
    const NAME: &'static str = "read_spec";
    type Error = ToolError;
    type Args = ReadSpecArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "read_spec".to_string(),
            description: "Read the .sb spec file for a package".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "Package name"
                    }
                },
                "required": ["package"],
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let sb_path = self.app_dir.join(format!("{}.sb", args.package));
        if sb_path.exists() {
            fs::read_to_string(&sb_path)
                .map_err(|e| ToolError(format!("Failed to read spec: {}", e)))
        } else {
            let hidden = self.app_dir.join(&args.package).join(".specbuilt.sb");
            if hidden.exists() {
                fs::read_to_string(&hidden)
                    .map_err(|e| ToolError(format!("Failed to read hidden spec: {}", e)))
            } else {
                Err(ToolError(format!("No spec found for package '{}'", args.package)))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// run_cargo_test
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RunCargoTestArgs {
    pub package: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RunCargoTestTool {
    pub app_dir: PathBuf,
}

impl Tool for RunCargoTestTool {
    const NAME: &'static str = "run_cargo_test";
    type Error = ToolError;
    type Args = RunCargoTestArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "run_cargo_test".to_string(),
            description: "Run 'cargo test' for a package in a temporary workspace that includes all closed dependencies.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "Package name to test"
                    }
                },
                "required": ["package"],
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let (temp_dir, _) = crate::setup_temp_workspace(&self.app_dir)
            .map_err(|e| ToolError(format!("Failed to setup temp workspace: {}", e)))?;

        let output = std::process::Command::new("cargo")
            .arg("test")
            .arg("-p")
            .arg(&args.package)
            .current_dir(&temp_dir)
            .output()
            .map_err(|e| ToolError(format!("Failed to run cargo test: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let result = format!(
            "Exit code: {:?}\n\nSTDOUT:\n{}\n\nSTDERR:\n{}",
            output.status.code(),
            stdout,
            stderr
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// run_cargo_check
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RunCargoCheckArgs {
    pub package: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RunCargoCheckTool {
    pub app_dir: PathBuf,
}

impl Tool for RunCargoCheckTool {
    const NAME: &'static str = "run_cargo_check";
    type Error = ToolError;
    type Args = RunCargoCheckArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "run_cargo_check".to_string(),
            description: "Run 'cargo check' for a package in a temporary workspace that includes all closed dependencies.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "Package name to check"
                    }
                },
                "required": ["package"],
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let (temp_dir, _) = crate::setup_temp_workspace(&self.app_dir)
            .map_err(|e| ToolError(format!("Failed to setup temp workspace: {}", e)))?;

        let output = std::process::Command::new("cargo")
            .arg("check")
            .arg("-p")
            .arg(&args.package)
            .current_dir(&temp_dir)
            .output()
            .map_err(|e| ToolError(format!("Failed to run cargo check: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let result = format!(
            "Exit code: {:?}\n\nSTDOUT:\n{}\n\nSTDERR:\n{}",
            output.status.code(),
            stdout,
            stderr
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// search_symbol
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SearchSymbolArgs {
    pub query: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SearchSymbolTool {
    pub app_dir: PathBuf,
}

impl Tool for SearchSymbolTool {
    const NAME: &'static str = "search_symbol";
    type Error = ToolError;
    type Args = SearchSymbolArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "search_symbol".to_string(),
            description: "Search the codebase symbol index for definitions. Requires `specbuild index` to have been run.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Symbol name, kind (struct, fn, trait, etc.), or doc text to search for"
                    }
                },
                "required": ["query"],
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let index_path = self.app_dir.join(".specbuilt-index.json");
        if !index_path.exists() {
            let sb_dir = crate::discover_sb_dir(&self.app_dir)
                .map_err(|e| ToolError(format!("Cannot find index: {}", e)))?;
            let alt_path = sb_dir.join("specbuilt-index.json");
            if !alt_path.exists() {
                return Err(ToolError(
                    "No symbol index found. Run `specbuild index` first.".to_string(),
                ));
            }
            let index = crate::index::load_index(&alt_path)
                .map_err(|e| ToolError(format!("Failed to load index: {}", e)))?;
            let results = crate::index::search_index(&index, &args.query);
            return Ok(format_search_results(&results));
        }

        let index = crate::index::load_index(&index_path)
            .map_err(|e| ToolError(format!("Failed to load index: {}", e)))?;
        let results = crate::index::search_index(&index, &args.query);
        Ok(format_search_results(&results))
    }
}

fn format_search_results(results: &[&crate::index::Symbol]) -> String {
    if results.is_empty() {
        return "No symbols found.".to_string();
    }
    let mut lines = vec![format!("Found {} symbol(s):", results.len())];
    for sym in results.iter().take(20) {
        lines.push(format!(
            "  [{}] {} ({}:{}){}",
            sym.kind,
            sym.name,
            sym.file,
            sym.line,
            if sym.docs.is_empty() {
                String::new()
            } else {
                format!(" -- {}", sym.docs)
            }
        ));
    }
    if results.len() > 20 {
        lines.push(format!("  ... and {} more", results.len() - 20));
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// run_shell_command
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RunShellArgs {
    pub command: String,
    pub cwd: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RunShellTool {
    pub app_dir: PathBuf,
}

impl Tool for RunShellTool {
    const NAME: &'static str = "run_shell_command";
    type Error = ToolError;
    type Args = RunShellArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "run_shell_command".to_string(),
            description: "Run a shell command in the workspace. Use for cargo test, cargo check, grep, find, etc.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to run"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional relative working directory (defaults to app root)"
                    }
                },
                "required": ["command"],
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let cwd = args
            .cwd
            .map(|p| self.app_dir.join(p))
            .unwrap_or_else(|| self.app_dir.clone());
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&args.command)
            .current_dir(&cwd)
            .output()
            .map_err(|e| ToolError(format!("Failed to run command: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let result = format!(
            "Exit code: {:?}\n\nSTDOUT:\n{}\n\nSTDERR:\n{}",
            output.status.code(),
            stdout,
            stderr
        );

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Agent setup
// ---------------------------------------------------------------------------

fn build_preamble(action: &str, package: &str) -> String {
    format!(
        r#"You are an expert software engineer working within the Specbuilt context-window management system.

You are currently performing a `{action}` operation on package `{package}`.

The codebase is organized into packages. Each package is either:
- CLOSED: represented by a `.sb` spec file (a lightweight contract with interface, dependencies, and behavioral spec)
- OPEN: full source code is available for editing

Packages may be written in Rust or TypeScript (indicated by `[package].language` in the .sb spec).

Your task is to modify the opened package according to the user's instructions.

Available tools:
- read_file: Read any file in the workspace
- write_file: Write content to a file (creates parent dirs if needed)
- list_directory: List files in a directory
- list_packages: List all packages and their open/closed status
- read_spec: Read the .sb spec for a package
- run_shell_command: Run a shell command (for cargo test, cargo check, npm test, tsc, grep, etc.)

Workflow:
1. Read the package spec to understand the contract and language
2. Explore existing source files
3. Make the necessary changes
4. For Rust: run `cargo test` or `cargo check` to verify
   For TypeScript: run `npm test`, `tsc --noEmit`, or the command in `[test].command`
5. Iterate until everything passes

Rules:
- Only modify files within the opened package unless explicitly asked
- Follow the idiomatic style of the package language (Rust or TypeScript)
- Maintain compatibility with the package's public API as described in the spec
- Do not break existing tests unless the spec explicitly requires a behavior change"#
    )
}

fn build_task_prompt(action: &str, package: &str, task: Option<&str>) -> String {
    let mut prompt = format!("Perform a `{action}` on package `{package}`.");
    if let Some(t) = task {
        prompt.push_str(&format!("\n\nTask: {t}"));
    }
    prompt.push_str("\n\nStart by reading the spec and exploring the source files.");
    prompt
}

async fn run_agent_async(
    action: &str,
    package: &str,
    app_dir: &std::path::Path,
    task_prompt: Option<&str>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<bool> {
    ensure_tracing();

    let provider = provider
        .or_else(|| std::env::var("SPECBUILD_AI_PROVIDER").ok())
        .unwrap_or_else(|| "openai".to_string());

    let model = model.or_else(|| std::env::var("SPECBUILD_AI_MODEL").ok());

    let app_dir = app_dir.to_path_buf();
    let preamble = build_preamble(action, package);
    let task = build_task_prompt(action, package, task_prompt);

    // Build tools
    let read_file_tool = ReadFileTool {
        app_dir: app_dir.clone(),
    };
    let write_file_tool = WriteFileTool {
        app_dir: app_dir.clone(),
    };
    let list_dir_tool = ListDirectoryTool {
        app_dir: app_dir.clone(),
    };
    let list_packages_tool = ListPackagesTool {
        app_dir: app_dir.clone(),
    };
    let read_spec_tool = ReadSpecTool {
        app_dir: app_dir.clone(),
    };
    let search_symbol_tool = SearchSymbolTool {
        app_dir: app_dir.clone(),
    };
    let run_cargo_test_tool = RunCargoTestTool {
        app_dir: app_dir.clone(),
    };
    let run_cargo_check_tool = RunCargoCheckTool {
        app_dir: app_dir.clone(),
    };
    let run_shell_tool = RunShellTool { app_dir };

    match provider.as_str() {
        "openai" => {
            let client = if let Ok(base_url) = std::env::var("OPENAI_BASE_URL") {
                let api_key = std::env::var("OPENAI_API_KEY")
                    .expect("OPENAI_API_KEY must be set when using OPENAI_BASE_URL");
                rig::providers::openai::Client::from_url(&api_key, &base_url)
            } else {
                rig::providers::openai::Client::from_env()
            };
            let model = model.as_deref().unwrap_or("gpt-4o");
            let agent = client
                .agent(model)
                .preamble(&preamble)
                .max_tokens(4096)
                .tool(read_file_tool)
                .tool(write_file_tool)
                .tool(list_dir_tool)
                .tool(list_packages_tool)
                .tool(read_spec_tool)
                .tool(search_symbol_tool.clone())
                .tool(run_cargo_test_tool.clone())
                .tool(run_cargo_check_tool.clone())
                .tool(run_shell_tool)
                .build();

            match agent.prompt(task.as_str()).await {
                Ok(response) => {
                    println!("  [agent] Response: {}", response);
                    Ok(true)
                }
                Err(e) => {
                    println!("  [agent] Error: {}", e);
                    Ok(false)
                }
            }
        }
        "anthropic" => {
            let client = rig::providers::anthropic::Client::from_env();
            let model = model.as_deref().unwrap_or("claude-3-5-sonnet-20240620");
            let agent = client
                .agent(model)
                .preamble(&preamble)
                .max_tokens(4096)
                .tool(read_file_tool)
                .tool(write_file_tool)
                .tool(list_dir_tool)
                .tool(list_packages_tool)
                .tool(read_spec_tool)
                .tool(search_symbol_tool)
                .tool(run_cargo_test_tool)
                .tool(run_cargo_check_tool)
                .tool(run_shell_tool)
                .build();

            match agent.prompt(task.as_str()).await {
                Ok(response) => {
                    println!("  [agent] Response: {}", response);
                    Ok(true)
                }
                Err(e) => {
                    println!("  [agent] Error: {}", e);
                    Ok(false)
                }
            }
        }
        "kimi" => {
            let client = if let Ok(base_url) = std::env::var("KIMI_BASE_URL") {
                let api_key = std::env::var("KIMI_API_KEY")
                    .expect("KIMI_API_KEY must be set when using KIMI_BASE_URL");
                rig::providers::openai::Client::from_url(&api_key, &base_url)
            } else {
                let api_key = std::env::var("KIMI_API_KEY")
                    .expect("KIMI_API_KEY must be set for kimi provider");
                rig::providers::openai::Client::from_url(&api_key, "https://api.moonshot.cn/v1")
            };
            let model = model.as_deref().unwrap_or("kimi-latest");
            let agent = client
                .agent(model)
                .preamble(&preamble)
                .max_tokens(4096)
                .tool(read_file_tool)
                .tool(write_file_tool)
                .tool(list_dir_tool)
                .tool(list_packages_tool)
                .tool(read_spec_tool)
                .tool(search_symbol_tool.clone())
                .tool(run_cargo_test_tool.clone())
                .tool(run_cargo_check_tool.clone())
                .tool(run_shell_tool)
                .build();

            match agent.prompt(task.as_str()).await {
                Ok(response) => {
                    println!("  [agent] Response: {}", response);
                    Ok(true)
                }
                Err(e) => {
                    println!("  [agent] Error: {}", e);
                    Ok(false)
                }
            }
        }
        _ => {
            anyhow::bail!(
                "Unknown AI provider: {}. Supported: openai, anthropic, kimi",
                provider
            )
        }
    }
}

/// Run the Rig-based AI agent for a fix or doover task.
pub fn run_agent(
    action: &str,
    package: &str,
    app_dir: &std::path::Path,
    task_prompt: Option<&str>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<bool> {
    let rt = tokio::runtime::Runtime::new().context("Failed to create tokio runtime")?;
    rt.block_on(async {
        run_agent_async(action, package, app_dir, task_prompt, provider, model).await
    })
}
