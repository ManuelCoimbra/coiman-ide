//! OpenAPI (3.x) and Swagger (2.0) specifications, in JSON or YAML: the endpoints with their
//! parameters, bodies and responses, the models, and each endpoint as a request for a `.http`
//! file, its body filled with an example made from the schema.

use serde_json::{Map, Value, json};

/// Whether this file looks like a spec (by its extension and first keys).
pub fn looks_like_spec(extension: &str, text: &str) -> bool {
    if !matches!(extension.to_ascii_lowercase().as_str(), "json" | "yaml" | "yml") {
        return false;
    }
    let head: String = text.chars().take(4000).collect();
    head.lines().any(|l| {
        let l = l.trim_start().trim_start_matches('"');
        l.starts_with("openapi\"")
            || l.starts_with("openapi:")
            || l.starts_with("swagger\"")
            || l.starts_with("swagger:")
    })
}

/// YAML as JSON values (mapping keys become strings).
fn yaml_to_json(y: &yaml_rust::Yaml) -> Value {
    use yaml_rust::Yaml;
    match y {
        Yaml::Real(s) => {
            s.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map_or(Value::String(s.clone()), Value::Number)
        }
        Yaml::Integer(i) => json!(i),
        Yaml::String(s) => Value::String(s.clone()),
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::Array(a) => Value::Array(a.iter().map(yaml_to_json).collect()),
        Yaml::Hash(h) => Value::Object(
            h.iter()
                .map(|(k, v)| {
                    let key = match k {
                        Yaml::String(s) | Yaml::Real(s) => s.clone(),
                        Yaml::Integer(i) => i.to_string(),
                        Yaml::Boolean(b) => b.to_string(),
                        _ => String::new(),
                    };
                    (key, yaml_to_json(v))
                })
                .collect(),
        ),
        Yaml::Null | Yaml::Alias(_) | Yaml::BadValue => Value::Null,
    }
}

/// A spec's text as JSON, whichever it was written in.
pub fn to_value(text: &str) -> Result<Value, String> {
    if text.trim_start().starts_with('{') {
        return serde_json::from_str(text).map_err(|e| format!("Not valid JSON: {e}"));
    }
    let docs = yaml_rust::YamlLoader::load_from_str(text).map_err(|e| format!("Not valid YAML: {e}"))?;
    docs.first().map(yaml_to_json).ok_or_else(|| "The file is empty".to_owned())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    /// `path`, `query`, `header` or `cookie`.
    pub place: String,
    pub required: bool,
    pub ty: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub method: String,
    pub path: String,
    pub summary: String,
    pub description: String,
    pub operation_id: Option<String>,
    pub tags: Vec<String>,
    pub params: Vec<Param>,
    /// The body's media type and schema.
    pub body: Option<(String, Value)>,
    pub body_required: bool,
    /// Status, what it means, and its schema's type.
    pub responses: Vec<(String, String, String)>,
    pub deprecated: bool,
    /// Names of the security schemes it needs.
    pub security: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub name: String,
    pub description: String,
    /// Name, type, required, description.
    pub fields: Vec<(String, String, bool, String)>,
}

/// A security scheme: what a request carries for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    Bearer,
    Basic,
    /// An API key in a header (its name), the query or a cookie.
    ApiKey {
        place: String,
        name: String,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub title: String,
    pub version: String,
    pub description: String,
    /// "OpenAPI 3.1.0", "Swagger 2.0".
    pub kind: String,
    pub servers: Vec<String>,
    pub endpoints: Vec<Endpoint>,
    pub models: Vec<Model>,
    pub auth: Vec<(String, Auth)>,
    doc: Value,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// What a `$ref` points at (`#/components/schemas/Pet`).
fn follow<'a>(doc: &'a Value, r: &str) -> Option<&'a Value> {
    let path = r.strip_prefix("#/")?;
    let mut at = doc;
    for part in path.split('/') {
        let part = part.replace("~1", "/").replace("~0", "~");
        at = at.get(part.as_str())?;
    }
    Some(at)
}

/// A schema or parameter with its `$ref` followed (a few times at most).
fn deref<'a>(doc: &'a Value, mut v: &'a Value) -> &'a Value {
    for _ in 0..8 {
        match v.get("$ref").and_then(Value::as_str).and_then(|r| follow(doc, r)) {
            Some(next) => v = next,
            None => break,
        }
    }
    v
}

/// A schema's type as people read it: `Pet`, `Pet[]`, `string (date-time)`, `A | B`.
pub fn type_name(schema: &Value) -> String {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        return r.rsplit('/').next().unwrap_or(r).to_owned();
    }
    for (key, sep) in [("oneOf", " | "), ("anyOf", " | "), ("allOf", " & ")] {
        if let Some(list) = schema.get(key).and_then(Value::as_array) {
            return list.iter().map(type_name).collect::<Vec<_>>().join(sep);
        }
    }
    let ty = match schema.get("type") {
        Some(Value::String(t)) => t.clone(),
        // 3.1 allows ["string", "null"].
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" | "),
        _ if schema.get("properties").is_some() => "object".to_owned(),
        _ => String::new(),
    };
    match ty.as_str() {
        "array" => format!("{}[]", schema.get("items").map_or_else(|| "any".to_owned(), type_name)),
        "" => "any".to_owned(),
        t => match (schema.get("format").and_then(Value::as_str), schema.get("enum").and_then(Value::as_array)) {
            (_, Some(values)) if !values.is_empty() => {
                let shown: Vec<String> =
                    values.iter().take(6).map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).collect();
                format!("{t} ({})", shown.join(", "))
            }
            (Some(f), _) => format!("{t} ({f})"),
            _ => t.to_owned(),
        },
    }
}

/// An example of a value fitting `schema`: its own example or default, else one made from the
/// type (with formats such as dates and e-mails), objects with their fields.
pub fn example(doc: &Value, schema: &Value, depth: usize) -> Value {
    let schema = deref(doc, schema);
    for key in ["example", "default"] {
        if let Some(v) = schema.get(key) {
            return v.clone();
        }
    }
    if let Some(v) = schema.get("examples").and_then(Value::as_array).and_then(|a| a.first()) {
        return v.clone();
    }
    if let Some(v) = schema.get("enum").and_then(Value::as_array).and_then(|a| a.first()) {
        return v.clone();
    }
    if depth > 6 {
        return Value::Null;
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(first) = schema.get(key).and_then(Value::as_array).and_then(|a| a.first()) {
            return example(doc, first, depth + 1);
        }
    }
    if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
        let mut merged = Map::new();
        for part in all {
            if let Value::Object(o) = example(doc, part, depth + 1) {
                merged.extend(o);
            }
        }
        return Value::Object(merged);
    }
    let ty = match schema.get("type") {
        Some(Value::String(t)) => t.as_str(),
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).find(|t| *t != "null").unwrap_or("null"),
        _ if schema.get("properties").is_some() => "object",
        _ => "",
    };
    match ty {
        "object" => {
            let mut out = Map::new();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (k, v) in props {
                    // Read-only fields come back, they aren't sent.
                    if deref(doc, v).get("readOnly") == Some(&Value::Bool(true)) {
                        continue;
                    }
                    out.insert(k.clone(), example(doc, v, depth + 1));
                }
            }
            Value::Object(out)
        }
        "array" => Value::Array(schema.get("items").map(|i| vec![example(doc, i, depth + 1)]).unwrap_or_default()),
        "integer" => json!(schema.get("minimum").and_then(Value::as_i64).unwrap_or(0)),
        "number" => json!(0.0),
        "boolean" => json!(true),
        "string" => json!(match schema.get("format").and_then(Value::as_str).unwrap_or("") {
            "date-time" => "2026-01-01T12:00:00Z",
            "date" => "2026-01-01",
            "time" => "12:00:00",
            "email" => "ann@example.com",
            "uuid" => "00000000-0000-4000-8000-000000000000",
            "uri" | "url" => "https://example.com",
            "hostname" => "example.com",
            "ipv4" => "192.0.2.1",
            "byte" => "aGVsbG8=",
            "password" => "secret",
            _ => "string",
        }),
        _ => Value::Null,
    }
}

fn params(doc: &Value, list: Option<&Value>) -> Vec<Param> {
    list.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|p| {
                    let p = deref(doc, p);
                    let schema = p.get("schema").cloned().unwrap_or_else(|| p.clone());
                    Param {
                        name: text(p, "name"),
                        place: text(p, "in"),
                        required: p.get("required") == Some(&Value::Bool(true)),
                        ty: type_name(&schema),
                        description: text(p, "description"),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse(text_in: &str) -> Result<Spec, String> {
    let doc = to_value(text_in)?;
    let swagger = doc.get("swagger").is_some();
    if !swagger && doc.get("openapi").is_none() {
        return Err("Not an OpenAPI or Swagger file (no openapi: or swagger: at the top)".to_owned());
    }
    let info = doc.get("info").cloned().unwrap_or(Value::Null);
    let kind = if swagger {
        format!("Swagger {}", doc["swagger"].as_str().unwrap_or("2.0"))
    } else {
        format!("OpenAPI {}", doc["openapi"].as_str().unwrap_or("3"))
    };
    let servers = if swagger {
        let host = text(&doc, "host");
        let base = text(&doc, "basePath");
        let schemes: Vec<String> = doc
            .get("schemes")
            .and_then(Value::as_array)
            .map_or_else(Vec::new, |s| s.iter().filter_map(Value::as_str).map(str::to_owned).collect());
        if host.is_empty() {
            vec![base]
        } else {
            let scheme = schemes
                .iter()
                .find(|s| *s == "https")
                .or_else(|| schemes.first())
                .cloned()
                .unwrap_or_else(|| "https".into());
            vec![format!("{scheme}://{host}{base}")]
        }
    } else {
        doc.get("servers")
            .and_then(Value::as_array)
            .map(|s| {
                s.iter()
                    .map(|srv| {
                        // Server variables with their defaults: https://{region}.api.com.
                        let mut url = text(srv, "url");
                        if let Some(vars) = srv.get("variables").and_then(Value::as_object) {
                            for (k, v) in vars {
                                url = url.replace(
                                    &format!("{{{k}}}"),
                                    v.get("default").and_then(Value::as_str).unwrap_or(""),
                                );
                            }
                        }
                        url
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let global_security: Vec<String> = security_names(doc.get("security"));
    let mut endpoints = Vec::new();
    if let Some(paths) = doc.get("paths").and_then(Value::as_object) {
        for (path, item) in paths {
            let item = deref(&doc, item);
            let shared = params(&doc, item.get("parameters"));
            for method in ["get", "post", "put", "patch", "delete", "head", "options", "trace"] {
                let Some(op) = item.get(method) else { continue };
                let mut ps = shared.clone();
                for p in params(&doc, op.get("parameters")) {
                    ps.retain(|q| !(q.name == p.name && q.place == p.place));
                    ps.push(p);
                }
                // The body: 3.x requestBody, or 2.0's `in: body` parameter.
                let (body, body_required) = if swagger {
                    let raw = op.get("parameters").and_then(Value::as_array).and_then(|a| {
                        a.iter().map(|p| deref(&doc, p)).find(|p| p.get("in").and_then(Value::as_str) == Some("body"))
                    });
                    let consumes = op
                        .get("consumes")
                        .or_else(|| doc.get("consumes"))
                        .and_then(Value::as_array)
                        .and_then(|c| c.first())
                        .and_then(Value::as_str)
                        .unwrap_or("application/json")
                        .to_owned();
                    ps.retain(|p| p.place != "body");
                    (
                        raw.and_then(|p| p.get("schema")).map(|s| (consumes, s.clone())),
                        raw.is_some_and(|p| p.get("required") == Some(&Value::Bool(true))),
                    )
                } else {
                    let rb = op.get("requestBody").map(|b| deref(&doc, b));
                    let content = rb.and_then(|b| b.get("content")).and_then(Value::as_object);
                    let pick =
                        content.and_then(|c| c.iter().find(|(k, _)| k.contains("json")).or_else(|| c.iter().next()));
                    (
                        pick.map(|(k, v)| (k.clone(), v.get("schema").cloned().unwrap_or(Value::Null))),
                        rb.is_some_and(|b| b.get("required") == Some(&Value::Bool(true))),
                    )
                };
                let responses = op
                    .get("responses")
                    .and_then(Value::as_object)
                    .map(|r| {
                        r.iter()
                            .map(|(code, resp)| {
                                let resp = deref(&doc, resp);
                                let schema = if swagger {
                                    resp.get("schema").cloned()
                                } else {
                                    resp.get("content")
                                        .and_then(Value::as_object)
                                        .and_then(|c| c.values().next())
                                        .and_then(|m| m.get("schema"))
                                        .cloned()
                                };
                                (
                                    code.clone(),
                                    text(resp, "description"),
                                    schema.map(|s| type_name(&s)).unwrap_or_default(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let security = match op.get("security") {
                    Some(s) => security_names(Some(s)),
                    None => global_security.clone(),
                };
                endpoints.push(Endpoint {
                    method: method.to_ascii_uppercase(),
                    path: path.clone(),
                    summary: text(op, "summary"),
                    description: text(op, "description"),
                    operation_id: op.get("operationId").and_then(Value::as_str).map(str::to_owned),
                    tags: op
                        .get("tags")
                        .and_then(Value::as_array)
                        .map_or_else(Vec::new, |t| t.iter().filter_map(Value::as_str).map(str::to_owned).collect()),
                    params: ps,
                    body,
                    body_required,
                    responses,
                    deprecated: op.get("deprecated") == Some(&Value::Bool(true)),
                    security,
                });
            }
        }
    }
    let schemas = if swagger { doc.get("definitions") } else { doc.get("components").and_then(|c| c.get("schemas")) };
    let models = schemas
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(name, s)| {
                    let required: Vec<&str> = s
                        .get("required")
                        .and_then(Value::as_array)
                        .map_or_else(Vec::new, |r| r.iter().filter_map(Value::as_str).collect());
                    let fields = s
                        .get("properties")
                        .and_then(Value::as_object)
                        .map(|p| {
                            p.iter()
                                .map(|(f, fs)| {
                                    (
                                        f.clone(),
                                        type_name(fs),
                                        required.contains(&f.as_str()),
                                        text(deref(&doc, fs), "description"),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Model { name: name.clone(), description: text(s, "description"), fields }
                })
                .collect()
        })
        .unwrap_or_default();
    let schemes = if swagger {
        doc.get("securityDefinitions")
    } else {
        doc.get("components").and_then(|c| c.get("securitySchemes"))
    };
    let auth = schemes
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(name, s)| {
                    let s = deref(&doc, s);
                    let a = match (text(s, "type").as_str(), text(s, "scheme").to_ascii_lowercase().as_str()) {
                        ("http", "bearer") | ("oauth2", _) | ("openIdConnect", _) => Auth::Bearer,
                        ("http", "basic") | ("basic", _) => Auth::Basic,
                        ("apiKey", _) => Auth::ApiKey { place: text(s, "in"), name: text(s, "name") },
                        _ => Auth::Other,
                    };
                    (name.clone(), a)
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Spec {
        title: text(&info, "title"),
        version: text(&info, "version"),
        description: text(&info, "description"),
        kind,
        servers,
        endpoints,
        models,
        auth,
        doc,
    })
}

fn security_names(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_object).flat_map(|o| o.keys().cloned()).collect())
        .unwrap_or_default()
}

/// A name usable as an `.http` variable.
fn var_name(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.is_empty() {
        out.push('x');
    }
    out
}

impl Spec {
    /// An endpoint as a request in a `.http` file: `{{baseUrl}}`, path and required query
    /// parameters as `{{variables}}`, the auth header the spec asks for, and an example body.
    pub fn request(&self, e: &Endpoint) -> String {
        let mut out = String::new();
        let title = if e.summary.is_empty() { format!("{} {}", e.method, e.path) } else { e.summary.clone() };
        out.push_str(&format!("### {title}\n"));
        if let Some(id) = &e.operation_id {
            out.push_str(&format!("# @name {}\n", var_name(id)));
        }
        let mut path = e.path.clone();
        for p in e.params.iter().filter(|p| p.place == "path") {
            path = path.replace(&format!("{{{}}}", p.name), &format!("{{{{{}}}}}", var_name(&p.name)));
        }
        let query: Vec<String> = e
            .params
            .iter()
            .filter(|p| p.place == "query" && p.required)
            .map(|p| format!("{}={{{{{}}}}}", p.name, var_name(&p.name)))
            .collect();
        let q = if query.is_empty() { String::new() } else { format!("?{}", query.join("&")) };
        out.push_str(&format!("{} {{{{baseUrl}}}}{path}{q}\n", e.method));
        for p in e.params.iter().filter(|p| p.place == "query" && !p.required) {
            out.push_str(&format!("# optional: &{}=  ({})\n", p.name, p.ty));
        }
        for p in e.params.iter().filter(|p| p.place == "header") {
            out.push_str(&format!("{}: {{{{{}}}}}\n", p.name, var_name(&p.name)));
        }
        for name in &e.security {
            match self.auth.iter().find(|(n, _)| n == name).map(|(_, a)| a) {
                Some(Auth::Bearer) => out.push_str("Authorization: Bearer {{token}}\n"),
                Some(Auth::Basic) => out.push_str("Authorization: Basic {{credentials}}\n"),
                Some(Auth::ApiKey { place, name }) if place == "header" => {
                    out.push_str(&format!("{name}: {{{{apiKey}}}}\n"))
                }
                _ => {}
            }
        }
        if let Some((media, schema)) = &e.body {
            out.push_str(&format!("Content-Type: {media}\n\n"));
            let ex = example(&self.doc, schema, 0);
            out.push_str(&if media.contains("json") {
                serde_json::to_string_pretty(&ex).unwrap_or_default()
            } else {
                ex.to_string()
            });
            out.push('\n');
        }
        out
    }

    /// The whole API as a `.http` file: its server as `@baseUrl`, then each endpoint.
    pub fn http_file(&self) -> String {
        let mut out = format!("# {} {} ({})\n", self.title, self.version, self.kind);
        out.push_str(&format!(
            "@baseUrl = {}\n",
            self.servers.first().cloned().unwrap_or_else(|| "http://localhost:8080".into())
        ));
        let mut vars: Vec<String> = Vec::new();
        for e in &self.endpoints {
            for p in e
                .params
                .iter()
                .filter(|p| p.place == "path" || p.place == "header" || (p.place == "query" && p.required))
            {
                let v = var_name(&p.name);
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
        }
        for v in vars {
            out.push_str(&format!("@{v} = \n"));
        }
        for e in &self.endpoints {
            out.push('\n');
            out.push_str(&self.request(e));
        }
        out
    }

    /// The endpoints by tag (untagged under "Other"), in the spec's order.
    pub fn by_tag(&self) -> Vec<(String, Vec<usize>)> {
        let mut out: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, e) in self.endpoints.iter().enumerate() {
            let tag = e.tags.first().cloned().unwrap_or_else(|| "Other".into());
            match out.iter_mut().find(|(t, _)| *t == tag) {
                Some((_, list)) => list.push(i),
                None => out.push((tag, vec![i])),
            }
        }
        out
    }

    /// An example body for an endpoint, as text.
    pub fn body_example(&self, e: &Endpoint) -> Option<String> {
        e.body.as_ref().map(|(_, s)| serde_json::to_string_pretty(&example(&self.doc, s, 0)).unwrap_or_default())
    }
}

/// The line (0-based) where a path is declared in the spec's text, to jump to it.
pub fn line_of_path(text: &str, path: &str) -> Option<usize> {
    let quoted = format!("\"{path}\"");
    let yaml = format!("{path}:");
    text.lines().position(|l| {
        let t = l.trim_start();
        t.starts_with(&quoted) || t.starts_with(&yaml) || t.starts_with(&format!("'{path}':"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PETS: &str = r#"openapi: 3.0.3
info:
  title: Pets
  version: "1.2"
servers:
  - url: https://{region}.pets.example.com/v1
    variables:
      region:
        default: eu
security:
  - bearer: []
paths:
  /pets/{petId}:
    parameters:
      - name: petId
        in: path
        required: true
        schema: { type: integer, format: int64 }
    get:
      summary: Get a pet
      operationId: getPet
      tags: [pets]
      parameters:
        - $ref: '#/components/parameters/Verbose'
      responses:
        '200':
          description: The pet
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Pet' }
        '404': { description: No such pet }
  /pets:
    post:
      summary: Add a pet
      operationId: add-pet
      tags: [pets]
      security: [{ key: [] }]
      requestBody:
        required: true
        content:
          application/json:
            schema: { $ref: '#/components/schemas/NewPet' }
      responses:
        '201': { description: Added }
components:
  parameters:
    Verbose:
      name: verbose
      in: query
      schema: { type: boolean }
  securitySchemes:
    bearer: { type: http, scheme: bearer }
    key: { type: apiKey, in: header, name: X-Api-Key }
  schemas:
    Pet:
      allOf:
        - $ref: '#/components/schemas/NewPet'
        - type: object
          properties:
            id: { type: integer, readOnly: true }
    NewPet:
      type: object
      required: [name]
      properties:
        name: { type: string, example: Rex }
        kind: { type: string, enum: [dog, cat] }
        born: { type: string, format: date }
        tags: { type: array, items: { type: string } }
"#;

    #[test]
    fn reads_openapi_3() {
        assert!(looks_like_spec("yaml", PETS) && !looks_like_spec("yaml", "a: 1") && !looks_like_spec("rs", PETS));
        let s = parse(PETS).unwrap();
        assert_eq!((s.title.as_str(), s.version.as_str(), s.kind.as_str()), ("Pets", "1.2", "OpenAPI 3.0.3"));
        assert_eq!(s.servers, ["https://eu.pets.example.com/v1"]);
        assert_eq!(s.endpoints.len(), 2);
        let get = &s.endpoints[0];
        assert_eq!((get.method.as_str(), get.path.as_str()), ("GET", "/pets/{petId}"));
        assert_eq!(get.params.len(), 2, "{:?}", get.params);
        assert_eq!(get.params[0].ty, "integer (int64)");
        assert_eq!(get.params[1].name, "verbose");
        assert_eq!(get.responses[0], ("200".into(), "The pet".into(), "Pet".into()));
        assert_eq!(get.security, ["bearer"]);
        let post = &s.endpoints[1];
        assert!(post.body_required && post.security == ["key"]);
        assert_eq!(s.models.iter().find(|m| m.name == "NewPet").unwrap().fields[1].1, "string (dog, cat)");
        assert_eq!(s.by_tag(), [("pets".to_owned(), vec![0, 1])]);
        // As requests.
        let r = s.request(get);
        assert!(r.contains("# @name getPet\nGET {{baseUrl}}/pets/{{petId}}\n"), "{r}");
        assert!(r.contains("# optional: &verbose=  (boolean)") && r.contains("Authorization: Bearer {{token}}"), "{r}");
        let r = s.request(post);
        assert!(r.contains("# @name add_pet") && r.contains("X-Api-Key: {{apiKey}}"), "{r}");
        let body: Value = serde_json::from_str(r.split("application/json\n\n").nth(1).unwrap()).unwrap();
        assert_eq!(body, json!({"name": "Rex", "kind": "dog", "born": "2026-01-01", "tags": ["string"]}));
        // The read-only id isn't in an example to send.
        let pet = example(&s.doc, &json!({"$ref": "#/components/schemas/Pet"}), 0);
        assert!(pet.get("id").is_none() && pet["name"] == "Rex");
        let file = s.http_file();
        assert!(file.contains("@baseUrl = https://eu.pets.example.com/v1\n@petId = \n"), "{file}");
        // Two requests, the second a POST (the HTTP Client extension reads them).
        let starts: Vec<&str> = file.lines().filter(|l| l.starts_with("GET ") || l.starts_with("POST ")).collect();
        assert_eq!(starts.len(), 2);
        assert!(starts[1].starts_with("POST "));
        assert_eq!(line_of_path(PETS, "/pets"), Some(31));
        assert_eq!(line_of_path(PETS, "/pets/{petId}"), Some(12));
    }

    #[test]
    fn reads_swagger_2_json() {
        let spec = r##"{
  "swagger": "2.0",
  "info": {"title": "Shop", "version": "1"},
  "host": "shop.example.com", "basePath": "/api", "schemes": ["http", "https"],
  "paths": {
    "/orders": {
      "post": {
        "summary": "New order",
        "parameters": [
          {"in": "body", "name": "order", "required": true, "schema": {"$ref": "#/definitions/Order"}},
          {"in": "query", "name": "dry", "required": true, "type": "boolean"}
        ],
        "responses": {"200": {"description": "ok", "schema": {"type": "array", "items": {"$ref": "#/definitions/Order"}}}}
      }
    }
  },
  "definitions": {"Order": {"type": "object", "properties": {"qty": {"type": "integer", "minimum": 1}}}}
}"##;
        let s = parse(spec).unwrap();
        assert_eq!((s.kind.as_str(), s.servers[0].as_str()), ("Swagger 2.0", "https://shop.example.com/api"));
        let e = &s.endpoints[0];
        assert_eq!(e.params.len(), 1);
        assert_eq!(e.params[0].ty, "boolean");
        assert_eq!(e.responses[0].2, "Order[]");
        let r = s.request(e);
        assert!(r.contains("POST {{baseUrl}}/orders?dry={{dry}}"), "{r}");
        assert!(r.contains("\"qty\": 1"), "{r}");
        assert!(parse("{\"a\": 1}").unwrap_err().contains("Not an OpenAPI"));
        assert!(parse("openapi: [").is_err());
    }
}
