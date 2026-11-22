//! Docker and Compose, through the `docker` program: its containers, images and a project's
//! Compose services, read from its JSON output, and the commands that act on them. Coiman
//! talks to no engine itself; `docker` does, run as an approved program.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    /// `running`, `exited`, `paused`, `created`…
    pub state: String,
    /// `Up 2 hours`, `Exited (0) 3 days ago`.
    pub status: String,
    pub ports: String,
}

impl Container {
    pub fn running(&self) -> bool {
        self.state == "running"
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Image {
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub size: String,
    pub created: String,
}

impl Image {
    /// `repo:tag`, or the id for an untagged image.
    pub fn reference(&self) -> String {
        if self.repository.is_empty() || self.repository == "<none>" {
            self.id.clone()
        } else {
            format!("{}:{}", self.repository, self.tag)
        }
    }
}

/// A Compose service's container.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Service {
    pub service: String,
    pub name: String,
    pub state: String,
    pub status: String,
    pub ports: String,
}

fn text(v: &Value, key: &str) -> String {
    match &v[key] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Docker's `--format json` output: one object a line, or (older Compose) one array.
fn objects(out: &str) -> Vec<Value> {
    let t = out.trim();
    if t.starts_with('[') {
        return serde_json::from_str::<Vec<Value>>(t).unwrap_or_default();
    }
    t.lines().filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok()).filter(Value::is_object).collect()
}

/// `docker ps -a --format json`.
pub fn parse_containers(out: &str) -> Vec<Container> {
    objects(out)
        .iter()
        .map(|v| Container {
            id: text(v, "ID"),
            name: text(v, "Names"),
            image: text(v, "Image"),
            state: text(v, "State"),
            status: text(v, "Status"),
            ports: text(v, "Ports"),
        })
        .collect()
}

/// `docker images --format json`.
pub fn parse_images(out: &str) -> Vec<Image> {
    objects(out)
        .iter()
        .map(|v| Image {
            id: text(v, "ID"),
            repository: text(v, "Repository"),
            tag: text(v, "Tag"),
            size: text(v, "Size"),
            created: text(v, "CreatedSince"),
        })
        .collect()
}

/// `docker compose ps -a --format json`.
pub fn parse_services(out: &str) -> Vec<Service> {
    objects(out)
        .iter()
        .map(|v| {
            let ports = v["Publishers"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter(|p| p["PublishedPort"].as_u64().unwrap_or(0) > 0)
                        .map(|p| format!("{}->{}", p["PublishedPort"], p["TargetPort"]))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_else(|| text(v, "Ports"));
            Service {
                service: text(v, "Service"),
                name: text(v, "Name"),
                state: text(v, "State"),
                status: text(v, "Status"),
                ports,
            }
        })
        .collect()
}

/// The names Compose looks for, in its order.
pub const COMPOSE_FILES: [&str; 4] = ["compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"];

/// What a failed `docker` call means, in a sentence.
pub fn explain(stderr: &str) -> String {
    let e = stderr.trim();
    let l = e.to_lowercase();
    if l.contains("cannot connect to the docker daemon")
        || l.contains("error during connect")
        || l.contains("docker_engine")
        || l.contains("is the docker daemon running")
    {
        "Docker's engine isn't running: start Docker Desktop (or the docker service), then refresh".to_owned()
    } else if l.contains("permission denied") && l.contains("docker.sock") {
        "No permission to use Docker: add yourself to the docker group, then log in again".to_owned()
    } else if l.contains("is not a docker command") && l.contains("compose") {
        "Docker Compose isn't installed (the `docker compose` plugin)".to_owned()
    } else {
        e.lines().last().unwrap_or("docker failed").to_owned()
    }
}

pub fn ps_args() -> Vec<String> {
    strs(&["ps", "-a", "--no-trunc", "--format", "json"])
}

pub fn images_args() -> Vec<String> {
    strs(&["images", "--format", "json"])
}

pub fn compose_ps_args(file: &str) -> Vec<String> {
    let mut a = compose(file);
    a.extend(strs(&["ps", "-a", "--format", "json"]));
    a
}

/// `docker compose -f <file>`.
pub fn compose(file: &str) -> Vec<String> {
    vec!["compose".to_owned(), "-f".to_owned(), file.to_owned()]
}

/// What can be done to a container, and the command for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerAction {
    Start,
    Stop,
    Restart,
    Remove,
}

impl ContainerAction {
    pub fn args(self, id: &str) -> Vec<String> {
        let verb = match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Remove => "rm",
        };
        vec![verb.to_owned(), id.to_owned()]
    }

    pub fn done(self) -> &'static str {
        match self {
            Self::Start => "Started",
            Self::Stop => "Stopped",
            Self::Restart => "Restarted",
            Self::Remove => "Removed",
        }
    }
}

/// `docker logs -f --tail 500 <id>`, for a terminal.
pub fn logs_args(id: &str) -> Vec<String> {
    strs(&["logs", "-f", "--tail", "500", id])
}

/// A shell in a running container: bash when it has one, else sh.
pub fn shell_args(id: &str) -> Vec<String> {
    strs(&["exec", "-it", id, "sh", "-c", "command -v bash >/dev/null && exec bash || exec sh"])
}

fn strs(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_docker_json() {
        let ps = concat!(
            r#"{"ID":"a1b2","Image":"nginx:1.27","Names":"web","Ports":"0.0.0.0:8080->80/tcp","State":"running","Status":"Up 2 hours"}"#,
            "\n",
            r#"{"ID":"c3d4","Image":"postgres:17","Names":"db","Ports":"","State":"exited","Status":"Exited (0) 3 days ago"}"#,
            "\nnot json\n"
        );
        let c = parse_containers(ps);
        assert_eq!(c.len(), 2);
        assert!(c[0].running() && !c[1].running());
        assert_eq!((c[1].name.as_str(), c[1].ports.as_str()), ("db", ""));
        let images = parse_images(
            r#"{"ID":"sha1","Repository":"nginx","Tag":"1.27","Size":"187MB","CreatedSince":"2 weeks ago"}
{"ID":"sha2","Repository":"<none>","Tag":"<none>","Size":"5MB","CreatedSince":"1 day ago"}"#,
        );
        assert_eq!(images[0].reference(), "nginx:1.27");
        assert_eq!(images[1].reference(), "sha2");
        // Compose as JSON lines, and as an array (older versions).
        let line = r#"{"Service":"web","Name":"app-web-1","State":"running","Status":"Up 5 minutes","Publishers":[{"URL":"0.0.0.0","TargetPort":80,"PublishedPort":8080,"Protocol":"tcp"},{"TargetPort":443,"PublishedPort":0}]}"#;
        let s = parse_services(line);
        assert_eq!((s[0].service.as_str(), s[0].ports.as_str()), ("web", "8080->80"));
        assert_eq!(parse_services(&format!("[{line}]")), s);
        assert!(parse_services("").is_empty());
    }

    #[test]
    fn commands_and_errors() {
        assert_eq!(ContainerAction::Remove.args("web"), ["rm", "web"]);
        assert_eq!(compose_ps_args("compose.yaml")[..3], ["compose", "-f", "compose.yaml"]);
        assert_eq!(logs_args("web"), ["logs", "-f", "--tail", "500", "web"]);
        assert!(
            explain("error during connect: this error may indicate that the docker daemon is not running")
                .contains("isn't running")
        );
        assert!(explain("docker: 'compose' is not a docker command.").contains("Compose isn't installed"));
        assert_eq!(explain("Error: No such container: x\n"), "Error: No such container: x");
    }
}
