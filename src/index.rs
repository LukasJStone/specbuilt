use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use syn::spanned::Spanned;
use walkdir::WalkDir;

const INDEX_FILE: &str = "specbuilt-index.json";

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SymbolIndex {
    /// crate name -> list of symbols
    pub crates: HashMap<String, Vec<Symbol>>,
    /// symbol name -> list of locations that import/use it
    pub usages: HashMap<String, Vec<Usage>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub crate_name: String,
    pub file: String,
    pub line: usize,
    pub kind: String, // "use", "pub_use", "extern_crate"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub docs: String,
}

/// Build a symbol index from all crates in specbuilt-source.
pub fn build_index(source_dir: &Path) -> Result<SymbolIndex> {
    let mut index = SymbolIndex::default();

    for entry in fs::read_dir(source_dir)
        .with_context(|| format!("Failed to read source dir: {}", source_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let crate_dir = entry.path();
        let crate_name = entry.file_name().to_string_lossy().to_string();

        let cargo_toml = crate_dir.join("Cargo.toml");
        if !cargo_toml.exists() {
            continue;
        }

        println!("  [index] Indexing '{}'...", crate_name);
        let (symbols, usages) = index_crate(&crate_dir, &crate_name)?;
        if !symbols.is_empty() {
            index.crates.insert(crate_name, symbols);
        }
        for (sym_name, usage_list) in usages {
            index.usages.entry(sym_name).or_default().extend(usage_list);
        }
    }

    Ok(index)
}

fn index_crate(crate_dir: &Path, crate_name: &str) -> Result<(Vec<Symbol>, HashMap<String, Vec<Usage>>)> {
    let mut symbols = Vec::new();
    let mut usages: HashMap<String, Vec<Usage>> = HashMap::new();

    for entry in WalkDir::new(crate_dir.join("src"))
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"))
    {
        let path = entry.path();
        let rel_path = path.strip_prefix(crate_dir).unwrap_or(path);
        let rel_str = rel_path.to_string_lossy().to_string();

        match index_file(path, crate_name, &rel_str) {
            Ok((mut syms, us)) => {
                symbols.append(&mut syms);
                for (k, v) in us {
                    usages.entry(k).or_default().extend(v);
                }
            }
            Err(e) => {
                eprintln!("  [index] Warning: failed to index {}: {}", path.display(), e);
            }
        }
    }

    Ok((symbols, usages))
}

fn index_file(path: &Path, crate_name: &str, rel_path: &str) -> Result<(Vec<Symbol>, HashMap<String, Vec<Usage>>)> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let file = syn::parse_file(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;

    let mut symbols = Vec::new();
    let mut usages: HashMap<String, Vec<Usage>> = HashMap::new();

    for item in &file.items {
        extract_symbol(item, crate_name, rel_path, &mut symbols);
        extract_usages(item, crate_name, rel_path, &mut usages);
    }
    Ok((symbols, usages))
}

fn extract_symbol(
    item: &syn::Item,
    crate_name: &str,
    rel_path: &str,
    symbols: &mut Vec<Symbol>,
) {
    let (name, kind, span, attrs) = match item {
        syn::Item::Fn(f) => {
            if !is_public(&f.vis) {
                return;
            }
            (
                format!("{}::{}", crate_name, f.sig.ident),
                "fn",
                f.sig.ident.span(),
                &f.attrs,
            )
        }
        syn::Item::Struct(s) => {
            if !is_public(&s.vis) {
                return;
            }
            (
                format!("{}::{}", crate_name, s.ident),
                "struct",
                s.ident.span(),
                &s.attrs,
            )
        }
        syn::Item::Enum(e) => {
            if !is_public(&e.vis) {
                return;
            }
            (
                format!("{}::{}", crate_name, e.ident),
                "enum",
                e.ident.span(),
                &e.attrs,
            )
        }
        syn::Item::Trait(t) => {
            if !is_public(&t.vis) {
                return;
            }
            (
                format!("{}::{}", crate_name, t.ident),
                "trait",
                t.ident.span(),
                &t.attrs,
            )
        }
        syn::Item::Type(t) => {
            if !is_public(&t.vis) {
                return;
            }
            (
                format!("{}::{}", crate_name, t.ident),
                "type",
                t.ident.span(),
                &t.attrs,
            )
        }
        syn::Item::Const(c) => {
            if !is_public(&c.vis) {
                return;
            }
            (
                format!("{}::{}", crate_name, c.ident),
                "const",
                c.ident.span(),
                &c.attrs,
            )
        }
        syn::Item::Static(s) => {
            if !is_public(&s.vis) {
                return;
            }
            (
                format!("{}::{}", crate_name, s.ident),
                "static",
                s.ident.span(),
                &s.attrs,
            )
        }
        syn::Item::Mod(m) => {
            if !is_public(&m.vis) {
                return;
            }
            (
                format!("{}::{}", crate_name, m.ident),
                "mod",
                m.ident.span(),
                &m.attrs,
            )
        }
        syn::Item::Macro(m) => {
            let macro_name = m
                .mac
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            if macro_name != "macro_rules" {
                return;
            }
            if let Some(ident) = &m.ident {
                (
                    format!("{}::{}", crate_name, ident),
                    "macro",
                    ident.span(),
                    &m.attrs,
                )
            } else {
                return;
            }
        }
        _ => return,
    };

    let line = span.start().line;
    let docs = extract_docs(attrs);

    symbols.push(Symbol {
        name,
        kind: kind.to_string(),
        file: rel_path.to_string(),
        line,
        docs,
    });
}

fn extract_usages(
    item: &syn::Item,
    crate_name: &str,
    rel_path: &str,
    usages: &mut HashMap<String, Vec<Usage>>,
) {
    let item_use = match item {
        syn::Item::Use(u) => u,
        _ => return,
    };

    let line = item_use.span().start().line;
    let kind = if is_public(&item_use.vis) {
        "pub_use"
    } else {
        "use"
    };

    fn walk_tree(
        tree: &syn::UseTree,
        prefix: &str,
        crate_name: &str,
        rel_path: &str,
        line: usize,
        kind: &str,
        usages: &mut HashMap<String, Vec<Usage>>,
    ) {
        match tree {
            syn::UseTree::Path(p) => {
                let new_prefix = if prefix.is_empty() {
                    p.ident.to_string()
                } else {
                    format!("{}::{}", prefix, p.ident)
                };
                walk_tree(&p.tree, &new_prefix, crate_name, rel_path, line, kind, usages);
            }
            syn::UseTree::Name(n) => {
                let name = if prefix.is_empty() {
                    n.ident.to_string()
                } else {
                    format!("{}::{}", prefix, n.ident)
                };
                usages.entry(name).or_default().push(Usage {
                    crate_name: crate_name.to_string(),
                    file: rel_path.to_string(),
                    line,
                    kind: kind.to_string(),
                });
            }
            syn::UseTree::Rename(r) => {
                let name = if prefix.is_empty() {
                    r.ident.to_string()
                } else {
                    format!("{}::{}", prefix, r.ident)
                };
                usages.entry(name).or_default().push(Usage {
                    crate_name: crate_name.to_string(),
                    file: rel_path.to_string(),
                    line,
                    kind: kind.to_string(),
                });
            }
            syn::UseTree::Glob(_) => {
                // Wildcard imports are hard to resolve precisely; skip
            }
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    walk_tree(item, prefix, crate_name, rel_path, line, kind, usages);
                }
            }
        }
    }

    walk_tree(
        &item_use.tree,
        "",
        crate_name,
        rel_path,
        line,
        kind,
        usages,
    );
}

fn is_public(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn extract_docs(attrs: &[syn::Attribute]) -> String {
    let mut docs = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(nv) = &attr.meta {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = &nv.value
                {
                    docs.push(s.value().trim().to_string());
                }
            }
        }
    }
    docs.join(" ")
}

pub fn get_index_path(app_dir: &Path) -> Result<PathBuf> {
    let sb_dir = crate::discover_sb_dir(app_dir)?;
    Ok(sb_dir.join(INDEX_FILE))
}

pub fn save_index(index: &SymbolIndex, path: &Path) -> Result<()> {
    let content = serde_json::to_string_pretty(index)
        .context("Failed to serialize symbol index")?;
    fs::write(path, content)
        .with_context(|| format!("Failed to write index to {}", path.display()))?;
    Ok(())
}

pub fn load_index(path: &Path) -> Result<SymbolIndex> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read index: {}", path.display()))?;
    let index: SymbolIndex = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse index: {}", path.display()))?;
    Ok(index)
}

pub fn search_index<'a>(index: &'a SymbolIndex, query: &str) -> Vec<&'a Symbol> {
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();
    for symbols in index.crates.values() {
        for sym in symbols {
            if sym.name.to_lowercase().contains(&query_lower)
                || sym.docs.to_lowercase().contains(&query_lower)
                || sym.kind.to_lowercase() == query_lower
            {
                results.push(sym);
            }
        }
    }
    // Sort by relevance: exact name match first, then partial, then docs
    results.sort_by(|a, b| {
        let a_name_exact = a.name.to_lowercase() == query_lower;
        let b_name_exact = b.name.to_lowercase() == query_lower;
        let a_name_partial = a.name.to_lowercase().contains(&query_lower);
        let b_name_partial = b.name.to_lowercase().contains(&query_lower);

        b_name_exact.cmp(&a_name_exact)
            .then(b_name_partial.cmp(&a_name_partial))
            .then_with(|| a.name.len().cmp(&b.name.len()))
    });
    results
}

pub fn print_search_results(index: &SymbolIndex, results: &[&Symbol]) {
    if results.is_empty() {
        println!("No symbols found.");
        return;
    }
    println!("Found {} symbol(s):", results.len());
    println!(
        "{:<30} {:<10} {:<30} {}",
        "NAME", "KIND", "FILE", "LINE"
    );
    println!("{}", "-".repeat(80));
    for sym in results.iter().take(50) {
        println!(
            "{:<30} {:<10} {:<30} {}",
            sym.name, sym.kind, sym.file, sym.line
        );
        if let Some(usages) = index.usages.get(&sym.name) {
            let unique_crates: std::collections::BTreeSet<_> =
                usages.iter().map(|u| u.crate_name.as_str()).collect();
            if !unique_crates.is_empty() {
                println!(
                    "  -> used in: {}",
                    unique_crates.into_iter().collect::<Vec<_>>().join(", ")
                );
            }
        }
    }
    if results.len() > 50 {
        println!("... and {} more", results.len() - 50);
    }
}
