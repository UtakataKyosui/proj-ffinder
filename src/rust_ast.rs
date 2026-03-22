use syn::{
    Attribute, File, Item, ItemConst, ItemEnum, ItemFn, ItemImpl, ItemMod, ItemStatic, ItemStruct,
    ItemTrait, ItemType, UseTree, Visibility, parse_file,
};

#[derive(Debug, Default)]
pub struct RustAstSummary {
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub symbols: Vec<String>,
    pub roles: Vec<String>,
}

pub fn analyze_rust_file(content: &str) -> Option<RustAstSummary> {
    let file = parse_file(content).ok()?;
    let mut summary = RustAstSummary::default();
    visit_file(&file, &mut summary);
    dedupe_summary(&mut summary);
    Some(summary)
}

fn visit_file(file: &File, summary: &mut RustAstSummary) {
    visit_items(&file.items, summary);
}

fn visit_items(items: &[Item], summary: &mut RustAstSummary) {
    for item in items {
        match item {
            Item::Use(item_use) => {
                collect_use_tree(&item_use.tree, String::new(), &mut summary.imports);
            }
            Item::Fn(item_fn) => visit_fn(item_fn, summary),
            Item::Struct(item_struct) => visit_struct(item_struct, summary),
            Item::Enum(item_enum) => visit_enum(item_enum, summary),
            Item::Trait(item_trait) => visit_trait(item_trait, summary),
            Item::Type(item_type) => visit_type(item_type, summary),
            Item::Const(item_const) => visit_const(item_const, summary),
            Item::Static(item_static) => visit_static(item_static, summary),
            Item::Mod(item_mod) => visit_mod(item_mod, summary),
            Item::Impl(item_impl) => visit_impl(item_impl, summary),
            _ => {}
        }
    }
}

fn visit_fn(item_fn: &ItemFn, summary: &mut RustAstSummary) {
    let symbol = item_fn.sig.ident.to_string().to_ascii_lowercase();
    summary.symbols.push(symbol.clone());
    if is_public(&item_fn.vis) {
        summary.exports.push(symbol.clone());
    }
    if symbol == "main" {
        summary.roles.push("entrypoint".to_owned());
    }
    if has_test_attr(&item_fn.attrs) {
        summary.roles.push("test".to_owned());
    }
}

fn visit_struct(item_struct: &ItemStruct, summary: &mut RustAstSummary) {
    push_named_item(
        item_struct.ident.to_string(),
        &item_struct.vis,
        &mut summary.symbols,
        &mut summary.exports,
    );
}

fn visit_enum(item_enum: &ItemEnum, summary: &mut RustAstSummary) {
    push_named_item(
        item_enum.ident.to_string(),
        &item_enum.vis,
        &mut summary.symbols,
        &mut summary.exports,
    );
}

fn visit_trait(item_trait: &ItemTrait, summary: &mut RustAstSummary) {
    push_named_item(
        item_trait.ident.to_string(),
        &item_trait.vis,
        &mut summary.symbols,
        &mut summary.exports,
    );
}

fn visit_type(item_type: &ItemType, summary: &mut RustAstSummary) {
    push_named_item(
        item_type.ident.to_string(),
        &item_type.vis,
        &mut summary.symbols,
        &mut summary.exports,
    );
}

fn visit_const(item_const: &ItemConst, summary: &mut RustAstSummary) {
    push_named_item(
        item_const.ident.to_string(),
        &item_const.vis,
        &mut summary.symbols,
        &mut summary.exports,
    );
}

fn visit_static(item_static: &ItemStatic, summary: &mut RustAstSummary) {
    push_named_item(
        item_static.ident.to_string(),
        &item_static.vis,
        &mut summary.symbols,
        &mut summary.exports,
    );
}

fn visit_mod(item_mod: &ItemMod, summary: &mut RustAstSummary) {
    let symbol = item_mod.ident.to_string().to_ascii_lowercase();
    summary.symbols.push(symbol.clone());
    if is_public(&item_mod.vis) {
        summary.exports.push(symbol.clone());
    }
    if symbol == "tests" || has_cfg_test(&item_mod.attrs) {
        summary.roles.push("test".to_owned());
    }
    if let Some((_, items)) = &item_mod.content {
        visit_items(items, summary);
    }
}

fn visit_impl(item_impl: &ItemImpl, summary: &mut RustAstSummary) {
    for impl_item in &item_impl.items {
        if let syn::ImplItem::Fn(method) = impl_item {
            let symbol = method.sig.ident.to_string().to_ascii_lowercase();
            summary.symbols.push(symbol);
            if has_test_attr(&method.attrs) {
                summary.roles.push("test".to_owned());
            }
        }
    }
}

fn collect_use_tree(tree: &UseTree, prefix: String, imports: &mut Vec<String>) {
    match tree {
        UseTree::Path(path) => {
            let next = join_segments(&prefix, &path.ident.to_string());
            collect_use_tree(&path.tree, next, imports);
        }
        UseTree::Name(name) => {
            imports.push(join_segments(&prefix, &name.ident.to_string()).to_ascii_lowercase());
        }
        UseTree::Rename(rename) => {
            imports.push(join_segments(&prefix, &rename.ident.to_string()).to_ascii_lowercase());
        }
        UseTree::Glob(_) => {
            imports.push(join_segments(&prefix, "*").to_ascii_lowercase());
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, prefix.clone(), imports);
            }
        }
    }
}

fn push_named_item(
    symbol: String,
    visibility: &Visibility,
    symbols: &mut Vec<String>,
    exports: &mut Vec<String>,
) {
    let symbol = symbol.to_ascii_lowercase();
    symbols.push(symbol.clone());
    if is_public(visibility) {
        exports.push(symbol);
    }
}

fn is_public(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Public(_))
}

fn has_test_attr(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn has_cfg_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        if !attribute.path().is_ident("cfg") {
            return false;
        }
        match &attribute.meta {
            syn::Meta::List(meta_list) => meta_list.tokens.to_string().contains("test"),
            _ => false,
        }
    })
}

fn join_segments(prefix: &str, segment: &str) -> String {
    if prefix.is_empty() {
        segment.to_owned()
    } else {
        format!("{prefix}::{segment}")
    }
}

fn dedupe_summary(summary: &mut RustAstSummary) {
    summary.imports = dedupe(summary.imports.clone());
    summary.exports = dedupe(summary.exports.clone());
    summary.symbols = dedupe(summary.symbols.clone());
    summary.roles = dedupe(summary.roles.clone());
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut unique = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            unique.push(value);
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::analyze_rust_file;

    #[test]
    fn extracts_nested_use_items() {
        let content = "use serde::{Serialize, Deserialize};";
        let summary = analyze_rust_file(content).expect("rust AST should parse");

        assert!(summary.imports.contains(&"serde::serialize".to_owned()));
        assert!(summary.imports.contains(&"serde::deserialize".to_owned()));
    }

    #[test]
    fn extracts_symbols_exports_and_roles() {
        let content = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn works() {}
}

pub struct User;
fn main() {}
"#;
        let summary = analyze_rust_file(content).expect("rust AST should parse");

        assert!(summary.symbols.contains(&"tests".to_owned()));
        assert!(summary.symbols.contains(&"works".to_owned()));
        assert!(summary.symbols.contains(&"user".to_owned()));
        assert!(summary.symbols.contains(&"main".to_owned()));
        assert!(summary.exports.contains(&"user".to_owned()));
        assert!(summary.roles.contains(&"test".to_owned()));
        assert!(summary.roles.contains(&"entrypoint".to_owned()));
    }
}
