use tree_sitter::{Language, Node, Parser, Tree};

#[derive(Debug, Default)]
pub struct PartialAstSummary {
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub symbols: Vec<String>,
    pub roles: Vec<String>,
}

pub fn analyze_other_file(content: &str, extension: Option<&str>) -> Option<PartialAstSummary> {
    match extension {
        Some("ts") | Some("js") => analyze_typescript(content, false),
        Some("tsx") | Some("jsx") => analyze_typescript(content, true),
        Some("py") => analyze_python(content),
        Some("go") => analyze_go(content),
        _ => None,
    }
}

fn analyze_typescript(content: &str, is_tsx: bool) -> Option<PartialAstSummary> {
    let language = if is_tsx {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    let tree = parse_tree(content, language.into())?;
    let mut summary = PartialAstSummary::default();
    visit_typescript_node(tree.root_node(), content.as_bytes(), &mut summary, false);
    dedupe_summary(&mut summary);
    Some(summary)
}

fn analyze_python(content: &str) -> Option<PartialAstSummary> {
    let tree = parse_tree(content, tree_sitter_python::LANGUAGE.into())?;
    let mut summary = PartialAstSummary::default();
    visit_python_node(tree.root_node(), content.as_bytes(), &mut summary);
    dedupe_summary(&mut summary);
    Some(summary)
}

fn analyze_go(content: &str) -> Option<PartialAstSummary> {
    let tree = parse_tree(content, tree_sitter_go::LANGUAGE.into())?;
    let mut summary = PartialAstSummary::default();
    let mut package_main = false;
    visit_go_node(
        tree.root_node(),
        content.as_bytes(),
        &mut summary,
        &mut package_main,
    );
    if package_main && summary.symbols.iter().any(|symbol| symbol == "main") {
        summary.roles.push("entrypoint".to_owned());
    }
    dedupe_summary(&mut summary);
    Some(summary)
}

fn parse_tree(content: &str, language: Language) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    parser.parse(content, None)
}

fn visit_typescript_node(
    node: Node<'_>,
    source: &[u8],
    summary: &mut PartialAstSummary,
    exported: bool,
) {
    match node.kind() {
        "import_statement" => {
            if let Some(module_name) = node
                .child_by_field_name("source")
                .and_then(|node| node_text(node, source))
            {
                summary
                    .imports
                    .push(strip_quotes(&module_name).to_ascii_lowercase());
            }
        }
        "export_statement" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                visit_typescript_node(child, source, summary, true);
            }
            return;
        }
        "function_declaration"
        | "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration"
        | "module"
        | "internal_module" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node_text(node, source))
            {
                push_symbol(summary, &name, exported);
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "variable_declarator" | "variable_declaration" | "lexical_binding"
                ) && let Some(name_node) = child.child_by_field_name("name")
                    && let Some(name) = node_text(name_node, source)
                {
                    push_symbol(summary, &name, exported);
                }
            }
        }
        "method_definition" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node_text(node, source))
            {
                summary.symbols.push(name.to_ascii_lowercase());
            }
        }
        "call_expression" => {
            if let Some(function_node) = node.child_by_field_name("function")
                && let Some(name) = node_text(function_node, source)
            {
                let normalized = name.to_ascii_lowercase();
                if matches!(normalized.as_str(), "describe" | "it" | "test") {
                    summary.roles.push("test".to_owned());
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_typescript_node(child, source, summary, exported);
    }
}

fn visit_python_node(node: Node<'_>, source: &[u8], summary: &mut PartialAstSummary) {
    match node.kind() {
        "import_statement" | "import_from_statement" => {
            if let Some(text) = node_text(node, source) {
                for import in parse_python_import_text(&text) {
                    summary.imports.push(import);
                }
            }
        }
        "function_definition" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node_text(node, source))
            {
                let normalized = name.to_ascii_lowercase();
                summary.symbols.push(normalized.clone());
                summary.exports.push(normalized.clone());
                if normalized.starts_with("test_") {
                    summary.roles.push("test".to_owned());
                }
            }
        }
        "class_definition" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node_text(node, source))
            {
                let normalized = name.to_ascii_lowercase();
                summary.symbols.push(normalized.clone());
                summary.exports.push(normalized.clone());
                if normalized.starts_with("test") {
                    summary.roles.push("test".to_owned());
                }
            }
        }
        "if_statement" => {
            if let Some(text) = node_text(node, source)
                && text.contains("__name__")
                && text.contains("__main__")
            {
                summary.roles.push("entrypoint".to_owned());
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_python_node(child, source, summary);
    }
}

fn visit_go_node(
    node: Node<'_>,
    source: &[u8],
    summary: &mut PartialAstSummary,
    package_main: &mut bool,
) {
    match node.kind() {
        "package_clause" => {
            if let Some(text) = node_text(node, source)
                && text.trim().eq_ignore_ascii_case("package main")
            {
                *package_main = true;
            }
        }
        "import_spec" => {
            if let Some(path_node) = node.child_by_field_name("path")
                && let Some(path) = node_text(path_node, source)
            {
                summary
                    .imports
                    .push(strip_quotes(&path).to_ascii_lowercase());
            }
        }
        "function_declaration" | "method_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node_text(node, source))
            {
                let normalized = name.to_ascii_lowercase();
                summary.symbols.push(normalized.clone());
                if is_go_exported(&name) {
                    summary.exports.push(normalized.clone());
                }
                if name == "main" {
                    summary.symbols.push("main".to_owned());
                }
                if name.starts_with("Test") {
                    summary.roles.push("test".to_owned());
                }
            }
        }
        "type_spec" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|node| node_text(node, source))
            {
                let normalized = name.to_ascii_lowercase();
                summary.symbols.push(normalized.clone());
                if is_go_exported(&name) {
                    summary.exports.push(normalized);
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_go_node(child, source, summary, package_main);
    }
}

fn push_symbol(summary: &mut PartialAstSummary, symbol: &str, exported: bool) {
    let normalized = symbol.to_ascii_lowercase();
    summary.symbols.push(normalized.clone());
    if exported {
        summary.exports.push(normalized);
    }
}

fn parse_python_import_text(text: &str) -> Vec<String> {
    let trimmed = text.trim();

    if let Some(rest) = trimmed.strip_prefix("import ") {
        return rest.split(',').filter_map(normalize_import).collect();
    }

    if let Some(rest) = trimmed.strip_prefix("from ")
        && let Some((module, _)) = rest.split_once(" import ")
        && let Some(module) = normalize_import(module)
    {
        return vec![module];
    }

    Vec::new()
}

fn normalize_import(value: &str) -> Option<String> {
    let normalized = value.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn node_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.utf8_text(source).ok().map(str::to_owned)
}

fn strip_quotes(value: &str) -> String {
    value.trim().trim_matches(['"', '\'']).to_owned()
}

fn is_go_exported(name: &str) -> bool {
    name.chars()
        .next()
        .map(|char| char.is_ascii_uppercase())
        .unwrap_or(false)
}

fn dedupe_summary(summary: &mut PartialAstSummary) {
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
    use super::analyze_other_file;

    #[test]
    fn extracts_typescript_imports_symbols_and_exports() {
        let content = r#"
import { describe } from "vitest";
export function login() {}
"#;
        let summary = analyze_other_file(content, Some("ts")).expect("typescript should parse");

        assert!(summary.imports.contains(&"vitest".to_owned()));
        assert!(summary.symbols.contains(&"login".to_owned()));
        assert!(summary.exports.contains(&"login".to_owned()));
    }

    #[test]
    fn extracts_javascript_and_jsx_with_typescript_grammar() {
        let js_content = r#"
import axios from "axios";
export const login = () => {};
"#;
        let jsx_content = r#"
import React from "react";
export function LoginPage() {
  return <main>hello</main>;
}
"#;

        let js_summary =
            analyze_other_file(js_content, Some("js")).expect("javascript should parse");
        let jsx_summary = analyze_other_file(jsx_content, Some("jsx")).expect("jsx should parse");

        assert!(js_summary.imports.contains(&"axios".to_owned()));
        assert!(js_summary.symbols.contains(&"login".to_owned()));
        assert!(js_summary.exports.contains(&"login".to_owned()));

        assert!(jsx_summary.imports.contains(&"react".to_owned()));
        assert!(jsx_summary.symbols.contains(&"loginpage".to_owned()));
        assert!(jsx_summary.exports.contains(&"loginpage".to_owned()));
    }

    #[test]
    fn extracts_python_imports_and_roles() {
        let content = r#"
import os
from pathlib import Path

def test_login():
    pass

if __name__ == "__main__":
    test_login()
"#;
        let summary = analyze_other_file(content, Some("py")).expect("python should parse");

        assert!(summary.imports.contains(&"os".to_owned()));
        assert!(summary.imports.contains(&"pathlib".to_owned()));
        assert!(summary.symbols.contains(&"test_login".to_owned()));
        assert!(summary.roles.contains(&"test".to_owned()));
        assert!(summary.roles.contains(&"entrypoint".to_owned()));
    }

    #[test]
    fn extracts_go_imports_exports_and_roles() {
        let content = r#"
package main

import "net/http"

func TestLogin() {}
func main() {}
type User struct {}
"#;
        let summary = analyze_other_file(content, Some("go")).expect("go should parse");

        assert!(summary.imports.contains(&"net/http".to_owned()));
        assert!(summary.symbols.contains(&"testlogin".to_owned()));
        assert!(summary.symbols.contains(&"main".to_owned()));
        assert!(summary.exports.contains(&"user".to_owned()));
        assert!(summary.roles.contains(&"test".to_owned()));
        assert!(summary.roles.contains(&"entrypoint".to_owned()));
    }
}
