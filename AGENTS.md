# Specbuilt — Agent Guide

## What This Project Is

Specbuilt is a context-window management CLI tool (`specbuild`) for AI agents working on large Rust codebases. It lets an agent see **lightweight `.sb` spec files** instead of full crate source, then **open** a crate when it needs to edit it.

## Directory Layout (You MUST Respect)

```
myproject-sb/                 ← You DO NOT operate here
├── myproject-rs/             ← YOUR WORKSPACE (where you run commands)
│   ├── Cargo.toml
│   ├── main.rs
│   ├── auth.sb               ← Closed: lightweight contract + API surface
│   ├── db.sb                 ← Closed: you see the spec, not the source
│   └── api/                  ← Open: full source materialized for editing
│       ├── Cargo.toml
│       ├── src/
│       └── .specbuilt.sb     ← Hidden spec for the opened package
└── specbuilt-source/         ← You DO NOT operate here
    ├── auth/                 ← Canonical full source
    └── db/
```

**Rule: Never read, write, or list files outside `myproject-rs/`. The `-sb` folder and `specbuilt-source/` are invisible to you.**

## `.sb` File Format

A closed package is a single TOML file named `<package>.sb`:

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
pub struct User { pub id: u64, pub name: String }
pub enum AuthError { NotFound, InvalidInput(String) }
pub fn login(creds: &str) -> Result<User, AuthError>;
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

| Section | What it tells you |
|---------|-------------------|
| `[package]` | Name, version, human description |
| `[interface]` | What it consumes/produces, internal deps, and the **auto-extracted public API surface** (types, traits, fns) |
| `[source]` | Where the canonical source lives (outside your workspace — do not touch) |
| `[test]` | How to test this package |
| `[spec]` | **Behavioral contract** — invariants, description, optional extra verify command |

## Core Commands You Can Run

All commands run from inside the `-rs` directory.

| Command | When to use it |
|---------|----------------|
| `specbuild list` | See every package and whether it's open or closed |
| `specbuild open <pkg>` | Materialize a closed `.sb` into an editable source directory (auto-opens dependencies) |
| `specbuild close <pkg>` | Sync source back to `specbuilt-source` and restore the `.sb` file |
| `specbuild close --regenerate <pkg>` | Close **and** regenerate the spec's `api_surface` from updated source |
| `specbuild build` | Build all packages in a temporary workspace (compiles even with closed packages) |
| `specbuild check [pkg]` | Run spec validation + `cargo test` + `verify_command` (cached; use `--force` to re-run) |
| `specbuild stub <pkg>` | Generate a compilable stub crate from a closed `.sb` spec |
| `specbuild index` | Build a symbol index from all crates (enables `search_symbol` tool) |
| `specbuild search <query>` | Search the symbol index for definitions/usages |

### Module-level granularity (Phase 1)

Inside an **opened** crate you can close individual submodules:

| Command | When to use it |
|---------|----------------|
| `specbuild open-module crate::submodule` | Replace a `.sb` inside an opened crate with full source |
| `specbuild close-module crate::submodule` | Archive a submodule back to `.sb` + auto-generated stub so the parent still compiles |

## Your Workflow

1. **Read specs first** — closed `.sb` files tell you the architecture without token bloat.
2. **Open what you need to edit** — `specbuild open <pkg>` materializes source.
3. **Edit `.rs` files directly** in the opened package.
4. **Check your work** — `specbuild check <pkg>` runs tests + spec validation.
5. **Close when done** — `specbuild close <pkg>` syncs changes back and restores the `.sb`.

### Rule of Thumb: `.rs` vs `.sb`

- **Write `.rs`** when you are actively implementing, refactoring, or debugging a package.
- **Offload to `.sb`** when a module/package is **isolatable enough** that its internal implementation is noise — preserve its contract (public API + behavioral spec) and close it.

A good candidate for `.sb`:
- Stable API surface
- Clear behavioral invariants you can write in `[spec]`
- Not the package you're currently editing
- Consumed by other packages only through its public interface

## What Happens During `check`

1. Spec validity — invariants non-empty? Well-formed?
2. `cargo test` — does it compile and pass?
3. `verify_command` — runs if defined in `[spec]`
4. External AI checker — runs if `SPECBUILD_AI_CHECKER` env var is set

Results are cached in `specbuilt-checks.json`. Re-checks are skipped automatically when nothing has changed.

## Environment Variables (Optional)

| Variable | Purpose |
|----------|---------|
| `OPENAI_API_KEY` | Use OpenAI for built-in agent (`fix` / `doover`) |
| `ANTHROPIC_API_KEY` | Use Anthropic for built-in agent |
| `KIMI_API_KEY` | Use Kimi (Moonshot AI) for built-in agent |
| `SPECBUILD_AI_PROVIDER` | Explicit provider: `openai`, `anthropic`, or `kimi` |
| `SPECBUILD_AI_MODEL` | Model override (e.g., `kimi-latest`) |
| `SPECBUILD_AI_AGENT` | External agent command fallback |
| `SPECBUILD_AI_CHECKER` | External checker command invoked during `check` |

## AI Agent Tools (Built-in)

If you are invoked via `specbuild fix` or `specbuild doover`, you receive these tools:

- `read_file` / `write_file` — edit source in the opened package
- `list_directory` — explore
- `list_packages` — see what's open/closed
- `read_spec` — read a `.sb` contract (even for closed packages)
- `run_cargo_test` / `run_cargo_check` — verify in a temp workspace
- `search_symbol` — query the symbol index
- `run_shell_command` — general shell access

## Important Constraints

- **Do not** manually edit `.sb` files unless you are refining a behavioral spec.
- **Do not** touch files outside the `-rs` directory.
- **Always close** packages after editing so the workspace stays clean for the next agent.
- If you change a public API, run `specbuild close --regenerate <pkg>` so the `.sb` `api_surface` stays accurate.
