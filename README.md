# Specbuilt

Context window management for AI agents working with Rust codebases.

## Concept

When an AI agent works on a large Rust project, it wastes tokens reading implementation details of modules it isn't modifying. **Specbuilt** solves this by replacing full source packages with lightweight `.sb` spec files.

- **Closed**: Only the `.sb` spec is visible (name, version, API surface, dependencies, behavioral spec)
- **Open**: Full source + tests are materialized for editing

## Directory Layout

```
application-sb/
├── application-rs/        # Agent workspace
│   ├── main.rs
│   ├── Cargo.toml
│   ├── auth.sb            # Closed: spec + extracted API surface
│   ├── db.sb
│   └── api.sb
├── specbuilt-source/      # Canonical source storage
│   ├── auth/              # Full Rust crate
│   ├── db/
│   └── api/
└── specbuild              # CLI tool
```

## Installation

```bash
cd Specbuilt
cargo build --release
```

The binary is built to `target/release/specbuild`. Copy or symlink it to your `$PATH` if desired.

## Commands

| Command | Description |
|---------|-------------|
| `specbuild base` | Auto-generate `.sb` files with extracted API surfaces from sibling `<name>-sb/specbuilt-source` |
| `specbuild list` | Show all packages and their open/closed/check status |
| `specbuild open <pkg>` | Replace `.sb` with full source (auto-opens dependencies) |
| `specbuild close <pkg>` | Sync source back to `specbuilt-source` and restore `.sb` |
| `specbuild close --regenerate <pkg>` | Close and regenerate the spec from updated source |
| `specbuild open-all` | Open all closed packages |
| `specbuild close-all` | Close all opened packages |
| `specbuild build` | Build all packages in a temporary workspace |
| `specbuild check [pkg]` | Run spec validation + `cargo test` + optional verify command (caches results) |
| `specbuild fix <pkg>` | Update spec, open, run AI agent, check with auto-repair, close |
| `specbuild doover <pkg>` | Back up, clear `.rs` files, rebuild from spec via AI agent, check with auto-repair |
| `specbuild index` | Build a symbol index from all crates in `specbuilt-source` |
| `specbuild search <query>` | Search the symbol index for definitions and usages |
| `specbuild stub <pkg>` | Generate a compilable stub crate from a closed `.sb` spec |
| `specbuild stub-all` | Generate stubs for all closed packages at once |
| `specbuild agent-guide` | Write the Specbuilt agent guide (`AGENTS.md`) into the application-rs directory |

### Global Options

Most commands accept:
- `--app-dir <path>` — Path to `application-rs` (defaults to `./application-rs` or current dir if it ends in `-rs`)

### Check-specific Options

- `--force` — Re-run checks even if spec and source haven't changed

### Fix / Doover Options

- `--prompt <text>` — Description of what to fix or rebuild
- `--retries <n>` — Max check retry attempts after the agent runs (default: 3)
- `--no-close` — Leave the package open after fixing / rebuilding
- `--agent <cmd>` — External AI agent command (falls back to `SPECBUILD_AI_AGENT` env var)
- `--provider <name>` — LLM provider for built-in agent: `openai` or `anthropic` (falls back to `SPECBUILD_AI_PROVIDER`)
- `--model <name>` — LLM model name (falls back to `SPECBUILD_AI_MODEL`)

## `.sb` File Format

```toml
[package]
name = "auth"
version = "0.1.0"
description = "Authentication module"

[interface]
inputs = ["db::ConnectionPool"]
outputs = ["auth::Authenticator", "auth::login"]
dependencies = ["db"]
api_surface = """
// Structs
/// A user in the system.
pub struct User { pub id : u64 , pub name : String , }

// Enums
/// Errors that can occur.
pub enum MyError { NotFound , InvalidInput (String) , }

// Traits
/// Validates user credentials.
 pub trait Validator {
/// Check if credentials are valid.
    fn validate (& self , creds : & str) -> Result < bool , MyError >;
}

// Functions
/// Login a user.
pub fn login (creds : & str) -> Result < User , MyError >;
"""

[source]
path = "../specbuilt-source/auth"

[test]
command = "cargo test -p auth"

[spec]
description = "Handles login, logout, and session validation."
invariants = [
    "Passwords must be hashed with argon2",
    "Session tokens must be UUID v4"
]
verify_command = "cargo clippy -p auth -- -D warnings"
```

### Sections

| Section | Purpose |
|---------|---------|
| `[package]` | Name, version, and human-readable description |
| `[interface]` | Inputs, outputs, internal dependencies, and **auto-extracted public API surface** |
| `[source]` | Relative path back to the canonical source directory |
| `[test]` | Default test command for this package |
| `[spec]` | **Behavioral specification** — invariants, description, and an optional `verify_command` |

## AI Agent Integration

Specbuilt includes a built-in AI agent powered by [rig.rs](https://rig.rs). The agent receives a curated view of the codebase through `.sb` spec files and uses tools to interact with opened packages:

| Tool | Purpose |
|------|---------|
| `read_file` | Read any file in the workspace |
| `write_file` | Write content to a file |
| `list_directory` | List files in a directory |
| `list_packages` | List all packages and their open/closed status |
| `read_spec` | Read the `.sb` spec for a package (includes API surface) |
| `run_cargo_test` | Run `cargo test` for a package in a temporary workspace |
| `run_cargo_check` | Run `cargo check` for a package in a temporary workspace |
| `search_symbol` | Search the symbol index for definitions and usages |
| `run_shell_command` | Run shell commands (`cargo test`, `cargo check`, `grep`, etc.) |

### Auto-Repair Loop

During `fix` and `doover`, if checks fail, the failure output (including `cargo test` stdout/stderr) is automatically fed back to the agent as a new prompt. The agent diagnoses the issue, edits the source, and the check loop retries — up to the configured `--retries` limit.

### Configuration

Set one of the following environment variables:

| Variable | Purpose |
|----------|---------|
| `OPENAI_API_KEY` | Use OpenAI as the default provider |
| `ANTHROPIC_API_KEY` | Use Anthropic as the default provider |
| `SPECBUILD_AI_PROVIDER` | Explicit provider selection (`openai`, `anthropic`, or `kimi`) |
| `SPECBUILD_AI_MODEL` | Model name override (e.g., `gpt-4o`, `claude-3-5-sonnet-20240620`) |
| `SPECBUILD_AI_AGENT` | External agent command (fallback if no provider is configured) |
| `SPECBUILD_AI_CHECKER` | External checker command invoked during `check` |

## Check Results

`specbuild check` evaluates:

1. **Spec validity** — Are invariants non-empty? Is the spec well-formed?
2. **`cargo test`** — Does the package compile and pass its tests?
3. **`verify_command`** — If defined in `[spec]`, is the custom command successful?
4. **AI checker** — If `SPECBUILD_AI_CHECKER` is set, run the external checker.

Results are cached in `specbuilt-checks.json`. Re-checks are skipped automatically when nothing has changed.

| Result | Meaning |
|--------|---------|
| `PASSED` | All checks succeeded |
| `FAILED` | Tests or verify command failed |
| `BUGGED` | Spec is invalid or incomplete |

## Why This Works

- **Agents see architecture first**: `.sb` files describe the contract without the noise.
- **Agents see API surfaces**: The `api_surface` field gives the agent full type signatures for closed crates without the implementation bloat.
- **Focused editing**: Only opened packages consume context tokens.
- **Self-healing**: The check feedback loop lets the agent fix its own mistakes.
- **Tests travel with source**: Each package is self-contained and testable.
- **Safe sync**: `close` writes changes back to `specbuilt-source/` and can regenerate specs automatically.
- **Verifiable specs**: The `[spec]` section lets you encode behavioral constraints that get checked automatically.

## Build Order / Roadmap

Features are prioritized by what unlocks the next level of scale. Completed items are checked.

### Phase 0 — Foundation ✅
- [x] **rig.rs agent integration** — Built-in AI agent with 9 tools (read/write files, run cargo, search symbols, shell commands)
- [x] **Structured API extraction** — Auto-extract public API surfaces from Rust source into `.sb` files using `syn`
- [x] **Auto-repair loop** — Failed checks feed error output back to the agent for self-correction
- [x] **Symbol index** — `specbuild index` / `search` with definition + usage tracking across crates
- [x] **Stub generation** — `specbuild stub` generates compilable stub crates from closed specs
- [x] **Smart dependency inference** — `use` statements scanned to populate `dependencies` automatically

### Phase 1 — Granularity ⬅️ Next
- [ ] **Module-level specs** — Close submodules *within* an opened crate. A 20k LOC crate shouldn't have to be fully open.
  - Recursive `.sb` discovery inside opened crates
  - `open-module` / `close-module` commands
  - Automatic stub injection for closed submodules so parent crate still compiles

### Phase 2 — Intelligence
- [ ] **Reference tracking** — Full call-graph analysis: "where is `auth::login` called?" across all closed crates
- [ ] **Persistent agent mode** — `specbuild agent` keeps an agent session alive with memory across turns
- [ ] **Spec diff** — `specbuild diff <pkg>` shows drift between current spec and regenerated spec