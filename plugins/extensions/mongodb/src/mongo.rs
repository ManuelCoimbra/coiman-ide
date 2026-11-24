//! MongoDB through its own shell, `mongosh`: databases, collections, documents found by a
//! filter, and a script box. The editor opens no connection itself; `mongosh` does (run by the
//! extension as an approved program), and prints its answers as Extended JSON. A password is
//! never saved. Each operation is a call and a reading of what came back.
//!
//! The filter, sort and projection are the shell's own syntax (`{ price: { $gt: 2 } }`,
//! `ObjectId("…")`, `ISODate("…")`), so they go into the script as code; names go in as JSON
//! strings.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Call, Out};

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

    /// `mongosh` running a script; what its last expression is comes back as JSON.
    pub fn call(&self, script: &str) -> Call {
        let mut args = vec![self.address(), "--quiet".into(), "--norc".into(), "--json=relaxed".into()];
        if !self.user.is_empty() {
            args.extend(["-u".into(), self.user.clone()]);
            if let Some(pw) = &self.password {
                args.extend(["-p".into(), pw.clone()]);
            }
            if !self.auth_db.is_empty() {
                args.extend(["--authenticationDatabase".into(), self.auth_db.clone()]);
            }
        }
        args.extend(["--eval".into(), script.to_owned()]);
        Call::new("mongosh", args, String::new(), Vec::new())
    }

    /// The databases and their sizes on disk, in bytes.
    pub fn databases_call(&self) -> Call {
        self.call("db.adminCommand({ listDatabases: 1 }).databases.map(d => [d.name, d.sizeOnDisk])")
    }
}

/// The databases and their sizes on disk, in bytes.
pub fn databases_read(out: &Out) -> Result<Vec<(String, u64)>, String> {
        let v = read(out)?;
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

impl Server {
    /// A database's collections (and views), with about how many documents each holds.
    pub fn collections_call(&self, db: &str) -> Call {
        let script = format!(
            "const d = db.getSiblingDB({}); d.getCollectionInfos({{}}, {{ nameOnly: false }})\
             .filter(c => !c.name.startsWith('system.'))\
             .map(c => [c.name, c.type === 'view' ? 0 : d.getCollection(c.name).estimatedDocumentCount(), c.type === 'view'])",
            js(db)
        );
        self.call(&script)
    }
}

/// A database's collections (and views), with about how many documents each holds.
pub fn collections_read(out: &Out) -> Result<Vec<(String, u64, bool)>, String> {
        let v = read(out)?;
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

impl Server {

    /// Documents of a collection matching `filter`, sorted and projected as given (each the
    /// shell's syntax; empty: none), skipping `skip`: up to [`LIMIT`], and how many match.
    pub fn find_call(
        &self,
        db: &str,
        collection: &str,
        filter: &str,
        sort: &str,
        projection: &str,
        skip: usize,
    ) -> Call {
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
        self.call(&script)
    }

    /// A shell script run in database `db`; what its last expression is.
    pub fn run_call(&self, db: &str, script: &str) -> Call {
        // `db` is the chosen database inside the script.
        self.call(&format!("db = db.getSiblingDB({});\n{script}", js(db)))
    }

    /// Replaces the document with this `_id` by `doc` (both Extended JSON, as shown).
    pub fn replace_call(&self, db: &str, collection: &str, doc: &str) -> Call {
        let script = format!(
            "const d = EJSON.parse({doc}); db.getSiblingDB({db}).getCollection({coll}).replaceOne({{ _id: d._id }}, d).modifiedCount",
            doc = js(doc),
            db = js(db),
            coll = js(collection)
        );
        self.call(&script)
    }

    pub fn insert_call(&self, db: &str, collection: &str, doc: &str) -> Call {
        let script = format!(
            "db.getSiblingDB({db}).getCollection({coll}).insertOne(EJSON.parse({doc})).acknowledged",
            doc = js(doc),
            db = js(db),
            coll = js(collection)
        );
        self.call(&script)
    }

    /// Deletes the document with this `_id` (Extended JSON).
    pub fn delete_call(&self, db: &str, collection: &str, id: &Value) -> Call {
        let script = format!(
            "db.getSiblingDB({db}).getCollection({coll}).deleteOne({{ _id: EJSON.parse({id}) }}).deletedCount",
            id = js(&id.to_string()),
            db = js(db),
            coll = js(collection)
        );
        self.call(&script)
    }
}

/// What `mongosh` answered, as JSON; or its error's message.
pub fn read(out: &Out) -> Result<Value, String> {
    let value: Option<Value> = serde_json::from_str(out.stdout.trim()).ok();
    if out.ok() {
        return value.ok_or_else(|| format!("mongosh answered something that isn't JSON: {}", out.stdout.trim()));
    }
    // An error comes as JSON too, with its message.
    let message = value
        .as_ref()
        .and_then(|v| v.get("message").or_else(|| v.get("errmsg")).and_then(Value::as_str).map(str::to_owned))
        .or_else(|| {
            out.stderr.lines().chain(out.stdout.lines()).find(|l| !l.trim().is_empty()).map(|l| l.trim().to_owned())
        })
        .unwrap_or_else(|| out.stopped("mongosh"));
    Err(message)
}

/// A find's documents and how many match.
pub fn find_read(out: &Out) -> Result<(Vec<Value>, u64), String> {
    let v = read(out)?;
    let docs = v.get(0).and_then(Value::as_array).cloned().unwrap_or_default();
    let count = v.get(1).and_then(Value::as_f64).unwrap_or(docs.len() as f64) as u64;
    Ok((docs, count))
}

/// The number a replace or a delete answered: how many it changed.
pub fn count_read(out: &Out) -> Result<u64, String> {
    Ok(read(out)?.as_f64().unwrap_or(0.0) as u64)
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

/// Connections used before, newest first, without passwords, from the extension's storage.
pub fn saved(text: &str) -> Vec<Server> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        server: Vec<Server>,
    }
    toml::from_str::<File>(text).map_or_else(|_| Vec::new(), |f| f.server)
}

/// The storage's text with `server` first (and only once), without its password.
pub fn remember(saved: &mut Vec<Server>, server: &Server) -> String {
    #[derive(Serialize)]
    struct File<'a> {
        server: &'a [Server],
    }
    let mut kept = server.clone();
    kept.password = None;
    saved.retain(|s| s.label() != kept.label());
    saved.insert(0, kept);
    saved.truncate(12);
    toml::to_string(&File { server: saved }).unwrap_or_default()
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

    #[test]
    fn calls_and_replies() {
        let s = Server { uri: "mongodb://h:1".into(), user: "ann".into(), password: Some("pw".into()), ..Server::default() };
        let call = s.find_call("shop", "items", "{ price: { $gt: 2 } }", "", "", 200);
        assert_eq!(call.program, "mongosh");
        assert_eq!(
            call.args[..6],
            ["mongodb://h:1/?serverSelectionTimeoutMS=5000", "--quiet", "--norc", "--json=relaxed", "-u", "ann"]
        );
        assert!(call.args.contains(&"pw".to_owned()));
        let script = call.args.last().unwrap();
        assert!(script.contains(r#"getSiblingDB("shop").getCollection("items")"#), "{script}");
        assert!(script.contains(".skip(200)"), "{script}");
        let ok = |stdout: &str| Out { code: Some(0), stdout: stdout.into(), stderr: String::new() };
        let (docs, count) = find_read(&ok(r#"[[{"_id": 1, "name": "pen"}], 7]"#)).unwrap();
        assert_eq!((docs.len(), count), (1, 7));
        assert_eq!(databases_read(&ok(r#"[["admin", 40960], ["shop", 8192]]"#)).unwrap()[1], ("shop".into(), 8192));
        assert_eq!(collections_read(&ok(r#"[["b", 2, false], ["a", 0, true]]"#)).unwrap()[0], ("a".into(), 0, true));
        assert_eq!(count_read(&ok("1")).unwrap(), 1);
        let failed =
            Out { code: Some(1), stdout: r#"{"message": "unknown operator: $bad"}"#.into(), stderr: String::new() };
        assert_eq!(read(&failed).unwrap_err(), "unknown operator: $bad");
        let silent = Out {
            code: Some(1),
            stdout: String::new(),
            stderr: "MongoServerSelectionError: connect ECONNREFUSED".into(),
        };
        assert!(read(&silent).unwrap_err().starts_with("MongoServerSelectionError"));
        let saved_text = remember(&mut Vec::new(), &s);
        assert!(!saved_text.contains("pw"));
        assert_eq!(saved(&saved_text)[0].user, "ann");
    }
}
