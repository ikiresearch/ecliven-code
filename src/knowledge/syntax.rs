//! Syntax-tree analysis for Rust, Python, JavaScript, TypeScript and Go:
//! symbol spans with their parent (method -> impl/class), call sites with the
//! enclosing function, and type references. Other languages use regex rules.

use tree_sitter::{Language, Node, Parser};

const MAX_REFS: usize = 5000;
const MAX_DEPTH: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynSymbol {
    pub name: String,
    pub kind: &'static str,
    pub start: i64,
    pub end: i64,
    pub parent: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynRef {
    pub name: String,
    pub line: i64,
    pub caller: String,
    pub kind: &'static str,
}

#[derive(Debug, Default)]
pub struct Syntax {
    pub symbols: Vec<SynSymbol>,
    pub refs: Vec<SynRef>,
    /// Module declarations without a body (`mod foo;`), as `self::foo`.
    pub mod_decls: Vec<String>,
}

pub fn supports(lang: &str) -> bool {
    grammar(lang).is_some()
}

fn grammar(lang: &str) -> Option<Language> {
    Some(match lang {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        _ => return None,
    })
}

fn def_kind(lang: &str, node: &Node<'_>) -> Option<&'static str> {
    let k = node.kind();
    match lang {
        "rust" => Some(match k {
            "function_item" | "function_signature_item" => "fn",
            "struct_item" | "union_item" => "struct",
            "enum_item" => "enum",
            "trait_item" => "trait",
            "impl_item" => "impl",
            "mod_item" => "mod",
            "type_item" => "type",
            "const_item" | "static_item" => "const",
            "macro_definition" => "macro",
            _ => return None,
        }),
        "python" => Some(match k {
            "function_definition" => "fn",
            "class_definition" => "class",
            _ => return None,
        }),
        "javascript" | "typescript" => Some(match k {
            "function_declaration" | "generator_function_declaration" | "method_definition" => "fn",
            "class_declaration" | "abstract_class_declaration" => "class",
            "interface_declaration" => "interface",
            "type_alias_declaration" => "type",
            "enum_declaration" => "enum",
            "variable_declarator" => {
                let value = node.child_by_field_name("value")?;
                match value.kind() {
                    "arrow_function" | "function_expression" | "function" => "fn",
                    _ => return None,
                }
            }
            _ => return None,
        }),
        "go" => Some(match k {
            "function_declaration" | "method_declaration" => "fn",
            "type_spec" => "type",
            _ => return None,
        }),
        _ => None,
    }
}

fn is_container(kind: &str) -> bool {
    matches!(kind, "impl" | "class" | "trait" | "interface" | "mod" | "struct" | "enum")
}

fn is_call(lang: &str, kind: &str) -> bool {
    match lang {
        "python" => kind == "call",
        "javascript" | "typescript" => kind == "call_expression" || kind == "new_expression",
        _ => kind == "call_expression",
    }
}

fn text<'a>(node: &Node<'_>, src: &'a str) -> &'a str {
    src.get(node.byte_range()).unwrap_or("")
}

fn base_type_name(raw: &str) -> String {
    raw.split(['<', '[', ' ', '('])
        .next()
        .unwrap_or("")
        .trim_start_matches(['&', '*'])
        .rsplit("::")
        .next()
        .unwrap_or("")
        .to_string()
}

/// Last identifier of a callee expression (`a.b.c()` -> `c`, `x::y()` -> `y`).
fn callee_name(node: Node<'_>, src: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" | "property_identifier" | "type_identifier" => {
            Some(text(&node, src).to_string())
        }
        "scoped_identifier" | "generic_function" | "member_expression" | "field_expression"
        | "attribute" | "selector_expression" | "parenthesized_expression" => {
            let child = ["name", "property", "field", "attribute", "function"]
                .iter()
                .find_map(|f| node.child_by_field_name(f))
                .or_else(|| node.named_child(node.named_child_count().checked_sub(1)?))?;
            callee_name(child, src)
        }
        _ => None,
    }
}

fn symbol_name(lang: &str, kind: &str, node: &Node<'_>, src: &str) -> Option<String> {
    if kind == "impl" {
        return Some(base_type_name(text(&node.child_by_field_name("type")?, src)));
    }
    let name = node.child_by_field_name("name")?;
    let raw = text(&name, src);
    let _ = lang;
    (!raw.is_empty()).then(|| raw.to_string())
}

fn go_receiver(node: &Node<'_>, src: &str) -> Option<String> {
    let recv = node.child_by_field_name("receiver")?;
    let mut stack = vec![recv];
    while let Some(n) = stack.pop() {
        if n.kind() == "type_identifier" {
            return Some(text(&n, src).to_string());
        }
        for i in (0..n.named_child_count()).rev() {
            if let Some(c) = n.named_child(i) {
                stack.push(c);
            }
        }
    }
    None
}

struct Walker<'s> {
    lang: &'s str,
    src: &'s str,
    out: Syntax,
}

impl Walker<'_> {
    fn walk(&mut self, node: Node<'_>, scope: &mut Vec<String>, caller: &str, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let line = node.start_position().row as i64 + 1;
        let mut pushed_scope = false;
        let mut caller_here = caller.to_string();

        if let Some(kind) = def_kind(self.lang, &node) {
            if let Some(name) = symbol_name(self.lang, kind, &node, self.src) {
                let parent = if self.lang == "go" && node.kind() == "method_declaration" {
                    go_receiver(&node, self.src).unwrap_or_default()
                } else {
                    scope.last().cloned().unwrap_or_default()
                };
                let first_line = text(&node, self.src).lines().next().unwrap_or("").trim();
                if kind == "mod" && node.child_by_field_name("body").is_none() {
                    self.out.mod_decls.push(format!("self::{name}"));
                }
                self.out.symbols.push(SynSymbol {
                    name: name.clone(),
                    kind,
                    start: line,
                    end: node.end_position().row as i64 + 1,
                    parent,
                    signature: crate::tools::truncate_str(first_line, 160).to_string(),
                });
                if kind == "fn" {
                    caller_here = name.clone();
                }
                if is_container(kind) {
                    scope.push(name);
                    pushed_scope = true;
                }
            }
        } else if is_call(self.lang, node.kind()) && self.out.refs.len() < MAX_REFS {
            let target = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("constructor"));
            if let Some(name) = target.and_then(|t| callee_name(t, self.src)) {
                self.out.refs.push(SynRef { name, line, caller: caller.to_string(), kind: "call" });
            }
        } else if node.kind() == "type_identifier" && self.out.refs.len() < MAX_REFS {
            let is_def_name = node
                .parent()
                .filter(|p| def_kind(self.lang, p).is_some())
                .and_then(|p| p.child_by_field_name("name"))
                .is_some_and(|n| n.id() == node.id());
            if !is_def_name {
                self.out.refs.push(SynRef {
                    name: text(&node, self.src).to_string(),
                    line,
                    caller: caller.to_string(),
                    kind: "type",
                });
            }
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, scope, &caller_here, depth + 1);
        }
        if pushed_scope {
            scope.pop();
        }
    }
}

/// `None` when the language has no grammar or the source cannot be parsed.
pub fn parse(lang: &str, src: &str) -> Option<Syntax> {
    let language = grammar(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(src, None)?;
    let mut walker = Walker { lang, src, out: Syntax::default() };
    walker.walk(tree.root_node(), &mut Vec::new(), "", 0);
    Some(walker.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym<'a>(s: &'a Syntax, name: &str) -> &'a SynSymbol {
        s.symbols.iter().find(|x| x.name == name).unwrap()
    }

    #[test]
    fn rust_spans_parents_calls_and_types() {
        let src = "mod util;\npub struct Engine { cfg: Config }\n\nimpl Engine {\n    pub fn run(&self) {\n        let x = helper(1);\n        self.step();\n        util::go();\n    }\n}\n\nfn helper(n: u32) -> u32 { n }\n";
        let s = parse("rust", src).unwrap();
        let run = sym(&s, "run");
        assert_eq!((run.start, run.end, run.parent.as_str()), (5, 9, "Engine"));
        assert_eq!(sym(&s, "Engine").kind, "struct");
        let calls: Vec<(&str, &str)> = s.refs.iter().filter(|r| r.kind == "call").map(|r| (r.name.as_str(), r.caller.as_str())).collect();
        assert_eq!(calls, vec![("helper", "run"), ("step", "run"), ("go", "run")]);
        assert!(s.refs.iter().any(|r| r.kind == "type" && r.name == "Config"));
        assert_eq!(s.mod_decls, vec!["self::util"]);
    }

    #[test]
    fn python_ts_and_go() {
        let py = "class Service:\n    def run(self):\n        return load(self.x)\n";
        let s = parse("python", py).unwrap();
        assert_eq!(sym(&s, "run").parent, "Service");
        assert!(s.refs.iter().any(|r| r.name == "load" && r.caller == "run"));

        let ts = "export const handler = async (req: Request) => {\n  return render(req);\n};\nclass View { show() { this.paint(); } }\n";
        let s = parse("typescript", ts).unwrap();
        assert_eq!(sym(&s, "handler").kind, "fn");
        assert_eq!(sym(&s, "show").parent, "View");
        assert!(s.refs.iter().any(|r| r.name == "render" && r.caller == "handler"));
        assert!(s.refs.iter().any(|r| r.name == "paint" && r.caller == "show"));

        let go = "package main\n\ntype Server struct{}\n\nfunc (s *Server) Start() {\n\tlisten()\n}\n";
        let s = parse("go", go).unwrap();
        assert_eq!(sym(&s, "Start").parent, "Server");
        assert!(s.refs.iter().any(|r| r.name == "listen" && r.caller == "Start"));
    }
}
