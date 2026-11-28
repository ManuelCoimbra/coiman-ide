//! The programs run when approved, all offline: `helm lint` and `helm template`, `terraform fmt
//! -check` and `terraform validate` (never `init` or `plan`, which reach the network), and
//! `ansible-lint`. Their output is shown as it is, with the places in files it names as links.

use std::collections::BTreeMap;

/// A program to run: its name, arguments, and a title for the panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub program: &'static str,
    pub args: Vec<String>,
    pub title: String,
    /// The folder paths in its output are relative to.
    pub base: String,
}

fn strs(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| (*s).to_owned()).collect()
}

/// What to run for the folder's charts, Terraform folders and Ansible files.
pub fn runs(charts: &[String], tf_dirs: &[String], ansible: bool) -> Vec<Run> {
    let mut out = Vec::new();
    for c in charts {
        let at = if c.is_empty() { "." } else { c.as_str() };
        out.push(Run { program: "helm", args: strs(&["lint", at]), title: format!("helm lint {at}"), base: c.clone() });
        out.push(Run {
            program: "helm",
            args: strs(&["template", at]),
            title: format!("helm template {at}"),
            base: c.clone(),
        });
    }
    if !tf_dirs.is_empty() {
        out.push(Run {
            program: "terraform",
            args: strs(&["fmt", "-check", "-recursive", "-no-color"]),
            title: "terraform fmt -check".into(),
            base: String::new(),
        });
        for d in tf_dirs {
            let mut args = Vec::new();
            if !d.is_empty() {
                args.push(format!("-chdir={d}"));
            }
            args.extend(strs(&["validate", "-no-color"]));
            let at = if d.is_empty() { "." } else { d.as_str() };
            out.push(Run { program: "terraform", args, title: format!("terraform validate in {at}"), base: d.clone() });
        }
    }
    if ansible {
        out.push(Run {
            program: "ansible-lint",
            args: strs(&["-p", "--nocolor"]),
            title: "ansible-lint".into(),
            base: String::new(),
        });
    }
    out
}

/// The Terraform folders `validate` runs in: those not used as another's module.
pub fn root_modules(modules: &BTreeMap<String, Vec<crate::terraform::File>>) -> Vec<String> {
    let used: Vec<String> = modules
        .iter()
        .flat_map(|(dir, files)| {
            files
                .iter()
                .flat_map(|f| &f.blocks)
                .filter_map(|b| b.source.as_deref())
                .filter(|s| s.starts_with("./") || s.starts_with("../"))
                .map(|s| crate::ansible::join(dir, s))
                .collect::<Vec<_>>()
        })
        .collect();
    modules.keys().filter(|d| !used.contains(d)).cloned().collect()
}

/// Why a run gave nothing useful, in words, when its output says.
pub fn explain(program: &str, output: &str) -> Option<String> {
    let o = output.to_lowercase();
    if program == "terraform"
        && (o.contains("terraform init")
            || o.contains("missing required provider")
            || o.contains("module not installed"))
    {
        return Some(
            "terraform validate needs the folder initialised: run terraform init yourself (it downloads providers, so \
             Coiman doesn't)"
                .into(),
        );
    }
    if program == "helm" && o.contains("found in chart.yaml, but missing in charts/") {
        return Some("The chart's dependencies aren't in charts/: helm dependency build fetches them".into());
    }
    None
}

fn clean(token: &str) -> &str {
    token.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ':' | ';'))
}

/// Places in files an output names: `path:line`, `on path line N` (Terraform), a file name on
/// a line of its own (`terraform fmt -check`), and `path: ... line N` (Helm). Paths are taken
/// relative to `base`, and only files of the folder count.
pub fn places(output: &str, base: &str, workspace: &str, files: &dyn Fn(&str) -> bool) -> Vec<(String, usize, String)> {
    let resolve = |p: &str| -> Option<String> {
        let p = p.replace('\\', "/");
        let ws = workspace.replace('\\', "/");
        let p = p.strip_prefix(&format!("{ws}/")).map(str::to_owned).unwrap_or(p);
        let p = p.trim_start_matches("./");
        let joined = crate::ansible::join(base, p);
        if files(&joined) {
            Some(joined)
        } else if files(p) {
            Some(p.to_owned())
        } else {
            None
        }
    };
    let mut out = Vec::new();
    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let words: Vec<&str> = t.split_whitespace().collect();
        let mut found: Option<(String, usize)> = None;
        // `path:line` or `path:line:col`.
        for w in &words {
            let mut parts = w.split(':');
            let (Some(p), Some(n)) = (parts.next(), parts.next()) else { continue };
            // A Windows drive: `C:/x/y.yml:3`.
            let (p, n) = if p.len() == 1 && n.starts_with(['/', '\\']) {
                match parts.next() {
                    Some(m) => (format!("{p}:{n}"), m),
                    None => continue,
                }
            } else {
                (p.to_owned(), n)
            };
            if let (Some(file), Ok(l)) = (resolve(clean(&p)), clean(n).parse::<usize>()) {
                found = Some((file, l));
                break;
            }
        }
        // `on main.tf line 3`.
        if found.is_none()
            && let Some(i) = words.iter().position(|w| *w == "on")
            && let (Some(p), Some(&"line"), Some(n)) = (words.get(i + 1), words.get(i + 2), words.get(i + 3))
            && let (Some(file), Ok(l)) = (resolve(clean(p)), clean(n).parse::<usize>())
        {
            found = Some((file, l));
        }
        // `[ERROR] templates/x.yaml: ... line 3: ...`, or a file name alone.
        if found.is_none() {
            for (i, w) in words.iter().enumerate() {
                if let Some(file) = resolve(clean(w)) {
                    let l = words[i..]
                        .windows(2)
                        .find(|p| p[0] == "line")
                        .and_then(|p| clean(p[1]).parse::<usize>().ok())
                        .unwrap_or(1);
                    found = Some((file, l));
                    break;
                }
            }
        }
        if let Some((file, l)) = found {
            out.push((file, l, t.to_owned()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_runs() {
        let r = runs(&["chart".into()], &["".into(), "env/prod".into()], true);
        let t: Vec<String> = r.iter().map(|r| format!("{} {}", r.program, r.args.join(" "))).collect();
        assert_eq!(
            t,
            vec![
                "helm lint chart",
                "helm template chart",
                "terraform fmt -check -recursive -no-color",
                "terraform validate -no-color",
                "terraform -chdir=env/prod validate -no-color",
                "ansible-lint -p --nocolor",
            ]
        );
        // Never what reaches the network.
        assert!(!t.iter().any(|x| x.contains(" init") || x.contains(" plan") || x.contains("dependency")));
    }

    #[test]
    fn roots() {
        let mut m = BTreeMap::new();
        m.insert("".to_owned(), vec![crate::terraform::read("main.tf", "module \"n\" {\n  source = \"./net\"\n}\n")]);
        m.insert("net".to_owned(), vec![crate::terraform::read("net/a.tf", "output \"x\" { value = 1 }\n")]);
        m.insert("other".to_owned(), vec![]);
        assert_eq!(root_modules(&m), vec!["", "other"]);
    }

    #[test]
    fn places_in_output() {
        let files = |p: &str| {
            ["site.yml", "roles/web/tasks/main.yml", "main.tf", "env/prod/vars.tf", "chart/templates/d.yaml"]
                .contains(&p)
        };
        let lint = "site.yml:3: name[missing]: All tasks should be named.\nroles/web/tasks/main.yml:7:5: yaml[truthy]: Truthy value\nC:/w/site.yml:9: x\nsome other line\n";
        let p = places(lint, "", "C:/w", &files);
        let got: Vec<(String, usize)> = p.iter().map(|(f, l, _)| (f.clone(), *l)).collect();
        assert_eq!(got, vec![("site.yml".into(), 3), ("roles/web/tasks/main.yml".into(), 7), ("site.yml".into(), 9)]);
        let tf = "Error: Reference to undeclared input variable\n\n  on vars.tf line 4, in resource \"a\" \"b\":\n";
        let p = places(tf, "env/prod", "", &files);
        assert_eq!(p[0].0, "env/prod/vars.tf");
        assert_eq!(p[0].1, 4);
        let fmt = "main.tf\nenv/prod/vars.tf\n";
        assert_eq!(places(fmt, "", "", &files).len(), 2);
        let helm = "[ERROR] templates/d.yaml: unable to parse YAML: error converting YAML to JSON: yaml: line 3: mapping values\n";
        let p = places(helm, "chart", "", &files);
        assert_eq!((p[0].0.as_str(), p[0].1), ("chart/templates/d.yaml", 3));
    }

    #[test]
    fn explains() {
        assert!(explain("terraform", "Error: Missing required provider\n").unwrap().contains("terraform init"));
        assert_eq!(explain("helm", "1 chart(s) linted, 0 chart(s) failed"), None);
    }
}
