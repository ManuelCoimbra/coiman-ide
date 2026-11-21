//! MongoDB through its own shell, `mongosh`: databases, collections, documents found by a
//! filter, and a script box. The app opens no connection itself; `mongosh` does, and prints its
//! answers as Extended JSON. A password is never saved.
//!
//! The filter, sort and projection are the shell's own syntax (`{ price: { $gt: 2 } }`,
//! `ObjectId("…")`, `ISODate("…")`), so they go into the script as code; names go in as JSON
//! strings.

use std::path::PathBuf;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::language::lsp::discovery::quiet_command;
use crate::language::lsp::servers::which;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    #[serde(default)]
    pub name: String,
    /// `mongodb://host:port`, `mongodb+srv://cluster.example.net`, with any options; no password.
    pub uri: String,
    #[serde(default)]
    pub user: String,
    /// Where the user is defined (`admin` mostly); empty: the database connected to.
    #[serde(default)]
    pub auth_db: String,
    #[serde(skip)]
    pub password: Option<String>,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            name: String::new(),
            uri: "mongodb://127.0.0.1:27017".to_owned(),
            user: String::new(),
            auth_db: String::new(),
            password: None,
        }
    }
}

/// How many documents a find shows.
pub const LIMIT: usize = 200;

pub fn client() -> Option<PathBuf> {
    if let Some(found) = which("mongosh") {
        return Some(found);
    }
    let exe = if cfg!(windows) { "mongosh.exe" } else { "mongosh" };
    let bundled = crate::settings::config_dir()?.join("tools").join("mongosh").join("bin").join(exe);
    bundled.is_file().then_some(bundled)
}

pub const MISSING: &str = "The mongosh program isn't on this computer. Install the MongoDB Shell (mongosh), or \
                           unpack it into the config folder's tools/mongosh, and try again.";

/// A text as a JavaScript string literal.
fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned())
}

impl Server {
    pub fn label(&self) -> String {
        let uri = self.uri.trim();
        if self.user.is_empty() {
            uri.to_owned()
        } else {
            match uri.split_once("://") {
                Some((scheme, rest)) => format!("{scheme}://{}@{rest}", self.user),
                None => uri.to_owned(),
            }
        }
    }

    pub fn title(&self) -> String {
        if self.name.trim().is_empty() { self.label() } else { self.name.trim().to_owned() }
    }

    /// The address with a short time to give up on an unreachable server.
    fn address(&self) -> String {
        let uri = self.uri.trim();
        if uri.contains("serverSelectionTimeoutMS") {
            return uri.to_owned();
        }
        // An address with no path gets "/" before its options.
        let (base, query) = uri.split_once('?').map_or((uri, None), |(b, q)| (b, Some(q)));
        let has_path = base.split_once("://").is_some_and(|(_, rest)| rest.contains('/'));
        let base = if has_path { base.to_owned() } else { format!("{base}/") };
        match query {
            Some(q) => format!("{base}?{q}&serverSelectionTimeoutMS=5000"),
            None => format!("{base}?serverSelectionTimeoutMS=5000"),
        }
    }

    /// Runs a script in `mongosh` and gives back what its last expression is, as JSON.
    pub fn eval(&self, script: &str) -> Result<Value, String> {
        let program = client().ok_or(MISSING)?;
        let mut cmd = quiet_command(&program);
        cmd.arg(self.address()).args(["--quiet", "--norc", "--json=relaxed"]);
        if !self.user.is_empty() {
            cmd.args(["-u", &self.user]);
            if let Some(pw) = &self.password {
                cmd.args(["-p", pw]);
            }
            if !self.auth_db.is_empty() {
                cmd.args(["--authenticationDatabase", &self.auth_db]);
            }
        }
        let out = cmd
            .args(["--eval", script])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("Couldn't start {}: {e}", program.display()))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let value: Option<Value> = serde_json::from_str(stdout.trim()).ok();
        if out.status.success() {
            return value.ok_or_else(|| format!("mongosh answered something that isn't JSON: {}", stdout.trim()));
        }
        // An error comes as JSON too, with its message.
        let message = value
            .as_ref()
            .and_then(|v| v.get("message").or_else(|| v.get("errmsg")).and_then(Value::as_str).map(str::to_owned))
            .or_else(|| {
                stderr.lines().chain(stdout.lines()).find(|l| !l.trim().is_empty()).map(|l| l.trim().to_owned())
            })
            .unwrap_or_else(|| format!("mongosh stopped ({})", out.status));
        Err(message)
    }

    /// The databases and their sizes on disk, in bytes.
    pub fn databases(&self) -> Result<Vec<(String, u64)>, String> {
        let v = self.eval("db.adminCommand({ listDatabases: 1 }).databases.map(d => [d.name, d.sizeOnDisk])")?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|d| {
                        Some((d.get(0)?.as_str()?.to_owned(), d.get(1).and_then(Value::as_f64).unwrap_or(0.0) as u64))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// A database's collections (and views), with about how many documents each holds.
    pub fn collections(&self, db: &str) -> Result<Vec<(String, u64, bool)>, String> {
        let script = format!(
            "const d = db.getSiblingDB({}); d.getCollectionInfos({{}}, {{ nameOnly: false }})\
             .filter(c => !c.name.startsWith('system.'))\
             .map(c => [c.name, c.type === 'view' ? 0 : d.getCollection(c.name).estimatedDocumentCount(), c.type === 'view'])",
            js(db)
        );
        let v = self.eval(&script)?;
        let mut out: Vec<(String, u64, bool)> = v
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        Some((
                            c.get(0)?.as_str()?.to_owned(),
                            c.get(1).and_then(Value::as_f64).unwrap_or(0.0) as u64,
                            c.get(2).and_then(Value::as_bool).unwrap_or(false),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|a| a.0.to_lowercase());
        Ok(out)
    }

    /// Documents of a collection matching `filter`, sorted and projected as given (each the
    /// shell's syntax; empty: none), skipping `skip`: up to [`LIMIT`], and how many match.
    pub fn find(
        &self,
        db: &str,
        collection: &str,
        filter: &str,
        sort: &str,
        projection: &str,
        skip: usize,
    ) -> Result<(Vec<Value>, u64), String> {
        let or_empty = |s: &str| if s.trim().is_empty() { "{}".to_owned() } else { s.trim().to_owned() };
        let script = format!(
            "const c = db.getSiblingDB({db}).getCollection({coll}); const f = ({filter}); \
             [c.find(f, ({projection})).sort(({sort})).skip({skip}).limit({LIMIT}).toArray(), c.countDocuments(f)]",
            db = js(db),
            coll = js(collection),
            filter = or_empty(filter),
            projection = or_empty(projection),
            sort = or_empty(sort),
        );
        let v = self.eval(&script)?;
        let docs = v.get(0).and_then(Value::as_array).cloned().unwrap_or_default();
        let count = v.get(1).and_then(Value::as_f64).unwrap_or(docs.len() as f64) as u64;
        Ok((docs, count))
    }

    /// A shell script run in database `db`; what its last expression is.
    pub fn run(&self, db: &str, script: &str) -> Result<Value, String> {
        // `db` is the chosen database inside the script.
        self.eval(&format!("db = db.getSiblingDB({});\n{script}", js(db)))
    }

    /// Replaces the document with this `_id` by `doc` (both Extended JSON, as shown).
    pub fn replace(&self, db: &str, collection: &str, doc: &str) -> Result<u64, String> {
        let script = format!(
            "const d = EJSON.parse({doc}); db.getSiblingDB({db}).getCollection({coll}).replaceOne({{ _id: d._id }}, d).modifiedCount",
            doc = js(doc),
            db = js(db),
            coll = js(collection)
        );
        Ok(self.eval(&script)?.as_f64().unwrap_or(0.0) as u64)
    }

    pub fn insert(&self, db: &str, collection: &str, doc: &str) -> Result<(), String> {
        let script = format!(
            "db.getSiblingDB({db}).getCollection({coll}).insertOne(EJSON.parse({doc})).acknowledged",
            doc = js(doc),
            db = js(db),
            coll = js(collection)
        );
        self.eval(&script).map(|_| ())
    }

    /// Deletes the document with this `_id` (Extended JSON).
    pub fn delete(&self, db: &str, collection: &str, id: &Value) -> Result<u64, String> {
        let script = format!(
            "db.getSiblingDB({db}).getCollection({coll}).deleteOne({{ _id: EJSON.parse({id}) }}).deletedCount",
            id = js(&id.to_string()),
            db = js(db),
            coll = js(collection)
        );
        Ok(self.eval(&script)?.as_f64().unwrap_or(0.0) as u64)
    }
}

/// The columns for documents shown as a table: `_id` first, then each top-level field in the
/// order first seen.
pub fn columns(docs: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for d in docs {
        if let Some(o) = d.as_object() {
            for k in o.keys() {
                if !out.contains(k) {
                    out.push(k.clone());
                }
            }
        }
    }
    if let Some(i) = out.iter().position(|c| c == "_id") {
        let id = out.remove(i);
        out.insert(0, id);
    }
    out
}

/// A value in a table cell: Extended JSON's wrappers as the shell writes them
/// (`ObjectId('…')`, a date), objects and arrays as compact JSON. None when absent.
pub fn cell(v: Option<&Value>) -> Option<String> {
    let v = v?;
    Some(match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_owned(),
        Value::Object(o) if o.len() == 1 => {
            let (k, inner) = o.iter().next().expect("one entry");
            match (k.as_str(), inner) {
                ("$oid", Value::String(s)) => format!("ObjectId('{s}')"),
                ("$date", Value::String(s)) => s.clone(),
                ("$date", Value::Object(d)) => {
                    d.get("$numberLong").map_or_else(|| inner.to_string(), |n| n.to_string())
                }
                ("$numberDecimal" | "$numberLong", Value::String(s)) => s.clone(),
                _ => v.to_string(),
            }
        }
        other => other.to_string(),
    })
}

/// A document as indented Extended JSON, for reading and editing.
pub fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

pub fn load_saved() -> Vec<Server> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        server: Vec<Server>,
    }
    let Some(path) = crate::settings::config_dir().map(|d| d.join("mongo_connections.toml")) else {
        return Vec::new();
    };
    std::fs::read_to_string(path).ok().and_then(|t| toml::from_str::<File>(&t).ok()).map_or_else(Vec::new, |f| f.server)
}

pub fn remember(saved: &mut Vec<Server>, server: &Server) -> Result<(), String> {
    #[derive(Serialize)]
    struct File<'a> {
        server: &'a [Server],
    }
    let mut kept = server.clone();
    kept.password = None;
    saved.retain(|s| s.label() != kept.label());
    saved.insert(0, kept);
    saved.truncate(12);
    let dir = crate::settings::config_dir().ok_or("no config folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = toml::to_string(&File { server: saved }).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("mongo_connections.toml"), text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn addresses_cells_and_columns() {
        let s = Server { uri: "mongodb://h:1".into(), ..Server::default() };
        assert_eq!(s.address(), "mongodb://h:1/?serverSelectionTimeoutMS=5000");
        let s = Server { uri: "mongodb://h:1/shop?tls=true".into(), user: "ann".into(), ..Server::default() };
        assert_eq!(s.address(), "mongodb://h:1/shop?tls=true&serverSelectionTimeoutMS=5000");
        assert_eq!(s.label(), "mongodb://ann@h:1/shop?tls=true");
        let docs = vec![json!({"name": "pen", "_id": {"$oid": "abc"}}), json!({"_id": 2, "price": 1.5, "tags": ["a"]})];
        assert_eq!(columns(&docs), ["_id", "name", "price", "tags"]);
        assert_eq!(cell(docs[0].get("_id")).as_deref(), Some("ObjectId('abc')"));
        assert_eq!(cell(docs[1].get("tags")).as_deref(), Some("[\"a\"]"));
        assert_eq!(cell(docs[1].get("name")), None);
        assert_eq!(cell(Some(&json!({"$date": "2026-01-02T00:00:00Z"}))).as_deref(), Some("2026-01-02T00:00:00Z"));
        assert_eq!(js("a\"b"), "\"a\\\"b\"");
    }

    /// Against a real server when `COIMAN_TEST_MONGO` is its address (a database
    /// `coiman_test` is made and dropped).
    #[test]
    fn real_server() {
        let Ok(uri) = std::env::var("COIMAN_TEST_MONGO") else { return };
        let s = Server { uri, ..Server::default() };
        s.run("coiman_test", "db.dropDatabase()").unwrap();
        s.insert("coiman_test", "items", r#"{"name": "pen", "price": 1.5}"#).unwrap();
        s.insert("coiman_test", "items", r#"{"name": "book", "price": 12, "made": {"$date": "2026-01-02T00:00:00Z"}}"#)
            .unwrap();
        s.insert("coiman_test", "items", r#"{"name": "cup", "price": 4}"#).unwrap();
        assert!(s.databases().unwrap().iter().any(|(n, _)| n == "coiman_test"));
        assert_eq!(s.collections("coiman_test").unwrap(), [("items".to_owned(), 3, false)]);
        let (docs, count) = s.find("coiman_test", "items", "{ price: { $gt: 2 } }", "{ price: -1 }", "", 0).unwrap();
        assert_eq!(count, 2);
        assert_eq!(docs[0]["name"], "book");
        assert_eq!(cell(docs[0].get("made")).as_deref(), Some("2026-01-02T00:00:00Z"));
        // Edit one, then delete it.
        let mut book = docs[0].clone();
        book["price"] = json!(13);
        assert_eq!(s.replace("coiman_test", "items", &book.to_string()).unwrap(), 1);
        let (docs, _) = s.find("coiman_test", "items", r#"{ name: "book" }"#, "", "{ price: 1, _id: 1 }", 0).unwrap();
        assert_eq!(docs[0]["price"], 13);
        assert!(docs[0].get("name").is_none());
        assert_eq!(s.delete("coiman_test", "items", &docs[0]["_id"]).unwrap(), 1);
        // Errors say what's wrong.
        let e = s.find("coiman_test", "items", "{ $bad: 1 }", "", "", 0).unwrap_err();
        assert!(e.contains("$bad"), "{e}");
        assert!(s.run("coiman_test", "this is not javascript").is_err());
        let down = Server { uri: "mongodb://127.0.0.1:1".into(), ..Server::default() };
        assert!(down.databases().is_err());
        s.run("coiman_test", "db.dropDatabase()").unwrap();
    }
}
