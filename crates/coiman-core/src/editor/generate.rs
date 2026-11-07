//! Code actions made here, without a language server: for Java and C#, getters and
//! setters (C# properties), a constructor, `toString()` / `ToString()`, and `equals()`
//! with `hashCode()` / `Equals()` with `GetHashCode()`, as the VS Code Java extension
//! and Visual Studio generate them; for C and C++, a NULL check after a pointer that may
//! be NULL is set. Nothing changes until one is chosen.

use std::collections::HashSet;

use tree_sitter::{Node, Parser};

use crate::language::syntax::Lang;

/// A change to the file: bytes `start..end` replaced by `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Writes new code (`source.generate`).
    Generate,
    /// Fixes a problem shown on the line (`quickfix`).
    QuickFix,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub title: String,
    pub kind: Kind,
    pub edits: Vec<Edit>,
}

/// How the file is written.
#[derive(Clone, Copy, Debug)]
pub struct Style<'a> {
    /// One level of indentation.
    pub unit: &'a str,
    pub nl: &'a str,
    /// C#: nullable reference types are on (`object?`).
    pub nullable: bool,
}

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn named_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn lowered(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The indentation of the line `byte` is on.
fn line_indent(source: &str, byte: usize) -> String {
    let start = source[..byte.min(source.len())].rfind('\n').map_or(0, |i| i + 1);
    source[start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// The actions at byte `cursor` of a file of this type.
pub fn actions(extension: &str, source: &str, cursor: usize, style: Style) -> Vec<Action> {
    let Some(lang) = Lang::from_extension(extension) else { return Vec::new() };
    if !matches!(lang, Lang::Java | Lang::CSharp | Lang::C | Lang::Cpp) || source.len() > 1024 * 1024 {
        return Vec::new();
    }
    let mut parser = Parser::new();
    if parser.set_language(&lang.language()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else { return Vec::new() };
    let root = tree.root_node();
    match lang {
        Lang::Java => java(root, source, cursor, style),
        Lang::CSharp => csharp(root, source, cursor, style),
        _ => null_fixes(root, source, cursor, lang == Lang::Cpp, style),
    }
}

// ----- C and C++ -----------------------------------------------------------------------

fn null_fixes(root: Node, source: &str, cursor: usize, cpp: bool, style: Style) -> Vec<Action> {
    let line = source[..cursor.min(source.len())].matches('\n').count();
    crate::language::null_check::find(root, source, cpp, style.unit, style.nl)
        .into_iter()
        .filter(|u| u.use_line == line || u.set_line == line)
        .map(|u| Action {
            title: format!("Add a {} check for `{}` (from {})", if cpp { "null" } else { "NULL" }, u.name, u.from),
            kind: Kind::QuickFix,
            edits: vec![Edit { start: u.set_end, end: u.set_end, text: u.fix }],
        })
        .collect()
}

// ----- shared by Java and C# -----------------------------------------------------------

#[derive(Clone, Debug)]
struct Field {
    name: String,
    ty: String,
    /// `final` / `readonly`: no setter.
    fixed: bool,
    /// Given its value where it's declared.
    initialized: bool,
}

/// The class the cursor is in, the innermost: its node, name and body.
fn class_at<'t>(root: Node<'t>, cursor: usize, source: &str, kinds: &[&str]) -> Option<(Node<'t>, String, Node<'t>)> {
    let mut found = None;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.start_byte() > cursor || node.end_byte() < cursor {
            continue;
        }
        if kinds.contains(&node.kind())
            && let (Some(name), Some(body)) = (node.child_by_field_name("name"), node.child_by_field_name("body"))
        {
            found = Some((node, text(name, source).to_owned(), body));
        }
        stack.extend(named_children(node));
    }
    found
}

/// Where new members go: the start of the line holding the body's `}` (or just before
/// it, when something else is on that line), and their indentation.
fn insertion(body: Node, class: Node, source: &str, style: Style) -> (usize, String, bool) {
    let close = body.end_byte().saturating_sub(1);
    let line_start = source[..close].rfind('\n').map_or(0, |i| i + 1);
    let alone = source[line_start..close].trim().is_empty();
    let members = named_children(body);
    let indent = members
        .iter()
        .find(|m| !matches!(m.kind(), "line_comment" | "block_comment" | "comment"))
        .map(|m| line_indent(source, m.start_byte()))
        .unwrap_or_else(|| line_indent(source, class.start_byte()) + style.unit);
    if alone { (line_start, indent, true) } else { (close, indent, false) }
}

/// Members put at `at`, each after a blank line.
fn members_edit(at: usize, alone: bool, members: &[String], style: Style) -> Edit {
    let nl = style.nl;
    let mut text = String::new();
    if !alone {
        text.push_str(nl);
    }
    for m in members {
        text.push_str(nl);
        text.push_str(m);
    }
    Edit { start: at, end: at, text }
}

/// Lines indented by `indent`, joined with the file's line ending, ending with one.
fn block(indent: &str, lines: &[String], nl: &str) -> String {
    lines.iter().map(|l| if l.is_empty() { nl.to_owned() } else { format!("{indent}{l}{nl}") }).collect()
}

/// An import line added after the last of its kind (or the package line), unless the
/// file has it already.
fn import_edit(root: Node, source: &str, kinds: &[&str], line: &str, style: Style) -> Option<Edit> {
    if source.lines().any(|l| l.trim() == line) {
        return None;
    }
    let nodes = named_children(root);
    let last_import = nodes.iter().rfind(|n| kinds.contains(&n.kind()));
    let package = nodes.iter().find(|n| matches!(n.kind(), "package_declaration"));
    match last_import.or(package) {
        Some(after) => {
            Some(Edit { start: after.end_byte(), end: after.end_byte(), text: format!("{}{line}", style.nl) })
        }
        None => Some(Edit { start: 0, end: 0, text: format!("{line}{}{}", style.nl, style.nl) }),
    }
}

// ----- Java ----------------------------------------------------------------------------

fn java_fields(body: Node, source: &str) -> Vec<Field> {
    let mut out = Vec::new();
    for member in named_children(body) {
        if member.kind() != "field_declaration" {
            continue;
        }
        let modifiers =
            named_children(member).into_iter().find(|c| c.kind() == "modifiers").map_or("", |m| text(m, source));
        if modifiers.split_whitespace().any(|m| m == "static") {
            continue;
        }
        let Some(ty) = member.child_by_field_name("type") else { continue };
        for declarator in named_children(member).into_iter().filter(|c| c.kind() == "variable_declarator") {
            let Some(name) = declarator.child_by_field_name("name") else { continue };
            // `int a[]` puts the brackets on the name.
            let dims = declarator.child_by_field_name("dimensions").map_or("", |d| text(d, source));
            out.push(Field {
                name: text(name, source).to_owned(),
                ty: format!("{}{dims}", text(ty, source)),
                fixed: modifiers.split_whitespace().any(|m| m == "final"),
                initialized: declarator.child_by_field_name("value").is_some(),
            });
        }
    }
    out
}

/// Methods by name with their number of parameters, and constructors by their types.
fn java_members(body: Node, source: &str) -> (HashSet<(String, usize)>, Vec<Vec<String>>) {
    let mut methods = HashSet::new();
    let mut constructors = Vec::new();
    for member in named_children(body) {
        let params = member.child_by_field_name("parameters").map(|p| {
            named_children(p)
                .into_iter()
                .filter(|c| c.kind() == "formal_parameter")
                .filter_map(|c| c.child_by_field_name("type").map(|t| text(t, source).to_owned()))
                .collect::<Vec<_>>()
        });
        match member.kind() {
            "method_declaration" => {
                if let Some(name) = member.child_by_field_name("name") {
                    methods.insert((text(name, source).to_owned(), params.map_or(0, |p| p.len())));
                }
            }
            "constructor_declaration" => constructors.push(params.unwrap_or_default()),
            _ => {}
        }
    }
    (methods, constructors)
}

fn java_getter(f: &Field) -> String {
    let prefix = if f.ty == "boolean" { "is" } else { "get" };
    format!("{prefix}{}", capitalized(&f.name))
}

fn java(root: Node, source: &str, cursor: usize, style: Style) -> Vec<Action> {
    let Some((class, name, body)) = class_at(root, cursor, source, &["class_declaration", "enum_declaration"]) else {
        return Vec::new();
    };
    let fields = java_fields(body, source);
    if fields.is_empty() {
        return Vec::new();
    }
    let (methods, constructors) = java_members(body, source);
    let (at, indent, alone) = insertion(body, class, source, style);
    let (u, nl) = (style.unit, style.nl);
    let getter = |f: &Field| {
        block(
            &indent,
            &[format!("public {} {}() {{", f.ty, java_getter(f)), format!("{u}return {};", f.name), "}".into()],
            nl,
        )
    };
    let setter = |f: &Field| {
        block(
            &indent,
            &[
                format!("public void set{}({} {}) {{", capitalized(&f.name), f.ty, f.name),
                format!("{u}this.{} = {};", f.name, f.name),
                "}".into(),
            ],
            nl,
        )
    };
    let missing_get: Vec<&Field> = fields.iter().filter(|f| !methods.contains(&(java_getter(f), 0))).collect();
    let missing_set: Vec<&Field> =
        fields.iter().filter(|f| !f.fixed && !methods.contains(&(format!("set{}", capitalized(&f.name)), 1))).collect();
    let mut out = Vec::new();
    let mut add = |title: String, members: Vec<String>, extra: Vec<Edit>| {
        let mut edits = extra;
        edits.push(members_edit(at, alone, &members, style));
        out.push(Action { title, kind: Kind::Generate, edits });
    };
    if !missing_get.is_empty() && !missing_set.is_empty() {
        let mut members = Vec::new();
        for f in &fields {
            if missing_get.iter().any(|g| g.name == f.name) {
                members.push(getter(f));
            }
            if missing_set.iter().any(|s| s.name == f.name) {
                members.push(setter(f));
            }
        }
        add("Generate Getters and Setters".into(), members, Vec::new());
    }
    if !missing_get.is_empty() {
        add("Generate Getters".into(), missing_get.iter().map(|f| getter(f)).collect(), Vec::new());
    }
    if !missing_set.is_empty() {
        add("Generate Setters".into(), missing_set.iter().map(|f| setter(f)).collect(), Vec::new());
    }
    // A constructor setting every field that isn't given its value already.
    let settable: Vec<&Field> = fields.iter().filter(|f| !(f.fixed && f.initialized)).collect();
    let types: Vec<String> = settable.iter().map(|f| f.ty.clone()).collect();
    if !settable.is_empty() && !constructors.contains(&types) {
        let params = settable.iter().map(|f| format!("{} {}", f.ty, f.name)).collect::<Vec<_>>().join(", ");
        let mut lines = vec![format!("public {name}({params}) {{")];
        lines.extend(settable.iter().map(|f| format!("{u}this.{} = {};", f.name, f.name)));
        lines.push("}".into());
        let names = settable.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ");
        add(format!("Generate Constructor ({names})"), vec![block(&indent, &lines, nl)], Vec::new());
    }
    // One with no parameters, when final fields don't need one set.
    if !constructors.is_empty()
        && !constructors.iter().any(Vec::is_empty)
        && !fields.iter().any(|f| f.fixed && !f.initialized)
    {
        add(
            "Generate Constructor with No Parameters".into(),
            vec![block(&indent, &[format!("public {name}() {{"), "}".into()], nl)],
            Vec::new(),
        );
    }
    if !methods.contains(&("toString".to_owned(), 0)) {
        let parts = fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let sep = if i == 0 { "" } else { ", " };
                let value = if f.ty.ends_with("[]") {
                    format!("java.util.Arrays.toString({})", f.name)
                } else {
                    f.name.clone()
                };
                if f.ty == "String" {
                    format!("\"{sep}{}='\" + {value} + '\\''", f.name)
                } else {
                    format!("\"{sep}{}=\" + {value}", f.name)
                }
            })
            .collect::<Vec<_>>()
            .join(" + ");
        let lines = vec![
            "@Override".into(),
            "public String toString() {".into(),
            format!("{u}return \"{name}{{\" + {parts} + '}}';"),
            "}".into(),
        ];
        add("Generate toString()".into(), vec![block(&indent, &lines, nl)], Vec::new());
    }
    if !methods.contains(&("equals".to_owned(), 1)) && !methods.contains(&("hashCode".to_owned(), 0)) {
        let compare = |f: &Field| match f.ty.as_str() {
            "float" => format!("Float.compare({0}, other.{0}) == 0", f.name),
            "double" => format!("Double.compare({0}, other.{0}) == 0", f.name),
            "int" | "long" | "short" | "byte" | "char" | "boolean" => format!("{0} == other.{0}", f.name),
            t if t.ends_with("[]") => format!("Arrays.equals({0}, other.{0})", f.name),
            _ => format!("Objects.equals({0}, other.{0})", f.name),
        };
        let all = fields.iter().map(compare).collect::<Vec<_>>().join(" && ");
        let equals = vec![
            "@Override".into(),
            "public boolean equals(Object o) {".into(),
            format!("{u}if (this == o) return true;"),
            format!("{u}if (o == null || getClass() != o.getClass()) return false;"),
            format!("{u}{name} other = ({name}) o;"),
            format!("{u}return {all};"),
            "}".into(),
        ];
        let (arrays, plain): (Vec<&Field>, Vec<&Field>) = fields.iter().partition(|f| f.ty.ends_with("[]"));
        let hashed = plain.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ");
        let mut hash = vec!["@Override".to_owned(), "public int hashCode() {".to_owned()];
        if arrays.is_empty() {
            hash.push(format!("{u}return Objects.hash({hashed});"));
        } else {
            hash.push(format!("{u}int result = Objects.hash({hashed});"));
            hash.extend(arrays.iter().map(|f| format!("{u}result = 31 * result + Arrays.hashCode({});", f.name)));
            hash.push(format!("{u}return result;"));
        }
        hash.push("}".into());
        let mut imports = Vec::new();
        if let Some(e) = import_edit(root, source, &["import_declaration"], "import java.util.Objects;", style) {
            imports.push(e);
        }
        if !arrays.is_empty()
            && let Some(mut e) = import_edit(root, source, &["import_declaration"], "import java.util.Arrays;", style)
        {
            // Both after the same line: one edit.
            match imports.first_mut() {
                Some(first) if first.start == e.start => first.text.push_str(&e.text),
                _ => imports.push(std::mem::replace(&mut e, Edit { start: 0, end: 0, text: String::new() })),
            }
        }
        add(
            "Generate equals() and hashCode()".into(),
            vec![block(&indent, &equals, nl), block(&indent, &hash, nl)],
            imports,
        );
    }
    out
}

// ----- C# ------------------------------------------------------------------------------

/// Fields, and auto-properties (`{ get; set; }`) too, which a constructor sets.
fn csharp_fields(body: Node, source: &str) -> (Vec<Field>, Vec<Field>, HashSet<String>) {
    let mut fields = Vec::new();
    let mut auto = Vec::new();
    let mut properties = HashSet::new();
    for member in named_children(body) {
        let modifiers: Vec<&str> =
            named_children(member).into_iter().filter(|c| c.kind() == "modifier").map(|m| text(m, source)).collect();
        if modifiers.iter().any(|m| matches!(*m, "static" | "const")) {
            if member.kind() == "property_declaration"
                && let Some(n) = member.child_by_field_name("name")
            {
                properties.insert(text(n, source).to_owned());
            }
            continue;
        }
        match member.kind() {
            "field_declaration" => {
                let Some(declaration) = named_children(member).into_iter().find(|c| c.kind() == "variable_declaration")
                else {
                    continue;
                };
                let ty = declaration.child_by_field_name("type").map_or("", |t| text(t, source));
                for declarator in named_children(declaration).into_iter().filter(|c| c.kind() == "variable_declarator")
                {
                    let Some(name) = declarator.child_by_field_name("name") else { continue };
                    let initialized = named_children(declarator).iter().any(|c| c.kind() == "equals_value_clause")
                        || text(declarator, source).contains('=');
                    fields.push(Field {
                        name: text(name, source).to_owned(),
                        ty: ty.to_owned(),
                        fixed: modifiers.contains(&"readonly"),
                        initialized,
                    });
                }
            }
            "property_declaration" => {
                let Some(name) = member.child_by_field_name("name") else { continue };
                properties.insert(text(name, source).to_owned());
                let accessors = member.child_by_field_name("accessors");
                let is_auto = accessors.is_some_and(|a| {
                    named_children(a)
                        .iter()
                        .all(|acc| acc.child_by_field_name("body").is_none() && !text(*acc, source).contains("=>"))
                });
                let ty = member.child_by_field_name("type").map_or("", |t| text(t, source));
                let initialized = member.child_by_field_name("value").is_some()
                    || text(member, source).trim_end().ends_with(';') && text(member, source).contains("} =");
                if is_auto {
                    auto.push(Field {
                        name: text(name, source).to_owned(),
                        ty: ty.to_owned(),
                        fixed: false,
                        initialized,
                    });
                }
            }
            _ => {}
        }
    }
    (fields, auto, properties)
}

/// A field's property name: `_name` and `m_name` give `Name`.
fn property_name(field: &str) -> String {
    capitalized(field.strip_prefix("m_").or_else(|| field.strip_prefix('_')).unwrap_or(field))
}

const CS_VALUE_TYPES: &[&str] = &[
    "int", "long", "short", "byte", "sbyte", "uint", "ulong", "ushort", "char", "bool", "float", "double", "decimal",
    "string", "DateTime", "Guid", "TimeSpan",
];

fn csharp(root: Node, source: &str, cursor: usize, style: Style) -> Vec<Action> {
    let Some((class, name, body)) = class_at(root, cursor, source, &["class_declaration", "struct_declaration"]) else {
        return Vec::new();
    };
    let (fields, auto, properties) = csharp_fields(body, source);
    if fields.is_empty() && auto.is_empty() {
        return Vec::new();
    }
    let (mut methods, mut constructors) = (HashSet::new(), Vec::new());
    for member in named_children(body) {
        let params = member.child_by_field_name("parameters").map(|p| {
            named_children(p)
                .into_iter()
                .filter(|c| c.kind() == "parameter")
                .filter_map(|c| c.child_by_field_name("type").map(|t| text(t, source).to_owned()))
                .collect::<Vec<_>>()
        });
        match member.kind() {
            "method_declaration" => {
                if let Some(n) = member.child_by_field_name("name") {
                    methods.insert((text(n, source).to_owned(), params.map_or(0, |p| p.len())));
                }
            }
            "constructor_declaration" => constructors.push(params.unwrap_or_default()),
            _ => {}
        }
    }
    let (at, indent, alone) = insertion(body, class, source, style);
    let (u, nl) = (style.unit, style.nl);
    let mut out = Vec::new();
    let mut add = |title: String, members: Vec<String>, extra: Vec<Edit>| {
        let mut edits = extra;
        edits.push(members_edit(at, alone, &members, style));
        out.push(Action { title, kind: Kind::Generate, edits });
    };
    // Properties for fields that have none.
    let unexposed: Vec<&Field> = fields
        .iter()
        .filter(|f| {
            let p = property_name(&f.name);
            p != f.name && !properties.contains(&p)
        })
        .collect();
    if !unexposed.is_empty() {
        let members = unexposed
            .iter()
            .map(|f| {
                let p = property_name(&f.name);
                let line = if f.fixed {
                    format!("public {} {p} => {};", f.ty, f.name)
                } else {
                    format!("public {} {p} {{ get => {}; set => {} = value; }}", f.ty, f.name, f.name)
                };
                block(&indent, &[line], nl)
            })
            .collect();
        add("Generate Properties".into(), members, Vec::new());
    }
    // A constructor for fields and auto-properties not given their values already.
    let settable: Vec<&Field> = fields.iter().chain(auto.iter()).filter(|f| !f.initialized).collect();
    let types: Vec<String> = settable.iter().map(|f| f.ty.clone()).collect();
    if !settable.is_empty() && !constructors.contains(&types) {
        let param =
            |f: &Field| lowered(f.name.strip_prefix("m_").or_else(|| f.name.strip_prefix('_')).unwrap_or(&f.name));
        let params = settable.iter().map(|f| format!("{} {}", f.ty, param(f))).collect::<Vec<_>>().join(", ");
        let mut lines = vec![format!("public {name}({params})"), "{".into()];
        for f in &settable {
            let p = param(f);
            if p == f.name {
                lines.push(format!("{u}this.{} = {p};", f.name));
            } else {
                lines.push(format!("{u}{} = {p};", f.name));
            }
        }
        lines.push("}".into());
        let names = settable.iter().map(|f| param(f)).collect::<Vec<_>>().join(", ");
        add(format!("Generate Constructor ({names})"), vec![block(&indent, &lines, nl)], Vec::new());
    }
    let shown: Vec<(String, String)> = fields
        .iter()
        .map(|f| (property_name(&f.name), f.name.clone()))
        .chain(auto.iter().map(|f| (f.name.clone(), f.name.clone())))
        .collect();
    if !methods.contains(&("ToString".to_owned(), 0)) {
        let parts = shown.iter().map(|(label, value)| format!("{label} = {{{value}}}")).collect::<Vec<_>>().join(", ");
        let line = format!("public override string ToString() => $\"{name} {{{{ {parts} }}}}\";");
        add("Generate ToString()".into(), vec![block(&indent, &[line], nl)], Vec::new());
    }
    if !methods.contains(&("Equals".to_owned(), 1)) && !methods.contains(&("GetHashCode".to_owned(), 0)) {
        let all: Vec<&Field> = fields.iter().chain(auto.iter()).collect();
        let mut generic = false;
        let compare = all
            .iter()
            .map(|f| {
                if CS_VALUE_TYPES.contains(&f.ty.trim_end_matches('?')) {
                    format!("{0} == other.{0}", f.name)
                } else {
                    generic = true;
                    format!("EqualityComparer<{1}>.Default.Equals({0}, other.{0})", f.name, f.ty)
                }
            })
            .collect::<Vec<_>>();
        let object = if style.nullable { "object?" } else { "object" };
        let equals =
            format!("public override bool Equals({object} obj) => obj is {name} other && {};", compare.join(" && "));
        let names: Vec<&str> = all.iter().map(|f| f.name.as_str()).collect();
        let hash = if names.len() <= 8 {
            vec![format!("public override int GetHashCode() => HashCode.Combine({});", names.join(", "))]
        } else {
            let mut lines = vec![
                "public override int GetHashCode()".to_owned(),
                "{".into(),
                format!("{u}var hash = new HashCode();"),
            ];
            lines.extend(names.iter().map(|n| format!("{u}hash.Add({n});")));
            lines.push(format!("{u}return hash.ToHashCode();"));
            lines.push("}".into());
            lines
        };
        let mut imports = Vec::new();
        let usings = ["using_directive"];
        let has = |line: &str| source.lines().any(|l| l.trim() == line);
        let mut wanted = vec!["using System;"];
        if generic {
            wanted.push("using System.Collections.Generic;");
        }
        let lines: Vec<&str> = wanted.into_iter().filter(|w| !has(w)).collect();
        if !lines.is_empty()
            && let Some(mut e) = import_edit(root, source, &usings, lines[0], style)
        {
            for more in &lines[1..] {
                if e.start == 0 {
                    e.text =
                        format!("{}{more}{nl}{nl}", e.text.trim_end_matches(nl).trim_end_matches(nl).to_owned() + nl);
                } else {
                    e.text.push_str(&format!("{nl}{more}"));
                }
            }
            imports.push(e);
        }
        add(
            "Generate Equals() and GetHashCode()".into(),
            vec![block(&indent, &[equals], nl), block(&indent, &hash, nl)],
            imports,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: Style = Style { unit: "    ", nl: "\n", nullable: false };

    fn apply(source: &str, action: &Action) -> String {
        let mut out = source.to_owned();
        let mut edits = action.edits.clone();
        edits.sort_by_key(|e| std::cmp::Reverse(e.start));
        for e in edits {
            out.replace_range(e.start..e.end, &e.text);
        }
        out
    }

    fn titles(actions: &[Action]) -> Vec<&str> {
        actions.iter().map(|a| a.title.as_str()).collect()
    }

    const PERSON: &str = "package shop;\n\nimport java.util.List;\n\npublic class Person {\n    private final String name;\n    private int age;\n    private boolean active;\n    private static int count = 0;\n\n    public Person(String name) {\n        this.name = name;\n    }\n\n    public String getName() {\n        return name;\n    }\n}\n";

    #[test]
    fn java_members_as_vscode_writes_them() {
        let at = PERSON.find("private int").unwrap();
        let found = actions("java", PERSON, at, STYLE);
        assert_eq!(
            titles(&found),
            [
                "Generate Getters and Setters",
                "Generate Getters",
                "Generate Setters",
                "Generate Constructor (name, age, active)",
                "Generate toString()",
                "Generate equals() and hashCode()",
            ]
        );
        let both = apply(PERSON, &found[0]);
        assert!(both.contains("        return name;\n    }\n\n    public int getAge() {\n        return age;\n    }\n\n    public void setAge(int age) {\n        this.age = age;\n    }\n\n    public boolean isActive() {\n        return active;\n    }\n\n    public void setActive(boolean active) {\n        this.active = active;\n    }\n}\n"), "{both}");
        assert!(!both.contains("setName"), "final: no setter");
        assert!(!both.contains("getCount"), "static: left out");
        let ctor = apply(PERSON, &found[3]);
        assert!(ctor.contains("    public Person(String name, int age, boolean active) {\n        this.name = name;\n        this.age = age;\n        this.active = active;\n    }\n}\n"));
        let s = apply(PERSON, &found[4]);
        assert!(s.contains("        return \"Person{\" + \"name='\" + name + '\\'' + \", age=\" + age + \", active=\" + active + '}';\n"), "{s}");
        let eq = apply(PERSON, &found[5]);
        assert!(eq.contains("import java.util.List;\nimport java.util.Objects;\n"));
        assert!(eq.contains(
            "        return Objects.equals(name, other.name) && age == other.age && active == other.active;\n"
        ));
        assert!(eq.contains("        return Objects.hash(name, age, active);\n"));
        // Once there, they aren't offered again.
        let mut done = eq;
        for title in ["Generate toString()", "Generate Getters and Setters", "Generate Constructor (name, age, active)"]
        {
            done = apply(&done, &pick(&done, title));
        }
        let left = actions("java", &done, done.find("private int").unwrap(), STYLE);
        // `name` is final and must be set: no constructor without parameters.
        assert!(left.is_empty(), "{:?}", titles(&left));
        let open = PERSON.replace("private final String name;", "private String name;");
        let left = actions("java", &open, open.find("private int").unwrap(), STYLE);
        assert!(titles(&left).contains(&"Generate Constructor with No Parameters"));
    }

    fn pick(source: &str, title: &str) -> Action {
        actions("java", source, source.find("private int").unwrap(), STYLE)
            .into_iter()
            .find(|a| a.title == title)
            .unwrap()
    }

    #[test]
    fn java_details() {
        let src = "class P {\n\tdouble w;\n\tint[] marks;\n}";
        let style = Style { unit: "\t", nl: "\n", nullable: false };
        let a = actions("java", src, 12, style);
        let eq = apply(src, a.iter().find(|a| a.title.starts_with("Generate equals")).unwrap());
        assert!(
            eq.starts_with("import java.util.Objects;\nimport java.util.Arrays;\n\nclass P {")
                || eq.starts_with("import java.util.Objects;\n\nimport java.util.Arrays;"),
            "{eq}"
        );
        assert!(eq.contains("Double.compare(w, other.w) == 0 && Arrays.equals(marks, other.marks)"));
        assert!(eq.contains("\t\tint result = Objects.hash(w);\n\t\tresult = 31 * result + Arrays.hashCode(marks);\n"));
        // `}` on the last field's line: put before it.
        let s = apply(src, a.iter().find(|a| a.title == "Generate toString()").unwrap());
        assert!(s.contains("java.util.Arrays.toString(marks)"));
        assert!(s.ends_with("\t}\n}"), "{s}");
        // Outside a class, and a class without fields: nothing.
        assert!(actions("java", "class E {}\n", 8, STYLE).is_empty());
        assert!(actions("java", "import x;\nclass E { int a; }\n", 2, STYLE).is_empty());
        // CRLF files get CRLF.
        let crlf = "class C {\r\n    int a;\r\n}\r\n";
        let a = actions("java", crlf, 12, Style { unit: "    ", nl: "\r\n", nullable: false });
        let g = apply(crlf, &a[0]);
        assert!(!g.replace("\r\n", "").contains('\n'), "{g:?}");
    }

    const CS: &str = "using System;\n\nnamespace Shop\n{\n    public class Person\n    {\n        private readonly string _name;\n        private int age;\n        public static int Count;\n        public string Nick { get; set; }\n\n        public override string ToString() => \"\";\n    }\n}\n";

    #[test]
    fn csharp_members_as_visual_studio_writes_them() {
        let a = actions("cs", CS, CS.find("private int").unwrap(), STYLE);
        assert_eq!(
            titles(&a),
            ["Generate Properties", "Generate Constructor (name, age, nick)", "Generate Equals() and GetHashCode()"]
        );
        let p = apply(CS, &a[0]);
        assert!(p.contains("        public string Name => _name;\n\n        public int Age { get => age; set => age = value; }\n    }\n}"), "{p}");
        let c = apply(CS, &a[1]);
        assert!(c.contains("        public Person(string name, int age, string nick)\n        {\n            _name = name;\n            this.age = age;\n            Nick = nick;\n        }\n"), "{c}");
        let e = apply(CS, &a[2]);
        assert!(e.contains("public override bool Equals(object obj) => obj is Person other && _name == other._name && age == other.age && Nick == other.Nick;"));
        assert!(e.contains("public override int GetHashCode() => HashCode.Combine(_name, age, Nick);"));
        assert_eq!(e.matches("using System;").count(), 1);
    }

    #[test]
    fn csharp_details() {
        let src = "class Item { Thing part; decimal price; }\n";
        let a = actions("cs", src, 14, Style { unit: "    ", nl: "\n", nullable: true });
        let t = apply(src, a.iter().find(|a| a.title == "Generate ToString()").unwrap());
        assert!(
            t.contains("public override string ToString() => $\"Item {{ Part = {part}, Price = {price} }}\";"),
            "{t}"
        );
        let e = apply(src, a.iter().find(|a| a.title.starts_with("Generate Equals")).unwrap());
        assert!(e.starts_with("using System;\nusing System.Collections.Generic;\n\nclass Item"), "{e}");
        assert!(e.contains("Equals(object? obj) => obj is Item other && EqualityComparer<Thing>.Default.Equals(part, other.part) && price == other.price;"));
    }

    #[test]
    fn c_null_checks_as_fixes() {
        let src = "#include <stdlib.h>\nvoid f(void) {\n    int *p = malloc(4);\n    *p = 1;\n}\n";
        let a = actions("c", src, src.find("*p = 1").unwrap(), STYLE);
        assert_eq!(titles(&a), ["Add a NULL check for `p` (from malloc)"]);
        assert_eq!(a[0].kind, Kind::QuickFix);
        assert_eq!(
            apply(src, &a[0]),
            "#include <stdlib.h>\nvoid f(void) {\n    int *p = malloc(4);\n    if (p == NULL) {\n        perror(\"malloc\");\n        return;\n    }\n    *p = 1;\n}\n"
        );
        // Elsewhere in the file: not offered.
        assert!(actions("c", src, 3, STYLE).is_empty());
        let crlf = src.replace('\n', "\r\n");
        let a = actions("c", &crlf, crlf.find("*p = 1").unwrap(), Style { unit: "    ", nl: "\r\n", nullable: false });
        assert!(!apply(&crlf, &a[0]).replace("\r\n", "").contains('\n'));
    }
}
