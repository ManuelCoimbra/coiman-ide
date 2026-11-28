//! Helm charts: a folder with `Chart.yaml`, its `values.yaml` and its `templates/`. The values
//! the templates use (`.Values.a.b`) are matched with the ones `values.yaml` defines: used but
//! not defined, and defined but never used.

use crate::yaml::{Node, Value};
use crate::{Finding, Level};

/// A use of a value in a template.
#[derive(Clone, Debug, PartialEq)]
pub struct Use {
    pub path: Vec<String>,
    pub file: String,
    pub line: usize,
    pub col: usize,
}

/// The folder of the chart a file belongs to, given the charts' folders: the deepest one that
/// holds it.
pub fn chart_of<'a>(file: &str, charts: &'a [String]) -> Option<&'a String> {
    charts.iter().filter(|c| c.is_empty() || file.starts_with(&format!("{c}/"))).max_by_key(|c| c.len())
}

/// Whether a file is one of a chart's templates.
pub fn is_template(file: &str, chart: &str) -> bool {
    let prefix = if chart.is_empty() { "templates/".to_owned() } else { format!("{chart}/templates/") };
    file.starts_with(&prefix)
}

/// Every `.Values.a.b` in a template's text (also `$.Values`).
pub fn uses(file: &str, text: &str) -> Vec<Use> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut from = 0;
        while let Some(i) = line[from..].find(".Values.") {
            let at = from + i;
            let start = at + ".Values.".len();
            let end = line[start..]
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '-'))
                .map_or(line.len(), |e| start + e);
            let path: Vec<String> = line[start..end]
                .trim_end_matches('.')
                .split('.')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            // The word before: none (`.Values`) or a variable (`$.Values`, `$root.Values`), not a field.
            let word = line[..at].rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).next();
            let ok = word.is_none_or(|w| w.is_empty() || w.starts_with('$'));
            if ok && !path.is_empty() {
                out.push(Use { path, file: file.to_owned(), line: n + 1, col: start });
            }
            from = end.max(start);
        }
    }
    out
}

/// What a lookup of a path in `values.yaml` finds.
#[derive(Debug, PartialEq)]
enum Found {
    Yes,
    /// A key along it is missing.
    No,
    /// A list or a scalar stands in the way: can't tell.
    Unknown,
}

fn lookup(values: &Node, path: &[String]) -> Found {
    let mut n = values;
    for k in path {
        match &n.value {
            Value::Map(_) => match n.get(k) {
                Some(next) => n = next,
                None => return Found::No,
            },
            Value::Null => return Found::Yes,
            _ => return Found::Unknown,
        }
    }
    Found::Yes
}

/// Uses of values `values.yaml` lacks. Keys named in `skip` (subcharts' values, `global`) are
/// not looked at.
pub fn missing(values: &Node, uses: &[Use], out: &mut Vec<Finding>) {
    for u in uses {
        if u.path[0] == "global" || lookup(values, &u.path) != Found::No {
            continue;
        }
        let name = u.path.join(".");
        out.push(Finding {
            path: u.file.clone(),
            line: u.line,
            start: u.col,
            end: u.col + name.len(),
            level: Level::Warning,
            message: format!(".Values.{name} is not in the chart's values.yaml: it is empty unless set at install"),
        });
    }
}

/// Values defined but used by no template: the top-most such key of each branch. `skip` names
/// keys that go to subcharts.
pub fn unused(values_path: &str, values: &Node, uses: &[Use], skip: &[String], out: &mut Vec<Finding>) {
    fn walk(node: &Node, prefix: &mut Vec<String>, uses: &[Use], file: &str, skip: &[String], out: &mut Vec<Finding>) {
        for e in node.entries() {
            prefix.push(e.key.clone());
            let top = prefix.len() == 1;
            if top && (e.key == "global" || skip.contains(&e.key)) {
                prefix.pop();
                continue;
            }
            // Used: a use goes through it (`.Values.a.b` for `a`), or takes it whole (`.Values.a`
            // for `a.b`, as `toYaml` does).
            let along = |u: &Use| {
                let n = u.path.len().min(prefix.len());
                u.path[..n] == prefix[..n]
            };
            let used_whole = uses.iter().any(|u| u.path.len() <= prefix.len() && along(u));
            let used_inside = uses.iter().any(|u| u.path.len() > prefix.len() && along(u));
            if used_whole {
            } else if used_inside {
                walk(&e.value, prefix, uses, file, skip, out);
            } else {
                out.push(Finding {
                    path: file.to_owned(),
                    line: e.line,
                    start: e.col,
                    end: e.col + e.key.len(),
                    level: Level::Hint,
                    message: format!(
                        "{} is set here but no template uses .Values.{}",
                        prefix.join("."),
                        prefix.join(".")
                    ),
                });
            }
            prefix.pop();
        }
    }
    walk(values, &mut Vec::new(), uses, values_path, skip, out);
}

/// The names of a chart's subcharts: its `dependencies` (by name or alias) and its `charts/`
/// folders.
pub fn subcharts(chart_yaml: Option<&Node>, folders: &[String]) -> Vec<String> {
    let mut out: Vec<String> = folders.to_vec();
    for d in chart_yaml.and_then(|c| c.get("dependencies")).map(Node::items).unwrap_or_default() {
        for k in ["alias", "name"] {
            if let Some(n) = d.get(k).and_then(Node::str) {
                out.push(n.to_owned());
            }
        }
    }
    out
}

/// What's wrong with `Chart.yaml` itself.
pub fn check_chart(path: &str, chart: &Node, out: &mut Vec<Finding>) {
    for key in ["apiVersion", "name", "version"] {
        if chart.get(key).is_none() {
            out.push(Finding {
                path: path.to_owned(),
                line: 1,
                start: 0,
                end: 1,
                level: Level::Error,
                message: format!("Chart.yaml has no {key}: Helm needs it"),
            });
        }
    }
    if let Some(v) = chart.get("apiVersion")
        && let Some(s) = v.str()
        && s != "v1"
        && s != "v2"
    {
        out.push(Finding {
            path: path.to_owned(),
            line: v.line,
            start: v.col,
            end: v.col + s.len(),
            level: Level::Error,
            message: format!("A chart's apiVersion is v2 (or v1 for Helm 2), not {s}"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml;

    fn values(text: &str) -> Node {
        yaml::read(text).remove(0).root
    }

    #[test]
    fn finds_uses() {
        let t = "image: {{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}\n\
                 {{- if $.Values.ingress.enabled }}\n{{ toYaml .Values.resources | nindent 4 }}\n\
                 x: {{ .Values.svc-port }} {{ $root.Values.nope }} {{ .Chart.Values.no }}\n";
        let u = uses("c/templates/d.yaml", t);
        let names: Vec<String> = u.iter().map(|u| format!("{}:{}", u.line, u.path.join("."))).collect();
        assert_eq!(
            names,
            vec!["1:image.repository", "1:image.tag", "2:ingress.enabled", "3:resources", "4:svc-port", "4:nope"]
        );
        assert_eq!(u[0].col, 18);
    }

    #[test]
    fn missing_and_unused() {
        let v = values(
            "image:\n  repository: nginx\n  tag: \"1.25\"\n  pullPolicy: IfNotPresent\nresources:\n  limits:\n    cpu: 1\nold: 1\nsub:\n  x: 1\nglobal:\n  a: 1\nlist:\n  - a\n",
        );
        let t = "{{ .Values.image.repository }}{{ .Values.image.tag }}{{ toYaml .Values.resources }}\n\
                 {{ .Values.ingress.host }}{{ .Values.list.0 }}{{ .Values.global.zone }}\n";
        let u = uses("t.yaml", t);
        let mut f = Vec::new();
        missing(&v, &u, &mut f);
        let m: Vec<String> = f.iter().map(|f| format!("{} {}", f.line, f.message)).collect();
        assert_eq!(
            m,
            vec!["2 .Values.ingress.host is not in the chart's values.yaml: it is empty unless set at install"]
        );
        let mut f = Vec::new();
        unused("values.yaml", &v, &u, &["sub".into()], &mut f);
        let m: Vec<String> = f.iter().map(|f| format!("{} {}", f.line, f.message)).collect();
        assert_eq!(
            m,
            vec![
                "4 image.pullPolicy is set here but no template uses .Values.image.pullPolicy",
                "8 old is set here but no template uses .Values.old",
            ]
        );
    }

    #[test]
    fn charts_and_templates() {
        let charts = vec!["".to_owned(), "deploy/app".to_owned()];
        assert_eq!(chart_of("deploy/app/templates/a.yaml", &charts).map(String::as_str), Some("deploy/app"));
        assert!(is_template("deploy/app/templates/a.yaml", "deploy/app"));
        assert!(!is_template("deploy/app/values.yaml", "deploy/app"));
        assert!(is_template("templates/x.tpl", ""));
    }

    #[test]
    fn chart_yaml() {
        let mut f = Vec::new();
        check_chart("Chart.yaml", &values("apiVersion: v3\nname: a\n"), &mut f);
        let m: Vec<String> = f.iter().map(|f| f.message.clone()).collect();
        assert_eq!(
            m,
            vec!["Chart.yaml has no version: Helm needs it", "A chart's apiVersion is v2 (or v1 for Helm 2), not v3"]
        );
        let deps = values("dependencies:\n  - name: redis\n    alias: cache\n");
        assert_eq!(subcharts(Some(&deps), &["pg".into()]), vec!["pg", "cache", "redis"]);
    }
}
