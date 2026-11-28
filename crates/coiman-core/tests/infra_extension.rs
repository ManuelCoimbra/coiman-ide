//! The Infrastructure Files extension (`plugins/extensions/infra`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host: a folder with Kubernetes manifests, a
//! Helm chart, Terraform files and an Ansible playbook; the problems the editor gets for each,
//! again while one is edited; the dependency graph; and stand-ins for helm, terraform and
//! ansible-lint on PATH.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::manifest::EventKind;
use coiman_core::extensions::plugin::{
    Block, Document, Event, Graph, Permission, PluginHost, Problem, Update, registry,
};

const ID: &str = "coiman.infra";

/// What the extension has said so far.
#[derive(Default)]
struct Seen {
    panel: String,
    problems: BTreeMap<String, Vec<Problem>>,
    graph: Option<Graph>,
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) | Block::Code(t) => out.push_str(&format!("{t}\n")),
            Block::Spans(s) => {
                out.push_str(&s.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Links(links) => {
                for l in links {
                    out.push_str(&format!("{} [{}] -> {}:{}\n", l.text, l.detail, l.path, l.line));
                }
            }
            _ => {}
        }
    }
    out
}

fn wait(host: &PluginHost, seen: &mut Seen, what: &str, until: impl Fn(&Seen) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        for u in host.poll() {
            match u {
                Update::Stopped { error, .. } => panic!("the extension stopped: {error}"),
                Update::Panel { blocks, .. } => seen.panel = text(&blocks),
                Update::Problems { path, items, .. } => {
                    seen.problems.insert(path, items);
                }
                Update::Graph { graph, .. } => seen.graph = Some(graph),
                _ => {}
            }
        }
        if until(seen) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("never saw {what}; panel:\n{}\nproblems: {:#?}", seen.panel, seen.problems);
}

fn messages(seen: &Seen, path: &str) -> Vec<String> {
    seen.problems
        .get(path)
        .map(|p| p.iter().map(|p| format!("{}: {}", p.line, p.message)).collect())
        .unwrap_or_default()
}

fn has(seen: &Seen, path: &str, part: &str) -> bool {
    messages(seen, path).iter().any(|m| m.contains(part))
}

fn write(ws: &Path, path: &str, text: &str) {
    let p = ws.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A program on PATH that prints its name, its arguments and a file of output, and exits
/// with `code`.
fn stand_in(bin: &Path, name: &str, output: &str, code: i32) {
    std::fs::write(bin.join(format!("{name}.txt")), output).unwrap();
    if cfg!(windows) {
        let script = format!("@echo off\r\necho ran {name} %*\r\ntype \"%~dp0{name}.txt\"\r\nexit /b {code}\r\n");
        std::fs::write(bin.join(format!("{name}.cmd")), script).unwrap();
    } else {
        let script = format!("#!/bin/sh\necho ran {name} \"$@\"\ncat \"$(dirname \"$0\")/{name}.txt\"\nexit {code}\n");
        std::fs::write(bin.join(name), script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

const APP: &str = "apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
spec:
  template:
    metadata:
      labels:
        app: web
    spec:
      containers:
        - name: web
          image: nginx
          envFrom:
            - configMapRef:
                name: web-config
---
apiVersion: v1
kind: Service
metadata:
  name: web
spec:
  selector:
    app: web
---
apiVersion: v1
kind: Service
metadata:
  name: api
spec:
  selector:
    app: api
---
apiVersion: extensions/v1beta1
kind: Ingress
metadata:
  name: front
  name: front2
";

const CONFIG: &str = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: web-config\ndata:\n  a: \"1\"\n";

const PLAY: &str = "- name: Web servers
  hosts: web
  task:
    - ping:
  tasks:
    - name: Install {{ package }}
      apt:
        name: \"{{ package }}\"
    - service: name=nginx state=started
    - name: More
      import_tasks: tasks/missing.yml
";

#[test]
fn infra_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-infra-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let ws = tmp.join("project");
    std::fs::create_dir_all(&ws).unwrap();
    stand_in(
        &bin,
        "helm",
        "==> Linting chart\n[INFO] Chart.yaml: icon is recommended\n\n1 chart(s) linted, 0 chart(s) failed\n",
        0,
    );
    stand_in(
        &bin,
        "terraform",
        "Error: Missing required provider\n\n  on main.tf line 1, in resource \"aws_s3_bucket\" \"logs\":\n",
        1,
    );
    stand_in(&bin, "ansible-lint", "site.yml:9: name[missing]: All tasks should be named.\n", 2);
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
        std::env::set_var("APPDATA", tmp.join("config"));
        std::env::set_var("XDG_CONFIG_HOME", tmp.join("config"));
    }

    write(&ws, "k8s/app.yaml", APP);
    write(&ws, "k8s/config.yaml", CONFIG);
    write(&ws, "chart/Chart.yaml", "apiVersion: v2\nname: demo\nversion: 0.1.0\n");
    write(&ws, "chart/values.yaml", "replicaCount: 1\nimage:\n  repository: nginx\nleftover: true\n");
    write(
        &ws,
        "chart/templates/deployment.yaml",
        "replicas: {{ .Values.replicaCount }}\nimage: {{ .Values.image.repository }}:{{ .Values.image.tag }}\n",
    );
    write(
        &ws,
        "tf/main.tf",
        "resource \"aws_s3_bucket\" \"logs\" {\n  bucket = var.bucket_name\n}\n\nresource \"aws_s3_bucket_policy\" \"p\" {\n  bucket = aws_s3_bucket.logs.id\n  policy = data.aws_iam_policy_document.p.json\n}\n",
    );
    write(&ws, "tf/vars.tf", "variable \"region\" {}\n\nlocals {\n  x = {\n");
    write(&ws, "site.yml", PLAY);
    write(&ws, "notes.yaml", "just: some yaml\nnot: infra\n");

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/infra");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::EditorRead].into();
    let programs = ["helm".to_owned(), "terraform".to_owned(), "ansible-lint".to_owned()].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, programs);

    // Opening a manifest reads the folder; the problems come per file.
    host.event(EventKind::Opened, Event::Opened("k8s/app.yaml".into()));
    let mut seen = Seen::default();
    wait(&host, &mut seen, "every file's problems", |s| {
        ["k8s/app.yaml", "chart/templates/deployment.yaml", "chart/values.yaml", "tf/main.tf", "tf/vars.tf", "site.yml"]
            .iter()
            .all(|p| s.problems.contains_key(*p))
    });
    // Kubernetes.
    assert!(has(&seen, "k8s/app.yaml", "13: Image nginx has no tag"), "{:?}", messages(&seen, "k8s/app.yaml"));
    assert!(has(&seen, "k8s/app.yaml", "12: web has no resources.limits"));
    assert!(has(&seen, "k8s/app.yaml", "31: Service api's selector app=api matches the labels of no workload"));
    assert!(has(&seen, "k8s/app.yaml", "34: Ingress is served as networking.k8s.io/v1, not extensions/v1beta1"));
    assert!(has(&seen, "k8s/app.yaml", "38: name is here twice"));
    assert!(!has(&seen, "k8s/app.yaml", "Service web's"), "web's selector matches the Deployment");
    assert!(!seen.problems.contains_key("k8s/config.yaml") || messages(&seen, "k8s/config.yaml").is_empty());
    assert!(!seen.problems.contains_key("notes.yaml"));
    // Helm.
    assert_eq!(
        messages(&seen, "chart/templates/deployment.yaml"),
        vec!["2: .Values.image.tag is not in the chart's values.yaml: it is empty unless set at install"]
    );
    assert_eq!(
        messages(&seen, "chart/values.yaml"),
        vec!["4: leftover is set here but no template uses .Values.leftover"]
    );
    // Terraform: the folder read as one module.
    let tf = messages(&seen, "tf/main.tf");
    assert!(
        tf.contains(&"2: var.bucket_name is not declared: no variable by that name in this folder".into()),
        "{tf:?}"
    );
    assert!(tf.iter().any(|m| m.starts_with("7: data.aws_iam_policy_document.p is not declared")), "{tf:?}");
    assert!(!tf.iter().any(|m| m.contains("aws_s3_bucket.logs is not")), "{tf:?}");
    let vars = messages(&seen, "tf/vars.tf");
    assert!(
        vars.contains(&"1: Variable region is declared but nothing in this folder uses var.region".into()),
        "{vars:?}"
    );
    assert!(vars.iter().any(|m| m.starts_with("3: This block is not closed")), "{vars:?}");
    assert!(vars.iter().any(|m| m.starts_with("4: This block is not closed")), "{vars:?}");
    // Ansible.
    let play = messages(&seen, "site.yml");
    assert!(play.contains(&"3: A play has no key task (did you mean tasks?)".into()), "{play:?}");
    assert!(
        play.contains(&"9: This task (service) has no name: the run's output shows only its module".into()),
        "{play:?}"
    );
    assert!(play.contains(&"11: Tasks file tasks/missing.yml is not there".into()), "{play:?}");
    assert!(play.iter().any(|m| m.starts_with("6: package is not set")), "{play:?}");

    // The panel lists the files by kind, with their problems linked.
    wait(&host, &mut seen, "the panel", |s| s.panel.contains("Terraform (2)"));
    let p = &seen.panel;
    for part in ["Kubernetes (2)", "Helm (3)", "Ansible (1)", "k8s/config.yaml [Kubernetes · no problems]"] {
        assert!(p.contains(part), "{part} not in\n{p}");
    }
    assert!(
        p.contains("13: Image nginx has no tag, so it is :latest: pin a version [warning] -> k8s/app.yaml:13"),
        "{p}"
    );
    assert!(!p.contains("notes.yaml"), "{p}");

    // Editing: the unsaved text is checked, and fixed problems go away.
    let fixed = APP.replace(
        "image: nginx\n",
        "image: nginx:1.25\n          resources:\n            limits:\n              cpu: 1\n",
    );
    host.set_document(Some(Document {
        path: "k8s/app.yaml".into(),
        language: "yaml".into(),
        text: fixed,
        line: 1,
        column: 1,
        dirty: true,
        selection: String::new(),
    }));
    host.event(EventKind::Edited, Event::Edited("k8s/app.yaml".into()));
    wait(&host, &mut seen, "the edit checked", |s| !has(s, "k8s/app.yaml", "has no tag"));
    assert!(!has(&seen, "k8s/app.yaml", "resources.limits"), "{:?}", messages(&seen, "k8s/app.yaml"));
    assert!(has(&seen, "k8s/app.yaml", "Service api's selector"));

    // The dependency graph.
    host.run_command(ID, "graph", "");
    wait(&host, &mut seen, "the graph", |s| s.graph.is_some());
    let g = seen.graph.as_ref().unwrap();
    let edges: Vec<String> = g.edges.iter().map(|e| format!("{} -> {}", e.source, e.target)).collect();
    for e in [
        "k8s:Service/web -> k8s:Deployment/web",
        "k8s:Deployment/web -> k8s:ConfigMap/web-config",
        "tf:tf:aws_s3_bucket_policy.p -> tf:tf:aws_s3_bucket.logs",
    ] {
        assert!(edges.contains(&e.to_owned()), "{e} not in {edges:?}");
    }
    let cm = g.nodes.iter().find(|n| n.id == "k8s:ConfigMap/web-config").unwrap();
    assert_eq!((cm.path.as_str(), cm.line), ("k8s/config.yaml", 2));

    // The tools, run by the stand-ins.
    host.run_command(ID, "tools", "");
    wait(&host, &mut seen, "the tools", |s| !s.panel.contains("Running…") && s.panel.contains("ran ansible-lint"));
    let p = &seen.panel;
    for part in [
        "helm lint chart\nPassed\n",
        "ran helm lint chart",
        "ran helm template chart",
        "ran terraform fmt -check -recursive -no-color",
        "-chdir=tf",
        "validate -no-color",
        "terraform validate needs the folder initialised",
        "-> tf/main.tf:1",
        "ran ansible-lint -p --nocolor",
        "site.yml:9: name[missing]: All tasks should be named. [site.yml:9] -> site.yml:9",
        "Exit code 2",
    ] {
        assert!(p.contains(part), "{part:?} not in\n{p}");
    }
    assert!(!p.contains(" init") || p.contains("run terraform init yourself"), "{p}");
    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
