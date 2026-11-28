//! Kubernetes manifests, read offline: each object's kind, apiVersion and name checked against
//! a table of the built-in kinds, its containers' images and limits, and the links between
//! objects of a folder (a Service's selector, ConfigMap and Secret references, an Ingress's
//! backends) for the problems and the graph.

use crate::yaml::{self, Node};
use crate::{Finding, Level};

/// The built-in kinds and the apiVersions they are served as.
const KINDS: &[(&str, &[&str])] = &[
    ("Pod", &["v1"]),
    ("Service", &["v1"]),
    ("ConfigMap", &["v1"]),
    ("Secret", &["v1"]),
    ("Namespace", &["v1"]),
    ("PersistentVolume", &["v1"]),
    ("PersistentVolumeClaim", &["v1"]),
    ("ServiceAccount", &["v1"]),
    ("Endpoints", &["v1"]),
    ("LimitRange", &["v1"]),
    ("ResourceQuota", &["v1"]),
    ("ReplicationController", &["v1"]),
    ("Node", &["v1"]),
    ("Event", &["v1", "events.k8s.io/v1"]),
    ("List", &["v1"]),
    ("Deployment", &["apps/v1"]),
    ("StatefulSet", &["apps/v1"]),
    ("DaemonSet", &["apps/v1"]),
    ("ReplicaSet", &["apps/v1"]),
    ("ControllerRevision", &["apps/v1"]),
    ("Job", &["batch/v1"]),
    ("CronJob", &["batch/v1"]),
    ("Ingress", &["networking.k8s.io/v1"]),
    ("IngressClass", &["networking.k8s.io/v1"]),
    ("NetworkPolicy", &["networking.k8s.io/v1"]),
    ("Role", &["rbac.authorization.k8s.io/v1"]),
    ("RoleBinding", &["rbac.authorization.k8s.io/v1"]),
    ("ClusterRole", &["rbac.authorization.k8s.io/v1"]),
    ("ClusterRoleBinding", &["rbac.authorization.k8s.io/v1"]),
    ("HorizontalPodAutoscaler", &["autoscaling/v2", "autoscaling/v1"]),
    ("PodDisruptionBudget", &["policy/v1"]),
    ("StorageClass", &["storage.k8s.io/v1"]),
    ("CSIDriver", &["storage.k8s.io/v1"]),
    ("VolumeAttachment", &["storage.k8s.io/v1"]),
    ("CustomResourceDefinition", &["apiextensions.k8s.io/v1"]),
    ("ValidatingWebhookConfiguration", &["admissionregistration.k8s.io/v1"]),
    ("MutatingWebhookConfiguration", &["admissionregistration.k8s.io/v1"]),
    ("PriorityClass", &["scheduling.k8s.io/v1"]),
    ("Lease", &["coordination.k8s.io/v1"]),
    ("RuntimeClass", &["node.k8s.io/v1"]),
    ("APIService", &["apiregistration.k8s.io/v1"]),
    ("CertificateSigningRequest", &["certificates.k8s.io/v1"]),
    ("Kustomization", &["kustomize.config.k8s.io/v1beta1"]),
];

/// Kinds with no `metadata.name` of their own.
const NAMELESS: &[&str] = &["Kustomization", "List"];

/// An object of a manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub path: String,
    pub kind: String,
    pub name: String,
    pub line: usize,
    /// The labels its pods get (a workload's template, a Pod's own).
    pub pod_labels: Vec<(String, String)>,
    /// A Service's selector, and its line.
    pub selector: Option<(Vec<(String, String)>, usize)>,
    /// Objects it names: (kind, name, line).
    pub refs: Vec<(String, String, usize)>,
}

impl Object {
    pub fn id(&self) -> String {
        format!("{}/{}", self.kind, self.name)
    }
}

/// Whether a YAML document is a Kubernetes object: it has `apiVersion` or `kind` at its top.
pub fn is_manifest(root: &Node) -> bool {
    root.get("apiVersion").is_some() || root.get("kind").is_some()
}

/// The paths to a workload's pod spec.
fn pod_spec<'a>(kind: &str, root: &'a Node) -> Option<&'a Node> {
    match kind {
        "Pod" => root.get("spec"),
        "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "Job" | "ReplicationController" => {
            root.at(&["spec", "template", "spec"])
        }
        "CronJob" => root.at(&["spec", "jobTemplate", "spec", "template", "spec"]),
        _ => None,
    }
}

fn pod_labels(kind: &str, root: &Node) -> Vec<(String, String)> {
    let labels = match kind {
        "Pod" => root.at(&["metadata", "labels"]),
        "CronJob" => root.at(&["spec", "jobTemplate", "spec", "template", "metadata", "labels"]),
        _ => root.at(&["spec", "template", "metadata", "labels"]),
    };
    pairs(labels)
}

fn pairs(node: Option<&Node>) -> Vec<(String, String)> {
    node.map(|n| n.entries().iter().map(|e| (e.key.clone(), e.value.str().unwrap_or_default().to_owned())).collect())
        .unwrap_or_default()
}

fn containers(spec: &Node) -> Vec<&Node> {
    ["initContainers", "containers"]
        .iter()
        .filter_map(|k| spec.get(k))
        .flat_map(|c| c.items().iter())
        .filter(|c| c.is_map())
        .collect()
}

/// The tag problem of an image, if any: none, or `latest`.
pub fn image_problem(image: &str) -> Option<&'static str> {
    if image.contains("{{") || image.contains("${") || image.is_empty() {
        return None;
    }
    if image.contains('@') {
        return None;
    }
    let last = image.rsplit('/').next().unwrap_or(image);
    match last.rsplit_once(':') {
        None => Some("has no tag, so it is :latest: pin a version"),
        Some((_, "latest")) => Some("is :latest: pin a version, so each start gets the same image"),
        Some(_) => None,
    }
}

/// Every name a pod spec gives of a ConfigMap, Secret, claim or service account.
fn spec_refs(spec: &Node, out: &mut Vec<(String, String, usize)>) {
    let mut push = |kind: &str, n: Option<&Node>| {
        if let Some(n) = n
            && let Some(name) = n.str()
        {
            out.push((kind.to_owned(), name.to_owned(), n.line));
        }
    };
    push("ServiceAccount", spec.get("serviceAccountName"));
    for v in spec.get("volumes").map(Node::items).unwrap_or_default() {
        push("ConfigMap", v.at(&["configMap", "name"]));
        push("Secret", v.at(&["secret", "secretName"]));
        push("PersistentVolumeClaim", v.at(&["persistentVolumeClaim", "claimName"]));
        for s in v.at(&["projected", "sources"]).map(Node::items).unwrap_or_default() {
            push("ConfigMap", s.at(&["configMap", "name"]));
            push("Secret", s.at(&["secret", "name"]));
        }
    }
    for s in spec.get("imagePullSecrets").map(Node::items).unwrap_or_default() {
        push("Secret", s.get("name"));
    }
    for c in containers(spec) {
        for e in c.get("envFrom").map(Node::items).unwrap_or_default() {
            push("ConfigMap", e.at(&["configMapRef", "name"]));
            push("Secret", e.at(&["secretRef", "name"]));
        }
        for e in c.get("env").map(Node::items).unwrap_or_default() {
            push("ConfigMap", e.at(&["valueFrom", "configMapKeyRef", "name"]));
            push("Secret", e.at(&["valueFrom", "secretKeyRef", "name"]));
        }
    }
}

/// What an object names outside a pod spec: an Ingress's services, an autoscaler's target, a
/// binding's role.
fn other_refs(kind: &str, root: &Node, out: &mut Vec<(String, String, usize)>) {
    let mut push = |kind: &str, n: Option<&Node>| {
        if let Some(n) = n
            && let Some(name) = n.str()
        {
            out.push((kind.to_owned(), name.to_owned(), n.line));
        }
    };
    match kind {
        "Ingress" => {
            push("Service", root.at(&["spec", "defaultBackend", "service", "name"]));
            for rule in root.at(&["spec", "rules"]).map(Node::items).unwrap_or_default() {
                for p in rule.at(&["http", "paths"]).map(Node::items).unwrap_or_default() {
                    push("Service", p.at(&["backend", "service", "name"]));
                    push("Service", p.at(&["backend", "serviceName"]));
                }
            }
            for t in root.at(&["spec", "tls"]).map(Node::items).unwrap_or_default() {
                push("Secret", t.get("secretName"));
            }
        }
        "HorizontalPodAutoscaler" | "VerticalPodAutoscaler" => {
            if let Some(t) = root.at(&["spec", "scaleTargetRef"]).or_else(|| root.at(&["spec", "targetRef"])) {
                let k = t.get("kind").and_then(Node::str).unwrap_or("Deployment").to_owned();
                push(&k, t.get("name"));
            }
        }
        "RoleBinding" | "ClusterRoleBinding" => {
            if let Some(r) = root.get("roleRef") {
                let k = r.get("kind").and_then(Node::str).unwrap_or("Role").to_owned();
                push(&k, r.get("name"));
            }
            for s in root.get("subjects").map(Node::items).unwrap_or_default() {
                if s.get("kind").and_then(Node::str) == Some("ServiceAccount") {
                    push("ServiceAccount", s.get("name"));
                }
            }
        }
        "StatefulSet" => push("Service", root.at(&["spec", "serviceName"])),
        _ => {}
    }
}

/// Checks one manifest document: what is wrong with it on its own, and the object it holds.
pub fn check(path: &str, doc: &yaml::Doc, out: &mut Vec<Finding>) -> Option<Object> {
    let root = &doc.root;
    let find = |line: usize, col: usize, len: usize, level: Level, message: String| Finding {
        path: path.to_owned(),
        line,
        start: col,
        end: col + len.max(1),
        level,
        message,
    };
    let api = root.get("apiVersion").and_then(Node::str);
    let kind = root.get("kind").and_then(Node::str);
    let top = root.line.max(1);
    if api.is_none() {
        out.push(find(top, 0, 1, Level::Error, "No apiVersion: every Kubernetes object needs one".into()));
    }
    let Some(kind) = kind else {
        out.push(find(top, 0, 1, Level::Error, "No kind: every Kubernetes object needs one".into()));
        return None;
    };
    let kind_node = root.get("kind").expect("it has a kind");
    if let (Some(api), Some((_, known))) = (api, KINDS.iter().find(|(k, _)| *k == kind))
        && !known.contains(&api)
    {
        let n = root.get("apiVersion").expect("it has an apiVersion");
        out.push(find(
            n.line,
            n.col,
            api.len(),
            Level::Error,
            format!("{kind} is served as {}, not {api}", known.join(" or ")),
        ));
    }
    let name_node = root.at(&["metadata", "name"]);
    let name = name_node.and_then(Node::str).unwrap_or_default().to_owned();
    if name_node.is_none() && root.at(&["metadata", "generateName"]).is_none() && !NAMELESS.contains(&kind) {
        let at = root.entry("metadata").map(|e| (e.line, e.col)).unwrap_or((kind_node.line, 0));
        out.push(find(at.0, at.1, 8, Level::Error, format!("No metadata.name: a {kind} needs a name")));
    }
    let mut refs = Vec::new();
    if let Some(spec) = pod_spec(kind, root) {
        for c in containers(spec) {
            let cname = c.get("name").and_then(Node::str).unwrap_or("a container");
            if let Some(img) = c.get("image")
                && let Some(text) = img.str()
                && let Some(why) = image_problem(text)
            {
                out.push(find(img.line, img.col, text.len(), Level::Warning, format!("Image {text} {why}")));
            }
            if c.at(&["resources", "limits"]).is_none() {
                let at = c.entry("name").map(|e| (e.line, e.col)).unwrap_or((c.line, c.col));
                out.push(find(
                    at.0,
                    at.1,
                    4,
                    Level::Hint,
                    format!("{cname} has no resources.limits: it may use all of a node's CPU and memory"),
                ));
            }
        }
        spec_refs(spec, &mut refs);
    }
    other_refs(kind, root, &mut refs);
    let selector = (kind == "Service" && root.at(&["spec", "type"]).and_then(Node::str) != Some("ExternalName"))
        .then(|| root.entry("spec").and_then(|s| s.value.entry("selector")))
        .flatten()
        .filter(|e| !e.value.entries().is_empty())
        .map(|e| (pairs(Some(&e.value)), e.line));
    Some(Object {
        path: path.to_owned(),
        kind: kind.to_owned(),
        name,
        line: kind_node.line,
        pod_labels: if pod_spec(kind, root).is_some() { pod_labels(kind, root) } else { Vec::new() },
        selector,
        refs,
    })
}

fn folder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn matches(selector: &[(String, String)], labels: &[(String, String)]) -> bool {
    !labels.is_empty() && selector.iter().all(|s| labels.contains(s))
}

/// The workloads a Service selects, among the objects of its own folder.
pub fn selected<'a>(service: &Object, objects: &'a [Object]) -> Vec<&'a Object> {
    let Some((sel, _)) = &service.selector else { return Vec::new() };
    objects.iter().filter(|o| folder(&o.path) == folder(&service.path) && matches(sel, &o.pod_labels)).collect()
}

/// Services of a folder whose selector matches no workload there.
pub fn check_selectors(objects: &[Object], out: &mut Vec<Finding>) {
    for s in objects.iter().filter(|o| o.selector.is_some()) {
        let has_workloads = objects.iter().any(|o| folder(&o.path) == folder(&s.path) && !o.pod_labels.is_empty());
        if !has_workloads || !selected(s, objects).is_empty() {
            continue;
        }
        let (sel, line) = s.selector.as_ref().expect("a selector");
        let text: Vec<String> = sel.iter().map(|(k, v)| format!("{k}={v}")).collect();
        out.push(Finding {
            path: s.path.clone(),
            line: *line,
            start: 0,
            end: 999,
            level: Level::Warning,
            message: format!(
                "Service {}'s selector {} matches the labels of no workload in this folder",
                s.name,
                text.join(",")
            ),
        });
    }
}

/// The links between objects, for the graph: (from, to, label). An object named but not in
/// the folders read is `kind/name` all the same; the graph shows it as outside.
pub fn links(objects: &[Object]) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for o in objects {
        for w in selected(o, objects) {
            out.push((o.id(), w.id(), "selects".to_owned()));
        }
        for (kind, name, _) in &o.refs {
            let to = objects
                .iter()
                .find(|x| &x.kind == kind && &x.name == name && folder(&x.path) == folder(&o.path))
                .or_else(|| objects.iter().find(|x| &x.kind == kind && &x.name == name))
                .map(Object::id)
                .unwrap_or_else(|| format!("{kind}/{name}"));
            let link = (o.id(), to, String::new());
            if !out.contains(&link) {
                out.push(link);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPLOY: &str = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  template:\n    metadata:\n      labels:\n        app: web\n    spec:\n      containers:\n        - name: web\n          image: nginx:1.25\n          resources:\n            limits:\n              cpu: 1\n          envFrom:\n            - configMapRef:\n                name: web-config\n        - name: side\n          image: busybox\n";

    fn run(path: &str, text: &str) -> (Vec<Finding>, Vec<Object>) {
        let mut out = Vec::new();
        let objects = yaml::read(text).iter().filter_map(|d| check(path, d, &mut out)).collect();
        (out, objects)
    }

    fn messages(f: &[Finding]) -> Vec<String> {
        let mut f = f.to_vec();
        f.sort_by_key(|f| f.line);
        f.iter().map(|f| format!("{}: {}", f.line, f.message)).collect()
    }

    #[test]
    fn a_good_deployment_and_its_problems() {
        let (f, o) = run("k8s/web.yaml", DEPLOY);
        let m = messages(&f);
        assert_eq!(
            m,
            vec![
                "20: side has no resources.limits: it may use all of a node's CPU and memory",
                "21: Image busybox has no tag, so it is :latest: pin a version",
            ]
        );
        assert_eq!(o[0].pod_labels, vec![("app".to_owned(), "web".to_owned())]);
        assert_eq!(o[0].refs, vec![("ConfigMap".to_owned(), "web-config".to_owned(), 19)]);
    }

    #[test]
    fn missing_and_wrong_heads() {
        let (f, _) = run("a.yaml", "kind: Pod\nspec: {}\n");
        let m = messages(&f);
        assert!(m.contains(&"1: No apiVersion: every Kubernetes object needs one".to_owned()), "{m:?}");
        assert!(m.iter().any(|x| x.contains("No metadata.name")), "{m:?}");
        let (f, _) = run("a.yaml", "apiVersion: extensions/v1beta1\nkind: Deployment\nmetadata:\n  name: x\n");
        assert_eq!(messages(&f), vec!["1: Deployment is served as apps/v1, not extensions/v1beta1"]);
        let (f, _) = run("a.yaml", "apiVersion: v1\nmetadata:\n  name: x\n");
        assert_eq!(messages(&f), vec!["1: No kind: every Kubernetes object needs one"]);
        // A kind not in the table (a custom resource) is fine.
        let (f, _) = run("a.yaml", "apiVersion: cert-manager.io/v1\nkind: Certificate\nmetadata:\n  name: x\n");
        assert!(f.is_empty());
        let (f, _) = run("a.yaml", "apiVersion: v1\nkind: Pod\nmetadata:\n  generateName: job-\n");
        assert!(f.is_empty(), "{:?}", messages(&f));
    }

    #[test]
    fn images() {
        assert_eq!(image_problem("nginx:1.25"), None);
        assert!(image_problem("nginx").is_some());
        assert!(image_problem("nginx:latest").unwrap().contains(":latest"));
        assert!(image_problem("registry.local:5000/app").is_some());
        assert_eq!(image_problem("registry.local:5000/app:2"), None);
        assert_eq!(image_problem("app@sha256:abc"), None);
        assert_eq!(image_problem("{{ .Values.image }}"), None);
    }

    #[test]
    fn selectors_in_a_folder() {
        let svc = "apiVersion: v1\nkind: Service\nmetadata:\n  name: web\nspec:\n  selector:\n    app: web\n---\napiVersion: v1\nkind: Service\nmetadata:\n  name: api\nspec:\n  selector:\n    app: api\n";
        let (_, mut objects) = run("k8s/web.yaml", DEPLOY);
        let (_, more) = run("k8s/svc.yaml", svc);
        objects.extend(more);
        let mut f = Vec::new();
        check_selectors(&objects, &mut f);
        assert_eq!(
            messages(&f),
            vec!["14: Service api's selector app=api matches the labels of no workload in this folder"]
        );
        let l = links(&objects);
        assert!(l.contains(&("Service/web".into(), "Deployment/web".into(), "selects".into())), "{l:?}");
        assert!(l.contains(&("Deployment/web".into(), "ConfigMap/web-config".into(), String::new())), "{l:?}");
        // In another folder, no workload: nothing to say.
        let (_, other) = run("other/svc.yaml", svc);
        let mut f = Vec::new();
        check_selectors(&other, &mut f);
        assert!(f.is_empty());
    }

    #[test]
    fn cronjob_and_ingress_refs() {
        let cj = "apiVersion: batch/v1\nkind: CronJob\nmetadata:\n  name: c\nspec:\n  jobTemplate:\n    spec:\n      template:\n        spec:\n          containers:\n            - name: c\n              image: app:latest\n              env:\n                - name: P\n                  valueFrom:\n                    secretKeyRef:\n                      name: db\n                      key: p\n";
        let (f, o) = run("a.yaml", cj);
        assert!(messages(&f).iter().any(|m| m.starts_with("12: Image app:latest is :latest")), "{:?}", messages(&f));
        assert_eq!(o[0].refs, vec![("Secret".into(), "db".into(), 17)]);
        let ing = "apiVersion: networking.k8s.io/v1\nkind: Ingress\nmetadata:\n  name: i\nspec:\n  rules:\n    - http:\n        paths:\n          - path: /\n            backend:\n              service:\n                name: web\n";
        let (_, o) = run("a.yaml", ing);
        assert_eq!(o[0].refs, vec![("Service".into(), "web".into(), 12)]);
    }

    #[test]
    fn kustomization_needs_no_name() {
        let (f, _) = run(
            "kustomization.yaml",
            "apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\nresources:\n  - web.yaml\n",
        );
        assert!(f.is_empty());
    }
}
