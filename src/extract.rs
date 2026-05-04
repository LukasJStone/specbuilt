use anyhow::{Context, Result};
use quote::ToTokens;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct ApiSurface {
    pub docs: Vec<String>,
    pub reexports: Vec<String>,
    pub modules: Vec<String>,
    pub structs: Vec<String>,
    pub enums: Vec<String>,
    pub traits: Vec<String>,
    pub types: Vec<String>,
    pub impls: Vec<String>,
    pub functions: Vec<String>,
    pub consts: Vec<String>,
    pub statics: Vec<String>,
}

fn format_attrs(attrs: &[syn::Attribute]) -> String {
    let mut docs = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(nv) = &attr.meta {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = &nv.value
                {
                    docs.push(format!("///{}", s.value()));
                }
            }
        }
    }
    if docs.is_empty() {
        String::new()
    } else {
        docs.join("\n") + "\n"
    }
}

macro_rules! format_item {
    ($item:expr) => {{
        let mut clean = $item.clone();
        clean.attrs.retain(|a| !a.path().is_ident("doc"));
        format!("{}{}", format_attrs(&$item.attrs), clean.to_token_stream().to_string())
    }};
}

/// Extract the public API surface of a Rust crate as a human-readable string.
pub fn extract_crate_api(crate_dir: &Path) -> Result<String> {
    let lib_rs = crate_dir.join("src/lib.rs");
    let main_rs = crate_dir.join("src/main.rs");

    let entry = if lib_rs.exists() {
        lib_rs
    } else if main_rs.exists() {
        main_rs
    } else {
        anyhow::bail!("No src/lib.rs or src/main.rs in {}", crate_dir.display());
    };

    let mut surface = ApiSurface::default();
    let mut visited = BTreeSet::new();
    extract_file_api(&entry, &mut surface, &mut visited)?;

    Ok(format_surface(&surface))
}

/// Extract the public API surface of a single module file (no recursion into child modules).
pub fn extract_module_api(module_path: &Path) -> Result<String> {
    let content = fs::read_to_string(module_path)
        .with_context(|| format!("Failed to read {}", module_path.display()))?;

    let file = match syn::parse_file(&content) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("  [extract] Warning: failed to parse {}: {}", module_path.display(), e);
            return Ok(String::new());
        }
    };

    let mut surface = ApiSurface::default();

    for item in &file.items {
        extract_module_item(item, &mut surface)?;
    }

    Ok(format_surface(&surface))
}

fn extract_module_item(item: &syn::Item, surface: &mut ApiSurface) -> Result<()> {
    match item {
        syn::Item::Fn(item_fn) => {
            if is_public(&item_fn.vis) {
                surface.functions.push(format_fn(item_fn));
            }
        }
        syn::Item::Struct(item_struct) => {
            if is_public(&item_struct.vis) {
                surface.structs.push(format_item!(item_struct));
            }
        }
        syn::Item::Enum(item_enum) => {
            if is_public(&item_enum.vis) {
                surface.enums.push(format_item!(item_enum));
            }
        }
        syn::Item::Trait(item_trait) => {
            if is_public(&item_trait.vis) {
                surface.traits.push(format_trait(item_trait));
            }
        }
        syn::Item::Type(item_type) => {
            if is_public(&item_type.vis) {
                surface.types.push(format_item!(item_type));
            }
        }
        syn::Item::Const(item_const) => {
            if is_public(&item_const.vis) {
                surface.consts.push(format_item!(item_const));
            }
        }
        syn::Item::Static(item_static) => {
            if is_public(&item_static.vis) {
                surface.statics.push(format_item!(item_static));
            }
        }
        syn::Item::Mod(item_mod) => {
            if is_public(&item_mod.vis) {
                surface.modules.push(item_mod.ident.to_string());
                // Do NOT recurse into module contents — this is module-level extraction
            }
        }
        syn::Item::Impl(item_impl) => {
            extract_impl(item_impl, surface)?;
        }
        syn::Item::Use(item_use) => {
            if is_public(&item_use.vis) {
                surface.reexports.push(item_use.to_token_stream().to_string());
            }
        }
        syn::Item::Macro(item_macro) => {
            let name = item_macro
                .mac
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            if name == "macro_rules" {
                if let Some(ident) = &item_macro.ident {
                    surface.functions.push(format!(
                        "{}macro_rules! {};",
                        format_attrs(&item_macro.attrs),
                        ident
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn extract_file_api(
    path: &Path,
    surface: &mut ApiSurface,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical) {
        return Ok(());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let file = match syn::parse_file(&content) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("  [extract] Warning: failed to parse {}: {}", path.display(), e);
            return Ok(());
        }
    };

    let dir = path.parent().unwrap_or(Path::new(""));

    for item in &file.items {
        extract_item(item, dir, surface, visited)?;
    }

    Ok(())
}

fn extract_item(
    item: &syn::Item,
    dir: &Path,
    surface: &mut ApiSurface,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    match item {
        syn::Item::Fn(item_fn) => {
            if is_public(&item_fn.vis) {
                surface.functions.push(format_fn(item_fn));
            }
        }
        syn::Item::Struct(item_struct) => {
            if is_public(&item_struct.vis) {
                surface.structs.push(format_item!(item_struct));
            }
        }
        syn::Item::Enum(item_enum) => {
            if is_public(&item_enum.vis) {
                surface.enums.push(format_item!(item_enum));
            }
        }
        syn::Item::Trait(item_trait) => {
            if is_public(&item_trait.vis) {
                surface.traits.push(format_trait(item_trait));
            }
        }
        syn::Item::Type(item_type) => {
            if is_public(&item_type.vis) {
                surface.types.push(format_item!(item_type));
            }
        }
        syn::Item::Const(item_const) => {
            if is_public(&item_const.vis) {
                surface.consts.push(format_item!(item_const));
            }
        }
        syn::Item::Static(item_static) => {
            if is_public(&item_static.vis) {
                surface.statics.push(format_item!(item_static));
            }
        }
        syn::Item::Mod(item_mod) => {
            if is_public(&item_mod.vis) {
                surface.modules.push(item_mod.ident.to_string());
                if let Some((_, items)) = &item_mod.content {
                    for inner in items {
                        extract_item(inner, dir, surface, visited)?;
                    }
                } else {
                    let mod_name = item_mod.ident.to_string();
                    let mod_file = dir.join(format!("{}.rs", mod_name));
                    if mod_file.exists() {
                        extract_file_api(&mod_file, surface, visited)?;
                    } else {
                        let mod_dir_file = dir.join(&mod_name).join("mod.rs");
                        if mod_dir_file.exists() {
                            extract_file_api(&mod_dir_file, surface, visited)?;
                        }
                    }
                }
            }
        }
        syn::Item::Impl(item_impl) => {
            extract_impl(item_impl, surface)?;
        }
        syn::Item::Use(item_use) => {
            if is_public(&item_use.vis) {
                surface.reexports.push(item_use.to_token_stream().to_string());
            }
        }
        syn::Item::Macro(item_macro) => {
            // Include macro declarations so the agent knows they exist
            let name = item_macro
                .mac
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            if name == "macro_rules" {
                if let Some(ident) = &item_macro.ident {
                    surface.functions.push(format!(
                        "{}macro_rules! {};",
                        format_attrs(&item_macro.attrs),
                        ident
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_public(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn format_fn(item_fn: &syn::ItemFn) -> String {
    let attrs = format_attrs(&item_fn.attrs);
    let vis = item_fn.vis.to_token_stream().to_string();
    let sig = item_fn.sig.to_token_stream().to_string();
    format!("{}{} {};", attrs, vis, sig)
}

fn format_trait(item_trait: &syn::ItemTrait) -> String {
    let attrs = format_attrs(&item_trait.attrs);
    let vis = item_trait.vis.to_token_stream().to_string();
    let ident = &item_trait.ident;
    let generics = item_trait.generics.to_token_stream().to_string();
    let supertraits = item_trait.supertraits.to_token_stream().to_string();

    let mut items = Vec::new();
    for item in &item_trait.items {
        match item {
            syn::TraitItem::Fn(method) => {
                let doc = format_attrs(&method.attrs);
                let sig = method.sig.to_token_stream().to_string();
                items.push(format!("{}    {};", doc, sig));
            }
            syn::TraitItem::Type(ty) => {
                items.push(format!("    {}", ty.to_token_stream().to_string()));
            }
            syn::TraitItem::Const(c) => {
                items.push(format!("    {}", c.to_token_stream().to_string()));
            }
            syn::TraitItem::Macro(_) => {}
            _ => {}
        }
    }

    let super_clause = if supertraits.is_empty() {
        String::new()
    } else {
        format!(": {}", supertraits)
    };

    format!(
        "{} {} trait {}{}{} {{\n{}\n}}",
        attrs,
        vis,
        ident,
        generics,
        super_clause,
        items.join("\n")
    )
}

fn extract_impl(item_impl: &syn::ItemImpl, surface: &mut ApiSurface) -> Result<()> {
    let pub_items: Vec<String> = item_impl
        .items
        .iter()
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method) if is_public(&method.vis) => {
                let doc = format_attrs(&method.attrs);
                let sig = method.sig.to_token_stream().to_string();
                Some(format!("{}    {};", doc, sig))
            }
            syn::ImplItem::Type(ty) if is_public(&ty.vis) => {
                Some(format!("    {}", ty.to_token_stream().to_string()))
            }
            syn::ImplItem::Const(c) if is_public(&c.vis) => {
                Some(format!("    {}", c.to_token_stream().to_string()))
            }
            _ => None,
        })
        .collect();

    if pub_items.is_empty() {
        return Ok(());
    }

    let self_ty = item_impl.self_ty.to_token_stream().to_string();
    let generics = item_impl.generics.to_token_stream().to_string();
    let where_clause = item_impl
        .generics
        .where_clause
        .to_token_stream()
        .to_string();

    let header = if let Some((_, trait_path, _)) = &item_impl.trait_ {
        format!(
            "impl {} for {}{}",
            trait_path.to_token_stream().to_string(),
            self_ty,
            generics
        )
    } else {
        format!("impl {}{}", self_ty, generics)
    };

    let where_str = if where_clause.is_empty() {
        String::new()
    } else {
        format!(" {}", where_clause)
    };

    surface.impls.push(format!(
        "{}{} {{\n{}\n}}",
        header,
        where_str,
        pub_items.join("\n")
    ));

    Ok(())
}

fn format_surface(surface: &ApiSurface) -> String {
    let mut parts = Vec::new();

    if !surface.docs.is_empty() {
        parts.push(format!("// Crate documentation\n{}", surface.docs.join("\n")));
    }

    if !surface.reexports.is_empty() {
        parts.push(format!(
            "// Re-exports\n{}",
            surface.reexports.join("\n")
        ));
    }

    if !surface.modules.is_empty() {
        let mods: Vec<_> = surface
            .modules
            .iter()
            .map(|m| format!("pub mod {};", m))
            .collect();
        parts.push(format!("// Modules\n{}", mods.join("\n")));
    }

    if !surface.structs.is_empty() {
        parts.push(format!("// Structs\n{}", surface.structs.join("\n\n")));
    }

    if !surface.enums.is_empty() {
        parts.push(format!("// Enums\n{}", surface.enums.join("\n\n")));
    }

    if !surface.traits.is_empty() {
        parts.push(format!("// Traits\n{}", surface.traits.join("\n\n")));
    }

    if !surface.types.is_empty() {
        parts.push(format!(
            "// Type Aliases\n{}",
            surface.types.join("\n\n")
        ));
    }

    if !surface.impls.is_empty() {
        parts.push(format!(
            "// Implementations\n{}",
            surface.impls.join("\n\n")
        ));
    }

    if !surface.functions.is_empty() {
        parts.push(format!("// Functions\n{}", surface.functions.join("\n\n")));
    }

    if !surface.consts.is_empty() {
        parts.push(format!(
            "// Constants\n{}",
            surface.consts.join("\n\n")
        ));
    }

    if !surface.statics.is_empty() {
        parts.push(format!(
            "// Statics\n{}",
            surface.statics.join("\n\n")
        ));
    }

    parts.join("\n\n")
}
