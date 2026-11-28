//! Ansible playbooks and roles, read offline: plays with keys Ansible doesn't know, tasks with
//! no name, includes and imports of files that aren't there, and variables used that no vars,
//! defaults, group_vars, register or set_fact sets (a hint: the inventory or the command line
//! may set them).

use std::collections::BTreeSet;

use crate::yaml::{Node, Value};
use crate::{Finding, Level};

/// The keys a play may have.
const PLAY_KEYS: &[&str] = &[
    "name",
    "hosts",
    "gather_facts",
    "gather_subset",
    "gather_timeout",
    "fact_path",
    "become",
    "become_user",
    "become_method",
    "become_flags",
    "become_exe",
    "vars",
    "vars_files",
    "vars_prompt",
    "tasks",
    "pre_tasks",
    "post_tasks",
    "handlers",
    "roles",
    "connection",
    "remote_user",
    "port",
    "serial",
    "strategy",
    "tags",
    "environment",
    "any_errors_fatal",
    "max_fail_percentage",
    "ignore_errors",
    "ignore_unreachable",
    "check_mode",
    "diff",
    "collections",
    "module_defaults",
    "order",
    "run_once",
    "throttle",
    "timeout",
    "force_handlers",
    "no_log",
    "debugger",
    "import_playbook",
    "ansible.builtin.import_playbook",
    "when",
    "vars_prompt",
];

const INCLUDE_TASKS: &[&str] = &[
    "include",
    "include_tasks",
    "import_tasks",
    "ansible.builtin.include",
    "ansible.builtin.include_tasks",
    "ansible.builtin.import_tasks",
];
const INCLUDE_VARS: &[&str] = &["include_vars", "ansible.builtin.include_vars"];
const INCLUDE_ROLE: &[&str] =
    &["include_role", "import_role", "ansible.builtin.include_role", "ansible.builtin.import_role"];
const SET_FACT: &[&str] = &["set_fact", "ansible.builtin.set_fact"];

/// Names Ansible and Jinja give without anyone setting them.
const MAGIC: &[&str] = &[
    "item",
    "inventory_hostname",
    "inventory_hostname_short",
    "hostvars",
    "groups",
    "group_names",
    "play_hosts",
    "omit",
    "playbook_dir",
    "role_path",
    "role_name",
    "inventory_dir",
    "inventory_file",
    "lookup",
    "query",
    "q",
    "range",
    "now",
    "undef",
    "dict",
    "lipsum",
    "cycler",
    "joiner",
    "namespace",
    "loop",
    "true",
    "false",
    "none",
    "True",
    "False",
    "None",
    "and",
    "or",
    "not",
    "is",
    "in",
    "if",
    "else",
    "defined",
    "undefined",
    "environment",
    "vars",
    "ansible_loop",
    "role_names",
    "playbook",
    "zip",
    "self",
];

/// What kind of Ansible file a YAML document is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Playbook,
    /// A role's (or a loose) list of tasks or handlers.
    Tasks,
}

fn is_play(n: &Node) -> bool {
    n.get("hosts").is_some() || n.get("import_playbook").is_some() || n.get("ansible.builtin.import_playbook").is_some()
}

pub fn kind(path: &str, root: &Node) -> Option<Kind> {
    let items = root.items();
    if items.is_empty() {
        return None;
    }
    if items.iter().any(is_play) {
        return Some(Kind::Playbook);
    }
    let in_role = ["tasks/", "handlers/"].iter().any(|d| path.starts_with(d) || path.contains(&format!("/{d}")));
    (in_role && items.iter().all(Node::is_map)).then_some(Kind::Tasks)
}

/// Where things a file names are looked for: a path relative to the open folder.
pub fn join(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    for p in rel.split(['/', '\\']) {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

pub fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// The role folder a file is in (`roles/web` for `roles/web/tasks/main.yml`).
fn role_dir(path: &str) -> Option<&str> {
    for d in ["/tasks/", "/handlers/", "/vars/", "/defaults/", "/meta/"] {
        if let Some(i) = path.find(d) {
            return Some(&path[..i]);
        }
    }
    None
}

/// What the checks need to know about the folder.
pub struct Ctx<'a> {
    /// Whether a file (relative path) is there.
    pub exists: &'a dyn Fn(&str) -> bool,
    /// Whether a folder (relative path) is there.
    pub dir_exists: &'a dyn Fn(&str) -> bool,
    /// Every variable set anywhere in the folder.
    pub defined: &'a BTreeSet<String>,
}

fn finding(path: &str, line: usize, col: usize, len: usize, level: Level, message: String) -> Finding {
    Finding { path: path.to_owned(), line, start: col, end: col + len.max(1), level, message }
}

/// The file a task's include names: its value, or its `file:`.
fn named_file(n: &Node) -> Option<&Node> {
    match &n.value {
        Value::Scalar(_) => Some(n),
        Value::Map(_) => n.get("file").or_else(|| n.get("_raw_params")),
        _ => None,
    }
}

fn check_file_ref(path: &str, n: &Node, candidates: &[String], what: &str, ctx: &Ctx, out: &mut Vec<Finding>) {
    let Some(text) = n.str() else { return };
    if text.contains("{{") || candidates.iter().any(|c| (ctx.exists)(c)) {
        return;
    }
    out.push(finding(path, n.line, n.col, text.len(), Level::Error, format!("{what} {text} is not there")));
}

fn check_role(path: &str, n: &Node, play_dir: &str, ctx: &Ctx, out: &mut Vec<Finding>) {
    let name_node = match &n.value {
        Value::Map(_) => n.get("role").or_else(|| n.get("name")),
        _ => Some(n),
    };
    let Some(name_node) = name_node else { return };
    let Some(name) = name_node.str() else { return };
    // A role from a collection or Galaxy (`ns.coll.role`, `user.role`) is installed elsewhere.
    if name.contains("{{") || name.contains('.') {
        return;
    }
    let mut dirs = vec![join(play_dir, &format!("roles/{name}")), format!("roles/{name}")];
    if let Some(r) = role_dir(path) {
        // A role's role: beside it.
        dirs.push(join(dir_of(r), name));
    }
    if dirs.iter().any(|d| (ctx.dir_exists)(d)) || name.contains('/') && (ctx.dir_exists)(&join(play_dir, name)) {
        return;
    }
    out.push(finding(
        path,
        name_node.line,
        name_node.col,
        name.len(),
        Level::Error,
        format!("Role {name} is not there: no roles/{name} folder beside the playbook"),
    ));
}

/// Checks a list of tasks (with blocks inside).
fn check_tasks_list(path: &str, items: &[Node], ctx: &Ctx, out: &mut Vec<Finding>) {
    let base = dir_of(path);
    for t in items {
        if !t.is_map() {
            continue;
        }
        let nested: Vec<&Node> = ["block", "rescue", "always"].iter().filter_map(|k| t.get(k)).collect();
        if !nested.is_empty() {
            for n in nested {
                check_tasks_list(path, n.items(), ctx, out);
            }
        } else if t.get("name").is_none() {
            let first =
                t.entries().first().map(|e| (e.line, e.col, e.key.clone())).unwrap_or((t.line, t.col, String::new()));
            out.push(finding(
                path,
                first.0,
                first.1,
                first.2.len(),
                Level::Warning,
                format!("This task ({}) has no name: the run's output shows only its module", first.2),
            ));
        }
        for e in t.entries() {
            if INCLUDE_TASKS.contains(&e.key.as_str())
                && let Some(n) = named_file(&e.value)
            {
                let mut c = vec![join(base, n.str().unwrap_or_default())];
                if let Some(r) = role_dir(path) {
                    c.push(join(&format!("{r}/tasks"), n.str().unwrap_or_default()));
                }
                check_file_ref(path, n, &c, "Tasks file", ctx, out);
            }
            if INCLUDE_VARS.contains(&e.key.as_str())
                && let Some(n) = named_file(&e.value)
            {
                let f = n.str().unwrap_or_default();
                let mut c = vec![join(base, f), join(base, &format!("vars/{f}"))];
                if let Some(r) = role_dir(path) {
                    c.push(join(&format!("{r}/vars"), f));
                    c.push(join(r, f));
                }
                check_file_ref(path, n, &c, "Vars file", ctx, out);
            }
            if INCLUDE_ROLE.contains(&e.key.as_str()) {
                check_role(path, &e.value, base, ctx, out);
            }
        }
    }
}

/// Checks a playbook.
pub fn check_playbook(path: &str, root: &Node, ctx: &Ctx, out: &mut Vec<Finding>) {
    let base = dir_of(path);
    for play in root.items() {
        for e in play.entries() {
            if !PLAY_KEYS.contains(&e.key.as_str()) {
                let hint = if e.key == "task" || e.key == "handler" || e.key == "role" {
                    format!(" (did you mean {}s?)", e.key)
                } else {
                    String::new()
                };
                out.push(finding(
                    path,
                    e.line,
                    e.col,
                    e.key.len(),
                    Level::Error,
                    format!("A play has no key {}{hint}", e.key),
                ));
            }
        }
        for k in ["import_playbook", "ansible.builtin.import_playbook"] {
            if let Some(n) = play.get(k) {
                check_file_ref(path, n, &[join(base, n.str().unwrap_or_default())], "Playbook", ctx, out);
            }
        }
        for f in play.get("vars_files").map(Node::items).unwrap_or_default() {
            check_file_ref(path, f, &[join(base, f.str().unwrap_or_default())], "Vars file", ctx, out);
        }
        for r in play.get("roles").map(Node::items).unwrap_or_default() {
            check_role(path, r, base, ctx, out);
        }
        for k in ["pre_tasks", "tasks", "post_tasks", "handlers"] {
            if let Some(t) = play.get(k) {
                check_tasks_list(path, t.items(), ctx, out);
            }
        }
    }
}

/// Checks a role's tasks or handlers file.
pub fn check_tasks(path: &str, root: &Node, ctx: &Ctx, out: &mut Vec<Finding>) {
    check_tasks_list(path, root.items(), ctx, out);
}

// ----- variables -----------------------------------------------------------------------------

/// The variables a Jinja expression reads: not attributes, filters, tests, functions, keywords
/// or what's in quotes.
pub fn expr_names(expr: &str) -> Vec<(String, usize)> {
    let chars: Vec<char> = expr.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut prev_sig: Option<char> = None;
    let mut prev_word = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != c {
                i += 1;
            }
            i += 1;
            prev_sig = Some(c);
            prev_word.clear();
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let next = chars.get(j).copied();
            let skip = matches!(prev_sig, Some('.' | '|'))
                || prev_word == "is"
                || prev_word == "not" && matches!(prev_sig, Some('i'))
                || next == Some('(')
                || next == Some('=') && chars.get(j + 1) != Some(&'=')
                || MAGIC.contains(&word.as_str())
                || word.starts_with("ansible_");
            if !skip && !out.iter().any(|(w, _)| *w == word) {
                out.push((word.clone(), start));
            }
            // `is not defined`: what follows `not` after `is` is a test too.
            prev_sig = if word == "is" || (word == "not" && prev_word == "is") { Some('i') } else { Some('w') };
            prev_word = word;
            continue;
        }
        if c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '.') {
                i += 1;
            }
            prev_sig = Some('0');
            continue;
        }
        if !c.is_whitespace() {
            prev_sig = Some(c);
            prev_word.clear();
        }
        i += 1;
    }
    out
}

/// The `{{ }}` expressions of a text, with their offsets.
fn templates(text: &str) -> Vec<(&str, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("{{") {
        let start = from + i + 2;
        let Some(end) = text[start..].find("}}") else { break };
        out.push((&text[start..start + end], start));
        from = start + end + 2;
    }
    out
}

/// Keys whose values are bare expressions.
const EXPR_KEYS: &[&str] = &["when", "failed_when", "changed_when", "until"];

/// Variables a play or task list reads that `defined` lacks.
pub fn undefined_vars(path: &str, root: &Node, defined: &BTreeSet<String>, out: &mut Vec<Finding>) {
    // Names a loop or a block sets for what's inside: `loop_control.loop_var`.
    fn walk(path: &str, n: &Node, key: &str, defined: &BTreeSet<String>, out: &mut Vec<Finding>) {
        match &n.value {
            Value::Scalar(s) => {
                let mut found: Vec<(String, usize)> = Vec::new();
                if EXPR_KEYS.contains(&key) && !s.contains("{{") {
                    found = expr_names(s);
                } else {
                    for (expr, at) in templates(s) {
                        found.extend(expr_names(expr).into_iter().map(|(w, c)| (w, c + at)));
                    }
                }
                for (w, c) in found {
                    if defined.contains(&w)
                        || out.iter().any(|f| f.path == path && f.message.starts_with(&format!("{w} ")))
                    {
                        continue;
                    }
                    // A plain one-line value: its column is known; else the line's start.
                    let col = if s.contains('\n') { n.col } else { n.col + c };
                    out.push(finding(
                        path,
                        n.line,
                        col,
                        w.len(),
                        Level::Hint,
                        format!("{w} is not set in this folder's vars, defaults, group_vars or facts: the inventory or -e must give it"),
                    ));
                }
            }
            Value::Map(entries) => {
                for e in entries {
                    walk(path, &e.value, &e.key, defined, out);
                }
            }
            Value::Seq(items) => {
                for i in items {
                    walk(path, i, key, defined, out);
                }
            }
            Value::Null => {}
        }
    }
    walk(path, root, "", defined, out);
}

/// Every name a play, a task list or a vars file sets.
pub fn defined_in(root: &Node, vars_file: bool, out: &mut BTreeSet<String>) {
    if vars_file {
        out.extend(root.entries().iter().map(|e| e.key.clone()));
        return;
    }
    fn walk(n: &Node, out: &mut BTreeSet<String>) {
        match &n.value {
            Value::Map(entries) => {
                for e in entries {
                    match e.key.as_str() {
                        "vars" | "extra_vars" => out.extend(e.value.entries().iter().map(|v| v.key.clone())),
                        "register" | "loop_var" | "index_var" => {
                            if let Some(s) = e.value.str() {
                                out.insert(s.to_owned());
                            }
                        }
                        "vars_prompt" => {
                            for p in e.value.items() {
                                if let Some(s) = p.get("name").and_then(Node::str) {
                                    out.insert(s.to_owned());
                                }
                            }
                        }
                        k if SET_FACT.contains(&k) => {
                            out.extend(e.value.entries().iter().map(|v| v.key.clone()).filter(|k| k != "cacheable"))
                        }
                        _ => {}
                    }
                    walk(&e.value, out);
                }
            }
            Value::Seq(items) => items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    walk(root, out);
}

/// Whether a YAML file holds variables for plays: group_vars, host_vars, a role's defaults or
/// vars.
pub fn is_vars_file(path: &str) -> bool {
    let p = format!("/{path}");
    ["/group_vars/", "/host_vars/", "/defaults/", "/vars/"].iter().any(|d| p.contains(d))
}

/// Whether a file is an INI inventory.
pub fn is_inventory(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(name, "hosts" | "inventory" | "hosts.ini" | "inventory.ini")
        || (path.starts_with("inventory/") || path.contains("/inventory/") || path.contains("inventories/"))
            && !name.ends_with(".yml")
            && !name.ends_with(".yaml")
}

/// The variables an INI inventory sets: `key=value` on host lines and in `[group:vars]`.
pub fn inventory_vars(text: &str, out: &mut BTreeSet<String>) {
    let mut in_vars = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('#') || l.starts_with(';') || l.is_empty() {
            continue;
        }
        if l.starts_with('[') {
            in_vars = l.ends_with(":vars]");
            continue;
        }
        if in_vars {
            if let Some((k, _)) = l.split_once('=') {
                out.insert(k.trim().to_owned());
            }
        } else {
            for tok in l.split_whitespace().skip(1) {
                if let Some((k, _)) = tok.split_once('=') {
                    out.insert(k.to_owned());
                }
            }
        }
    }
}

/// The variables a YAML inventory sets: the keys of every `vars:` and of every host.
pub fn yaml_inventory_vars(root: &Node, out: &mut BTreeSet<String>) {
    fn walk(n: &Node, key: &str, out: &mut BTreeSet<String>) {
        for e in n.entries() {
            if key == "vars" || key == "hosts" && e.value.is_map() {
                if key == "vars" {
                    out.insert(e.key.clone());
                } else {
                    out.extend(e.value.entries().iter().map(|v| v.key.clone()));
                }
            }
            walk(&e.value, &e.key, out);
        }
    }
    walk(root, "", out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml;

    fn root(text: &str) -> Node {
        yaml::read(text).remove(0).root
    }

    fn messages(f: &[Finding]) -> Vec<String> {
        f.iter().map(|f| format!("{}: {}", f.line, f.message)).collect()
    }

    const PLAY: &str = "- name: Web\n  hosts: web\n  become: true\n  task:\n    - x\n  vars_files:\n    - vars/common.yml\n    - vars/missing.yml\n  roles:\n    - common\n    - nowhere\n    - geerlingguy.docker\n  tasks:\n    - name: Install\n      apt:\n        name: nginx\n    - service: name=nginx state=started\n    - include_tasks: extra.yml\n    - import_tasks: missing.yml\n    - block:\n        - debug: msg=hi\n      rescue:\n        - name: Fail\n          fail:\n- import_playbook: other.yml\n";

    #[test]
    fn kinds() {
        assert_eq!(kind("site.yml", &root(PLAY)), Some(Kind::Playbook));
        assert_eq!(kind("roles/web/tasks/main.yml", &root("- name: a\n  ping:\n")), Some(Kind::Tasks));
        assert_eq!(kind("list.yml", &root("- a\n- b\n")), None);
        assert_eq!(kind("k8s.yaml", &root("apiVersion: v1\n")), None);
    }

    #[test]
    fn playbook_problems() {
        let files = ["vars/common.yml", "extra.yml", "other.yml"];
        let exists = |p: &str| files.contains(&p);
        let dir_exists = |p: &str| p == "roles/common";
        let defined = BTreeSet::new();
        let ctx = Ctx { exists: &exists, dir_exists: &dir_exists, defined: &defined };
        let mut out = Vec::new();
        check_playbook("site.yml", &root(PLAY), &ctx, &mut out);
        assert_eq!(
            messages(&out),
            vec![
                "4: A play has no key task (did you mean tasks?)",
                "8: Vars file vars/missing.yml is not there",
                "11: Role nowhere is not there: no roles/nowhere folder beside the playbook",
                "17: This task (service) has no name: the run's output shows only its module",
                "18: This task (include_tasks) has no name: the run's output shows only its module",
                "19: This task (import_tasks) has no name: the run's output shows only its module",
                "19: Tasks file missing.yml is not there",
                "21: This task (debug) has no name: the run's output shows only its module",
            ]
        );
    }

    #[test]
    fn role_tasks_and_includes() {
        let exists = |p: &str| p == "roles/web/tasks/install.yml" || p == "roles/web/vars/debian.yml";
        let dir_exists = |_: &str| false;
        let defined = BTreeSet::new();
        let ctx = Ctx { exists: &exists, dir_exists: &dir_exists, defined: &defined };
        let t = "- name: a\n  include_tasks: install.yml\n- name: b\n  include_vars:\n    file: debian.yml\n- name: c\n  include_tasks: \"{{ os }}.yml\"\n- name: d\n  import_tasks:\n    file: gone.yml\n";
        let mut out = Vec::new();
        check_tasks("roles/web/tasks/main.yml", &root(t), &ctx, &mut out);
        assert_eq!(messages(&out), vec!["10: Tasks file gone.yml is not there"]);
    }

    #[test]
    fn expressions() {
        let names = |e: &str| expr_names(e).into_iter().map(|(w, _)| w).collect::<Vec<_>>();
        assert_eq!(names("port | default(80) | int"), vec!["port"]);
        assert_eq!(names("user.name ~ '-' ~ suffix"), vec!["user", "suffix"]);
        assert_eq!(names("x is defined and y is not none and z == 'a b'"), vec!["x", "y", "z"]);
        assert_eq!(names("lookup('env', 'HOME') + item + ansible_os_family"), Vec::<String>::new());
        assert_eq!(names("dict(a=b)"), vec!["b"]);
        assert_eq!(names("hostvars[inventory_hostname]['x'] or groups['web'][0]"), Vec::<String>::new());
        assert_eq!(names("1.5 and 2"), Vec::<String>::new());
    }

    #[test]
    fn variables() {
        let p = root(
            "- hosts: all\n  vars:\n    port: 80\n  vars_prompt:\n    - name: pw\n  tasks:\n    - name: Get\n      command: whoami\n      register: me\n    - set_fact:\n        greeting: hi\n    - name: Show {{ port }} {{ me.stdout }} {{ who }}\n      debug:\n        msg: \"{{ greeting }} {{ pw }} {{ loopy }}\"\n      loop: \"{{ things }}\"\n      loop_control:\n        loop_var: loopy\n      when: enabled and port > 1\n",
        );
        let mut defined = BTreeSet::new();
        defined_in(&p, false, &mut defined);
        defined_in(&root("things: []\n"), true, &mut defined);
        assert!(
            ["port", "pw", "me", "greeting", "loopy", "things"].iter().all(|v| defined.contains(*v)),
            "{defined:?}"
        );
        let mut out = Vec::new();
        undefined_vars("site.yml", &p, &defined, &mut out);
        let m: Vec<String> =
            out.iter().map(|f| format!("{}:{} {}", f.line, f.start, f.message.split(' ').next().unwrap())).collect();
        assert_eq!(m, vec!["12:47 who", "18:12 enabled"]);
    }

    #[test]
    fn inventories() {
        let mut v = BTreeSet::new();
        inventory_vars("[web]\nweb1 ansible_host=1.2.3.4 http_port=8080\n\n[web:vars]\nproxy = x\n", &mut v);
        assert_eq!(v.into_iter().collect::<Vec<_>>(), vec!["ansible_host", "http_port", "proxy"]);
        let mut v = BTreeSet::new();
        yaml_inventory_vars(&root("all:\n  hosts:\n    one:\n      ip: 1\n  vars:\n    region: eu\n"), &mut v);
        assert_eq!(v.into_iter().collect::<Vec<_>>(), vec!["ip", "region"]);
        assert!(is_inventory("inventory/prod"));
        assert!(is_inventory("hosts"));
        assert!(!is_inventory("src/hosts.rs"));
        assert!(is_vars_file("group_vars/all.yml"));
        assert!(is_vars_file("roles/web/defaults/main.yml"));
        assert_eq!(join("a/b", "../c/./d.yml"), "a/c/d.yml");
    }
}
