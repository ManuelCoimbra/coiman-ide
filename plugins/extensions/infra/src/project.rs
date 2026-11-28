//! The open folder's infrastructure files as a whole: which they are (Kubernetes manifests,
//! Helm charts, Terraform modules, Ansible playbooks and roles), the problems of each, found
//! with what the others say, and the graph of what refers to what.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Finding, Level, ansible, helm, k8s, terraform, yaml};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Kubernetes,
    Chart,
    Values,
    Template,
    Terraform,
    Playbook,
    Tasks,
    AnsibleVars,
    Inventory,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Kubernetes => "Kubernetes",
            Kind::Chart => "Helm chart",
            Kind::Values => "Helm values",
            Kind::Template => "Helm template",
            Kind::Terraform => "Terraform",
            Kind::Playbook => "Ansible playbook",
            Kind::Tasks => "Ansible tasks",
            Kind::AnsibleVars => "Ansible variables",
            Kind::Inventory => "Ansible inventory",
        }
    }
}

/// A node of the dependency graph.
#[derive(Clone, Debug, PartialEq)]
pub struct GNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub group: String,
    pub path: String,
    pub line: usize,
    /// Named but not found in the folder.
    pub outside: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GEdge {
    pub from: String,
    pub to: String,
    pub label: String,
}

#[derive(Debug, Default)]
pub struct Analysis {
    /// Every infrastructure file found, and what it is.
    pub files: BTreeMap<String, Kind>,
    pub findings: Vec<Finding>,
    pub objects: Vec<k8s::Object>,
    /// Terraform files by folder (one module each).
    pub modules: BTreeMap<String, Vec<terraform::File>>,
    /// Folders that hold a `Chart.yaml`.
    pub charts: Vec<String>,
    pub ansible: bool,
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn is_yaml(path: &str) -> bool {
    path.ends_with(".yaml") || path.ends_with(".yml")
}

/// The chart folders among a folder's files.
pub fn charts(all: &BTreeSet<String>) -> Vec<String> {
    all.iter().filter(|p| name_of(p) == "Chart.yaml").map(|p| dir_of(p).to_owned()).collect()
}

/// Whether a file is worth reading: YAML, `.tf`, a chart's template, an INI inventory.
pub fn candidate(path: &str, charts: &[String]) -> bool {
    if path.starts_with(".git/") || path.contains("/node_modules/") || path.contains("/.terraform/") {
        return false;
    }
    if path.starts_with(".terraform/") || path.starts_with(".github/") {
        return false;
    }
    is_yaml(path)
        || path.ends_with(".tf")
        || ansible::is_inventory(path) && !path.ends_with(".md")
        || helm::chart_of(path, charts).is_some_and(|c| helm::is_template(path, c))
}

/// Reads them all. `all` is every file of the open folder; `texts` the candidates' text.
pub fn analyse(all: &BTreeSet<String>, texts: &BTreeMap<String, String>) -> Analysis {
    let mut a = Analysis { charts: charts(all), ..Analysis::default() };
    let mut defined: BTreeSet<String> = BTreeSet::new();
    let mut ansible_docs: Vec<(String, yaml::Node, ansible::Kind)> = Vec::new();
    let mut template_uses: BTreeMap<String, Vec<helm::Use>> = BTreeMap::new();
    let mut values: BTreeMap<String, (String, yaml::Node)> = BTreeMap::new();
    let mut chart_yaml: BTreeMap<String, yaml::Node> = BTreeMap::new();

    for (path, text) in texts {
        let chart = helm::chart_of(path, &a.charts).cloned();
        if path.ends_with(".tf") {
            let f = terraform::read(path, text);
            a.findings.extend(f.problems.iter().cloned());
            a.modules.entry(dir_of(path).to_owned()).or_default().push(f);
            a.files.insert(path.clone(), Kind::Terraform);
            continue;
        }
        if let Some(c) = &chart
            && helm::is_template(path, c)
        {
            template_uses.entry(c.clone()).or_default().extend(helm::uses(path, text));
            a.files.insert(path.clone(), Kind::Template);
            continue;
        }
        if !is_yaml(path) {
            if ansible::is_inventory(path) {
                ansible::inventory_vars(text, &mut defined);
                a.files.insert(path.clone(), Kind::Inventory);
            }
            continue;
        }
        let docs = yaml::read(text);
        let Some(first) = docs.first() else { continue };
        let in_chart_root = chart.as_deref().is_some_and(|c| dir_of(path) == c);
        let kind = if in_chart_root && name_of(path) == "Chart.yaml" {
            check_chart(&mut a, &mut chart_yaml, path, &first.root);
            Some(Kind::Chart)
        } else if in_chart_root && name_of(path).starts_with("values") {
            if name_of(path) == "values.yaml" || name_of(path) == "values.yml" {
                values.insert(dir_of(path).to_owned(), (path.clone(), first.root.clone()));
            }
            Some(Kind::Values)
        } else if docs.iter().any(|d| k8s::is_manifest(&d.root)) {
            for d in docs.iter().filter(|d| k8s::is_manifest(&d.root)) {
                if let Some(o) = k8s::check(path, d, &mut a.findings) {
                    a.objects.push(o);
                }
            }
            Some(Kind::Kubernetes)
        } else if let Some(k) = ansible::kind(path, &first.root) {
            ansible_docs.push((path.clone(), first.root.clone(), k));
            Some(if k == ansible::Kind::Playbook { Kind::Playbook } else { Kind::Tasks })
        } else if ansible::is_vars_file(path) && first.root.is_map() {
            ansible::defined_in(&first.root, true, &mut defined);
            Some(Kind::AnsibleVars)
        } else if ansible::is_inventory(path)
            || matches!(name_of(path), "hosts.yml" | "hosts.yaml" | "inventory.yml" | "inventory.yaml")
        {
            ansible::yaml_inventory_vars(&first.root, &mut defined);
            Some(Kind::Inventory)
        } else {
            None
        };
        let Some(kind) = kind else { continue };
        a.files.insert(path.clone(), kind);
        // What YAML itself rejects, and keys a mapping has twice.
        if let Some(e) = yaml::syntax_error(text) {
            a.findings.push(Finding {
                path: path.clone(),
                line: e.line,
                start: e.col,
                end: e.col + 1,
                level: Level::Error,
                message: format!("YAML: {}", e.message),
            });
        }
        for d in &docs {
            for dup in &d.duplicates {
                a.findings.push(Finding {
                    path: path.clone(),
                    line: dup.line,
                    start: dup.col,
                    end: dup.col + dup.key.len(),
                    level: Level::Error,
                    message: format!("{} is here twice in the same mapping: the last one wins", dup.key),
                });
            }
        }
    }

    // Kubernetes: Services against the workloads of their folder.
    k8s::check_selectors(&a.objects, &mut a.findings);

    // Helm: the values the templates use against values.yaml.
    for c in &a.charts {
        let uses = template_uses.remove(c).unwrap_or_default();
        let Some((vpath, v)) = values.get(c) else { continue };
        helm::missing(v, &uses, &mut a.findings);
        let prefix = if c.is_empty() { "charts/".to_owned() } else { format!("{c}/charts/") };
        let sub: BTreeSet<String> = all
            .iter()
            .filter_map(|p| p.strip_prefix(&prefix))
            .filter_map(|r| r.split('/').next())
            .map(|n| match n.strip_suffix(".tgz") {
                // A packed subchart: `redis-17.0.0.tgz`.
                Some(packed) => packed.rsplit_once('-').map_or(packed, |(a, _)| a).to_owned(),
                None => n.to_owned(),
            })
            .collect();
        let sub: Vec<String> = sub.into_iter().collect();
        helm::unused(vpath, v, &uses, &helm::subcharts(chart_yaml.get(c), &sub), &mut a.findings);
    }

    // Terraform: each folder as a module.
    for files in a.modules.values() {
        terraform::check_module(files, &mut a.findings);
    }

    // Ansible: includes, roles and variables, with what every file sets.
    a.ansible = !ansible_docs.is_empty();
    for (_, root, _) in &ansible_docs {
        ansible::defined_in(root, false, &mut defined);
    }
    let exists = |p: &str| all.contains(p);
    let dir_exists = |d: &str| {
        let prefix = format!("{d}/");
        all.range(prefix.clone()..).next().is_some_and(|p| p.starts_with(&prefix))
    };
    let ctx = ansible::Ctx { exists: &exists, dir_exists: &dir_exists, defined: &defined };
    for (path, root, kind) in &ansible_docs {
        match kind {
            ansible::Kind::Playbook => ansible::check_playbook(path, root, &ctx, &mut a.findings),
            ansible::Kind::Tasks => ansible::check_tasks(path, root, &ctx, &mut a.findings),
        }
        ansible::undefined_vars(path, root, &defined, &mut a.findings);
    }
    a.findings.sort_by(|x, y| (&x.path, x.line, x.start).cmp(&(&y.path, y.line, y.start)));
    a
}

fn check_chart(a: &mut Analysis, chart_yaml: &mut BTreeMap<String, yaml::Node>, path: &str, root: &yaml::Node) {
    helm::check_chart(path, root, &mut a.findings);
    chart_yaml.insert(dir_of(path).to_owned(), root.clone());
}

/// The findings by file.
pub fn by_file(findings: &[Finding]) -> BTreeMap<String, Vec<Finding>> {
    let mut out: BTreeMap<String, Vec<Finding>> = BTreeMap::new();
    for f in findings {
        out.entry(f.path.clone()).or_default().push(f.clone());
    }
    out
}

/// The dependency graph: Kubernetes objects and Terraform blocks, by what refers to what.
pub fn graph(a: &Analysis) -> (Vec<GNode>, Vec<GEdge>) {
    let mut nodes: Vec<GNode> = Vec::new();
    let mut edges: Vec<GEdge> = Vec::new();
    for o in &a.objects {
        nodes.push(GNode {
            id: format!("k8s:{}", o.id()),
            label: o.name.clone(),
            kind: o.kind.clone(),
            group: dir_of(&o.path).to_owned(),
            path: o.path.clone(),
            line: o.line,
            outside: false,
        });
    }
    for (from, to, label) in k8s::links(&a.objects) {
        let to_id = format!("k8s:{to}");
        if !nodes.iter().any(|n| n.id == to_id) {
            let (kind, name) = to.split_once('/').unwrap_or(("", &to));
            nodes.push(GNode {
                id: to_id.clone(),
                label: name.to_owned(),
                kind: format!("{kind} (not here)"),
                group: String::new(),
                path: String::new(),
                line: 0,
                outside: true,
            });
        }
        edges.push(GEdge { from: format!("k8s:{from}"), to: to_id, label });
    }
    for (dir, files) in &a.modules {
        let id = |addr: &str| format!("tf:{dir}:{addr}");
        for b in files.iter().flat_map(|f| &f.blocks) {
            let Some(addr) = b.address() else { continue };
            if b.kind == "variable" {
                continue;
            }
            nodes.push(GNode {
                id: id(&addr),
                label: addr.clone(),
                kind: b.kind.clone(),
                group: if dir.is_empty() { "terraform".into() } else { dir.clone() },
                path: b.file.clone(),
                line: b.line,
                outside: false,
            });
        }
        for (from, to) in terraform::links(files) {
            if to.starts_with("var.") || from.starts_with("var.") {
                continue;
            }
            edges.push(GEdge { from: id(&from), to: id(&to), label: String::new() });
        }
        // A module's own folder, when it is here: its blocks hang under it.
        for b in files.iter().flat_map(|f| &f.blocks).filter(|b| b.kind == "module") {
            if let (Some(src), Some(addr)) = (&b.source, b.address())
                && (src.starts_with("./") || src.starts_with("../"))
            {
                let target = ansible::join(dir, src);
                if let Some(t) = a.modules.get(&target) {
                    for tb in t.iter().flat_map(|f| &f.blocks) {
                        if let Some(ta) = tb.address().filter(|_| tb.kind == "output") {
                            edges.push(GEdge {
                                from: id(&addr),
                                to: format!("tf:{target}:{ta}"),
                                label: "source".into(),
                            });
                        }
                    }
                }
            }
        }
    }
    (nodes, edges)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)], extra: &[&str]) -> Analysis {
        let mut all: BTreeSet<String> = files.iter().map(|(p, _)| p.to_string()).collect();
        all.extend(extra.iter().map(|p| p.to_string()));
        let charts = charts(&all);
        let texts =
            files.iter().filter(|(p, _)| candidate(p, &charts)).map(|(p, t)| (p.to_string(), t.to_string())).collect();
        analyse(&all, &texts)
    }

    fn messages(a: &Analysis) -> Vec<String> {
        a.findings.iter().map(|f| format!("{}:{} {}", f.path, f.line, f.message)).collect()
    }

    #[test]
    fn a_mixed_folder() {
        let a = project(
            &[
                (
                    "k8s/app.yaml",
                    "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  template:\n    metadata:\n      labels:\n        app: web\n    spec:\n      containers:\n        - name: web\n          image: nginx:1.25\n          resources:\n            limits: {cpu: 1}\n---\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n  name: web2\nspec:\n  selector:\n    app: api\n",
                ),
                ("chart/Chart.yaml", "apiVersion: v2\nname: demo\nversion: 0.1.0\n"),
                ("chart/values.yaml", "replicas: 1\nunused: true\n"),
                ("chart/templates/d.yaml", "replicas: {{ .Values.replicas }}\nimage: {{ .Values.image.tag }}\n"),
                ("infra/main.tf", "resource \"aws_s3_bucket\" \"b\" {\n  bucket = var.name\n}\n"),
                ("site.yml", "- hosts: all\n  tasks:\n    - ping:\n"),
                (".github/workflows/ci.yml", "on: push\n"),
                ("docker-compose.yml", "services:\n  web:\n    image: x\n"),
                ("bad.yaml", "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: x\n bad: 1\n"),
            ],
            &[],
        );
        let kinds: Vec<(&str, Kind)> = a.files.iter().map(|(p, k)| (p.as_str(), *k)).collect();
        assert_eq!(
            kinds,
            vec![
                ("bad.yaml", Kind::Kubernetes),
                ("chart/Chart.yaml", Kind::Chart),
                ("chart/templates/d.yaml", Kind::Template),
                ("chart/values.yaml", Kind::Values),
                ("infra/main.tf", Kind::Terraform),
                ("k8s/app.yaml", Kind::Kubernetes),
                ("site.yml", Kind::Playbook),
            ]
        );
        let m = messages(&a);
        let expect = [
            "bad.yaml:5 YAML:",
            "chart/templates/d.yaml:2 .Values.image.tag is not in the chart's values.yaml",
            "chart/values.yaml:2 unused is set here but no template uses .Values.unused",
            "infra/main.tf:2 var.name is not declared",
            "k8s/app.yaml:21 name is here twice in the same mapping",
            "k8s/app.yaml:23 Service web2's selector app=api matches the labels of no workload",
            "site.yml:3 This task (ping) has no name",
        ];
        for e in expect {
            assert!(m.iter().any(|x| x.starts_with(e)), "{e} not in {m:#?}");
        }
        assert_eq!(m.len(), expect.len(), "{m:#?}");
    }

    #[test]
    fn graph_of_objects_and_resources() {
        let a = project(
            &[
                (
                    "k/a.yaml",
                    "apiVersion: v1\nkind: Service\nmetadata:\n  name: s\nspec:\n  selector:\n    app: w\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: w\nspec:\n  template:\n    metadata:\n      labels:\n        app: w\n    spec:\n      volumes:\n        - name: c\n          configMap:\n            name: cfg\n      containers:\n        - name: w\n          image: w:1\n          resources:\n            limits: {cpu: 1}\n",
                ),
                (
                    "main.tf",
                    "module \"net\" {\n  source = \"./net\"\n}\nresource \"aws_instance\" \"i\" {\n  subnet_id = module.net.id\n}\n",
                ),
                ("net/out.tf", "output \"id\" {\n  value = aws_subnet.s.id\n}\nresource \"aws_subnet\" \"s\" {}\n"),
            ],
            &[],
        );
        let (nodes, edges) = graph(&a);
        let e: Vec<String> = edges.iter().map(|e| format!("{} -> {}", e.from, e.to)).collect();
        assert!(e.contains(&"k8s:Service/s -> k8s:Deployment/w".to_owned()), "{e:?}");
        assert!(e.contains(&"k8s:Deployment/w -> k8s:ConfigMap/cfg".to_owned()), "{e:?}");
        assert!(e.contains(&"tf::aws_instance.i -> tf::module.net".to_owned()), "{e:?}");
        assert!(e.contains(&"tf::module.net -> tf:net:output.id".to_owned()), "{e:?}");
        assert!(e.contains(&"tf:net:output.id -> tf:net:aws_subnet.s".to_owned()), "{e:?}");
        let cfg = nodes.iter().find(|n| n.id == "k8s:ConfigMap/cfg").unwrap();
        assert!(cfg.outside);
    }

    #[test]
    fn ansible_role_layout() {
        let a = project(
            &[
                (
                    "site.yml",
                    "- hosts: web\n  roles:\n    - web\n  tasks:\n    - name: Hi {{ greeting }} {{ http_port }} {{ secret }}\n      debug: msg=x\n",
                ),
                (
                    "roles/web/tasks/main.yml",
                    "- name: a\n  include_tasks: setup.yml\n- name: b\n  template:\n    src: x.j2\n    dest: \"/etc/{{ conf_name }}\"\n",
                ),
                ("roles/web/tasks/setup.yml", "- name: s\n  ping:\n"),
                ("roles/web/defaults/main.yml", "conf_name: web.conf\n"),
                ("group_vars/all.yml", "greeting: hello\n"),
                ("inventory/hosts", "[web]\nw1 http_port=80\n"),
            ],
            &[],
        );
        assert_eq!(
            messages(&a),
            vec![
                "site.yml:5 secret is not set in this folder's vars, defaults, group_vars or facts: the inventory or -e must give it"
            ]
        );
        assert_eq!(a.files.get("inventory/hosts"), Some(&Kind::Inventory));
        assert_eq!(a.files.get("roles/web/tasks/main.yml"), Some(&Kind::Tasks));
    }
}
