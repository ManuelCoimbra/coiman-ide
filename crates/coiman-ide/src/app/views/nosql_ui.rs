//! Database: Redis… and Database: MongoDB…: windows over `redis-cli` and `mongosh`. Every call
//! runs on a thread of its own; the window shows what came back.

use std::sync::mpsc::{self, Receiver, Sender};

use coiman_core::data::mongo;
use coiman_core::data::redis::{self, Key, Reply};
use eframe::egui::{self, Color32, RichText, Stroke};
use serde_json::Value;

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

const RED: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);

fn window<'a>(ctx: &egui::Context, title: &'a str, id: &str, open: &'a mut bool) -> egui::Window<'a> {
    let pal = theme::colors();
    let room = ctx.content_rect().size();
    let size = egui::vec2((room.x * 0.8).min(1100.0), (room.y * 0.72).max(380.0));
    egui::Window::new(RichText::new(title).size(16.0).strong())
        .id(egui::Id::unique(id))
        .open(open)
        .collapsible(false)
        .default_size(size)
        .max_height(ctx.content_rect().height() - 40.0)
        .default_pos(ctx.content_rect().center() - size / 2.0)
        .frame(
            egui::Frame::window(&ctx.global_style())
                .fill(pal.editor_bg)
                .corner_radius(16.0)
                .stroke(Stroke::new(1.0, pal.border))
                .inner_margin(egui::Margin::symmetric(16, 10)),
        )
}

fn dim(s: impl Into<String>) -> RichText {
    RichText::new(s.into()).color(theme::colors().text_dim)
}

/// A table of text cells with headers, scrolling inside.
fn grid(ui: &mut egui::Ui, id: &str, headers: &[&str], rows: &[Vec<String>], mut clicked: Option<&mut Option<usize>>) {
    egui::Grid::new(id).striped(true).num_columns(headers.len()).spacing([14.0, 4.0]).show(ui, |ui| {
        for h in headers {
            ui.label(dim(*h).small().strong());
        }
        ui.end_row();
        for (i, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let mut shown = cell.replace('\n', "⏎");
                if shown.chars().count() > 120 {
                    shown = shown.chars().take(119).collect::<String>() + "…";
                }
                let r = ui.add(egui::Label::new(RichText::new(shown).monospace().small()).sense(egui::Sense::click()));
                let r = if cell.chars().count() > 120 || cell.contains('\n') { r.on_hover_text(cell) } else { r };
                if c == 0
                    && r.clicked()
                    && let Some(at) = clicked.as_deref_mut()
                {
                    *at = Some(i);
                }
            }
            ui.end_row();
        }
    });
}

// ----- Redis -----------------------------------------------------------------------------

/// A value as headers and rows.
type Table = (Vec<&'static str>, Vec<Vec<String>>);

enum RedisMsg {
    Keys(Result<(String, Vec<Key>, bool), String>),
    Value(String, Result<Table, String>),
    Console(String, Result<Reply, String>),
    Keyspace(Result<Vec<(u32, u64)>, String>),
    Done(Result<String, String>),
}

pub(crate) struct RedisView {
    open: bool,
    form: redis::Server,
    port: String,
    password: String,
    saved: Option<Vec<redis::Server>>,
    server: Option<redis::Server>,
    keyspace: Vec<(u32, u64)>,
    pattern: String,
    cursor: String,
    keys: Vec<Key>,
    selected: Option<Key>,
    value: Option<Result<Table, String>>,
    edit: String,
    ttl: String,
    confirm_delete: bool,
    console: String,
    log: Vec<(String, String)>,
    busy: Option<String>,
    error: Option<String>,
    tx: Sender<RedisMsg>,
    rx: Receiver<RedisMsg>,
}

impl Default for RedisView {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            open: false,
            form: redis::Server::default(),
            port: "6379".to_owned(),
            password: String::new(),
            saved: None,
            server: None,
            keyspace: Vec::new(),
            pattern: String::new(),
            cursor: "0".to_owned(),
            keys: Vec::new(),
            selected: None,
            value: None,
            edit: String::new(),
            ttl: String::new(),
            confirm_delete: false,
            console: String::new(),
            log: Vec::new(),
            busy: None,
            error: None,
            tx,
            rx,
        }
    }
}

enum RedisAsk {
    Connect,
    Disconnect,
    Scan { more: bool },
    Select(Key),
    Db(u32),
    Console,
    SaveString,
    SetTtl,
    Persist,
    Delete,
}

impl CodeApp {
    pub(crate) fn show_redis(&mut self) {
        let v = &mut self.redis;
        v.open = true;
        if v.server.is_none() {
            let saved = v.saved.get_or_insert_with(redis::load_saved);
            if let Some(last) = saved.first() {
                v.form = last.clone();
                v.port = last.port.to_string();
            }
        }
    }

    fn redis_spawn(
        &mut self,
        ctx: &egui::Context,
        what: &str,
        job: impl FnOnce(redis::Server) -> RedisMsg + Send + 'static,
    ) {
        let Some(server) = self.redis.server.clone() else { return };
        let (tx, wake) = (self.redis.tx.clone(), repaint(ctx));
        self.redis.busy = Some(what.to_owned());
        std::thread::spawn(move || {
            let _ = tx.send(job(server));
            wake();
        });
    }

    fn redis_scan(&mut self, ctx: &egui::Context, more: bool) {
        let cursor = if more { self.redis.cursor.clone() } else { "0".to_owned() };
        let pattern = self.redis.pattern.clone();
        self.redis_spawn(ctx, "Scanning…", move |s| {
            // A few rounds: SCAN may come back with few keys per round.
            let mut keys = Vec::new();
            let mut cursor = cursor;
            for _ in 0..20 {
                match s.scan(&cursor, &pattern, 500) {
                    Ok((next, found)) => {
                        keys.extend(found);
                        cursor = next;
                        if cursor == "0" || keys.len() >= 500 {
                            break;
                        }
                    }
                    Err(e) => return RedisMsg::Keys(Err(e)),
                }
            }
            RedisMsg::Keys(Ok((cursor, keys, more)))
        });
    }

    fn redis_select(&mut self, ctx: &egui::Context, key: Key) {
        self.redis.selected = Some(key.clone());
        self.redis.value = None;
        self.redis.confirm_delete = false;
        self.redis.ttl = if key.ttl_ms >= 0 { (key.ttl_ms / 1000).to_string() } else { String::new() };
        let name = key.name.clone();
        self.redis_spawn(ctx, "Reading…", move |s| RedisMsg::Value(name, s.value(&key)));
    }

    fn redis_do(&mut self, ctx: &egui::Context, words: Vec<String>, done: String) {
        self.redis_spawn(ctx, "Working…", move |s| {
            RedisMsg::Done(match s.run(&words) {
                Ok(Reply::Error(e)) => Err(e),
                Ok(_) => Ok(done),
                Err(e) => Err(e),
            })
        });
    }

    pub(crate) fn redis_window(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.redis.rx.try_recv() {
            let v = &mut self.redis;
            v.busy = None;
            match msg {
                RedisMsg::Keys(Ok((cursor, keys, more))) => {
                    v.error = None;
                    if !more {
                        v.keys.clear();
                    }
                    v.keys.extend(keys);
                    v.keys.sort_by(|a, b| a.name.cmp(&b.name));
                    v.cursor = cursor;
                }
                RedisMsg::Keys(Err(e)) => v.error = Some(e),
                RedisMsg::Value(name, value) => {
                    if v.selected.as_ref().is_some_and(|k| k.name == name) {
                        if let Ok((_, rows)) = &value
                            && v.selected.as_ref().is_some_and(|k| k.kind == "string")
                        {
                            v.edit = rows.first().and_then(|r| r.first()).cloned().unwrap_or_default();
                        }
                        v.value = Some(value);
                    }
                }
                RedisMsg::Console(line, reply) => {
                    let shown = match reply {
                        Ok(r) => r.show(),
                        Err(e) => format!("(error) {e}"),
                    };
                    v.log.push((line, shown));
                }
                RedisMsg::Keyspace(Ok(k)) => v.keyspace = k,
                RedisMsg::Keyspace(Err(e)) => {
                    // The first call after Connect: its error says the server can't be reached.
                    v.error = Some(e);
                    v.server = None;
                }
                RedisMsg::Done(Ok(done)) => {
                    self.status = Some(done);
                    let key = self.redis.selected.clone();
                    self.redis_scan(ctx, false);
                    if let Some(k) = key {
                        self.redis_select(ctx, k);
                    }
                }
                RedisMsg::Done(Err(e)) => v.error = Some(e),
            }
        }
        if !self.redis.open {
            return;
        }
        let mut open = true;
        let mut ask = None;
        window(ctx, "Redis", "redis_window", &mut open).show(ctx, |ui| ask = redis_ui(ui, &mut self.redis));
        self.redis.open = open;
        let Some(ask) = ask else { return };
        match ask {
            RedisAsk::Connect => {
                let v = &mut self.redis;
                let Ok(port) = v.port.trim().parse::<u16>() else {
                    v.error = Some("The port is a number, like 6379".to_owned());
                    return;
                };
                let mut server = v.form.clone();
                server.port = port;
                server.host = server.host.trim().to_owned();
                server.password = (!v.password.is_empty()).then(|| v.password.clone());
                let saved = v.saved.get_or_insert_with(redis::load_saved);
                let _ = redis::remember(saved, &server);
                v.server = Some(server);
                v.keys.clear();
                v.selected = None;
                v.value = None;
                v.error = None;
                self.redis_spawn(ctx, "Connecting…", |s| RedisMsg::Keyspace(s.keyspace()));
                self.redis_scan(ctx, false);
            }
            RedisAsk::Disconnect => {
                let v = &mut self.redis;
                v.server = None;
                v.keys.clear();
                v.selected = None;
                v.value = None;
            }
            RedisAsk::Scan { more } => self.redis_scan(ctx, more),
            RedisAsk::Select(key) => self.redis_select(ctx, key),
            RedisAsk::Db(db) => {
                if let Some(s) = self.redis.server.as_mut() {
                    s.db = db;
                }
                self.redis.form.db = db;
                self.redis.selected = None;
                self.redis.value = None;
                self.redis_scan(ctx, false);
            }
            RedisAsk::Console => {
                let line = std::mem::take(&mut self.redis.console);
                match redis::split_words(&line) {
                    Ok(words) if !words.is_empty() => {
                        self.redis_spawn(ctx, "Running…", move |s| RedisMsg::Console(line, s.run(&words)));
                    }
                    Ok(_) => {}
                    Err(e) => self.redis.log.push((line, format!("(error) {e}"))),
                }
            }
            RedisAsk::SaveString => {
                let Some(key) = self.redis.selected.clone() else { return };
                let mut words = vec!["SET".to_owned(), key.name.clone(), self.redis.edit.clone()];
                // SET drops the time to live: keep it.
                if key.ttl_ms > 0 {
                    words.extend(["PX".to_owned(), key.ttl_ms.to_string()]);
                }
                self.redis_do(ctx, words, format!("Saved {}", key.name));
            }
            RedisAsk::SetTtl => {
                let Some(key) = self.redis.selected.clone() else { return };
                let Ok(seconds) = self.redis.ttl.trim().parse::<u64>() else {
                    self.redis.error = Some("The time to live is a number of seconds".to_owned());
                    return;
                };
                self.redis_do(
                    ctx,
                    vec!["EXPIRE".to_owned(), key.name.clone(), seconds.to_string()],
                    format!("{} expires in {seconds} s", key.name),
                );
            }
            RedisAsk::Persist => {
                let Some(key) = self.redis.selected.clone() else { return };
                self.redis_do(
                    ctx,
                    vec!["PERSIST".to_owned(), key.name.clone()],
                    format!("{} no longer expires", key.name),
                );
            }
            RedisAsk::Delete => {
                let Some(key) = self.redis.selected.take() else { return };
                self.redis.value = None;
                self.redis_do(ctx, vec!["DEL".to_owned(), key.name.clone()], format!("Deleted {}", key.name));
            }
        }
    }
}

fn redis_ui(ui: &mut egui::Ui, v: &mut RedisView) -> Option<RedisAsk> {
    let mut ask = None;
    let Some(server) = v.server.clone() else {
        ui.label(RichText::new("Connect to Redis").strong());
        ui.add_space(4.0);
        let mut enter = false;
        let mut db = v.form.db.to_string();
        egui::Grid::new("redis_connect").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            for (label, text, hint) in [
                ("Host", &mut v.form.host, "127.0.0.1"),
                ("Port", &mut v.port, "6379"),
                ("Database", &mut db, "0"),
                ("User", &mut v.form.user, "empty: the default user"),
                ("Name", &mut v.form.name, "optional: Cache, staging"),
            ] {
                ui.label(label);
                let r = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(320.0));
                enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.end_row();
            }
            ui.label("Password");
            let r = ui.add(
                egui::TextEdit::singleline(&mut v.password)
                    .password(true)
                    .hint_text("kept only while open")
                    .desired_width(320.0),
            );
            enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut v.form.tls, "TLS");
            ui.end_row();
        });
        v.form.db = db.trim().parse().unwrap_or(0);
        ui.horizontal(|ui| {
            if ui.button("Connect").clicked() || enter {
                ask = Some(RedisAsk::Connect);
            }
        });
        if let Some(e) = &v.error {
            ui.colored_label(RED, e);
        }
        ui.label(dim("Runs through redis-cli: the password goes to it for each run and is never saved.").small());
        if redis::client().is_none() {
            ui.colored_label(RED, redis::MISSING);
        }
        let saved = v.saved.get_or_insert_with(redis::load_saved).clone();
        if !saved.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Recent").small().strong());
            for s in saved {
                if ui.selectable_label(false, s.title()).on_hover_text(s.label()).clicked() {
                    v.port = s.port.to_string();
                    v.form = s;
                    v.password.clear();
                }
            }
        }
        return ask;
    };

    ui.horizontal(|ui| {
        ui.label(RichText::new(server.title()).strong());
        let dbs: Vec<(u32, u64)> = v.keyspace.clone();
        egui::ComboBox::from_id_salt("redis_db").selected_text(format!("db {}", server.db)).show_ui(ui, |ui| {
            for n in 0..16u32 {
                let keys = dbs.iter().find(|(d, _)| *d == n).map_or(String::new(), |(_, k)| format!("  {k} keys"));
                if ui.selectable_label(server.db == n, format!("db {n}{keys}")).clicked() {
                    ask = Some(RedisAsk::Db(n));
                }
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Disconnect").clicked() {
                ask = Some(RedisAsk::Disconnect);
            }
            if let Some(b) = &v.busy {
                ui.label(dim(b));
            }
        });
    });
    if let Some(e) = &v.error {
        ui.colored_label(RED, e);
    }
    ui.separator();
    let console_height = 150.0;
    // From the screen, not what is left: the window would grow to fit its own panes.
    let body = (ui.ctx().content_rect().height() * 0.72 - 120.0 - console_height).max(180.0);
    // Held to their height: scroll areas would otherwise take all the window allows.
    ui.allocate_ui(egui::vec2(ui.available_width(), body), |ui| {
        ui.horizontal_top(|ui| {
            // Keys.
            ui.vertical(|ui| {
                ui.set_width(340.0);
                ui.set_height(body);
                ui.set_max_height(body);
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut v.pattern).hint_text("Pattern: user:*").desired_width(240.0),
                    );
                    if ui.button("Scan").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        ask = Some(RedisAsk::Scan { more: false });
                    }
                });
                ui.label(
                    dim(format!("{} keys{}", v.keys.len(), if v.cursor != "0" { ", more to scan" } else { "" }))
                        .small(),
                );
                egui::ScrollArea::vertical().id_salt("redis_keys").auto_shrink([false, false]).show(ui, |ui| {
                    for k in &v.keys {
                        let chosen = v.selected.as_ref().is_some_and(|s| s.name == k.name);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&k.kind).small().monospace().color(theme::colors().accent));
                            if ui.selectable_label(chosen, RichText::new(&k.name).monospace()).clicked() {
                                ask = Some(RedisAsk::Select(k.clone()));
                            }
                            let ttl = redis::human_ttl(k.ttl_ms);
                            if !ttl.is_empty() {
                                ui.label(dim(format!("⏱ {ttl}")).small());
                            }
                        });
                    }
                    if v.cursor != "0" && ui.button("Scan more").clicked() {
                        ask = Some(RedisAsk::Scan { more: true });
                    }
                });
            });
            ui.separator();
            // The selected key.
            ui.vertical(|ui| {
                ui.set_height(body);
                ui.set_max_height(body);
                let Some(key) = v.selected.clone() else {
                    ui.label(dim("Choose a key."));
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&key.name).monospace().strong());
                    let unit = match key.kind.as_str() {
                        "string" => "characters",
                        "hash" => "fields",
                        "set" | "zset" => "members",
                        "stream" => "entries",
                        _ => "items",
                    };
                    ui.label(dim(format!("{} · {} {unit}", key.kind, key.size.max(0))));
                });
                ui.horizontal(|ui| {
                    ui.label("Expires in");
                    ui.add(egui::TextEdit::singleline(&mut v.ttl).hint_text("never").desired_width(70.0));
                    ui.label("s");
                    if ui.small_button("Set").clicked() {
                        ask = Some(RedisAsk::SetTtl);
                    }
                    if key.ttl_ms >= 0 && ui.small_button("Never").on_hover_text("PERSIST").clicked() {
                        ask = Some(RedisAsk::Persist);
                    }
                    if v.confirm_delete {
                        ui.colored_label(RED, "Delete this key?");
                        if ui.small_button("Delete").clicked() {
                            ask = Some(RedisAsk::Delete);
                        }
                        if ui.small_button("Keep").clicked() {
                            v.confirm_delete = false;
                        }
                    } else if ui.small_button("Delete…").clicked() {
                        v.confirm_delete = true;
                    }
                });
                ui.separator();
                match &v.value {
                    None => {
                        ui.label(dim("Reading…"));
                    }
                    Some(Err(e)) => {
                        ui.colored_label(RED, e);
                    }
                    Some(Ok(_)) if key.kind == "string" => {
                        egui::ScrollArea::vertical().id_salt("redis_string").max_height(body - 90.0).show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut v.edit)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(6),
                            );
                        });
                        if ui.button("Save").clicked() {
                            ask = Some(RedisAsk::SaveString);
                        }
                    }
                    Some(Ok((headers, rows))) => {
                        if rows.len() >= redis::SHOWN {
                            ui.label(
                                dim(format!("The first {} items; the console shows the rest.", redis::SHOWN)).small(),
                            );
                        }
                        egui::ScrollArea::both().id_salt("redis_value").auto_shrink([false, false]).show(ui, |ui| {
                            grid(ui, "redis_value_grid", headers, rows, None);
                        });
                    }
                }
            });
        });
    });
    // The console.
    ui.separator();
    egui::ScrollArea::vertical().id_salt("redis_log").max_height(console_height - 40.0).stick_to_bottom(true).show(
        ui,
        |ui| {
            for (line, out) in &v.log {
                ui.label(RichText::new(format!("> {line}")).monospace().small().strong());
                ui.label(RichText::new(out).monospace().small());
            }
        },
    );
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut v.console)
                .hint_text("A command: HGETALL user:1, INFO memory, …")
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !v.console.trim().is_empty() {
            ask = Some(RedisAsk::Console);
            r.request_focus();
        }
    });
    ask
}

// ----- MongoDB ---------------------------------------------------------------------------

enum MongoMsg {
    Databases(Result<Vec<(String, u64)>, String>),
    Collections(String, Result<Vec<(String, u64, bool)>, String>),
    Found(Result<(Vec<Value>, u64), String>),
    Script(Result<Value, String>),
    Done(Result<String, String>),
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum MongoTab {
    #[default]
    Documents,
    Script,
}

pub(crate) struct MongoView {
    open: bool,
    form: mongo::Server,
    password: String,
    saved: Option<Vec<mongo::Server>>,
    server: Option<mongo::Server>,
    databases: Vec<(String, u64)>,
    collections: std::collections::HashMap<String, Vec<(String, u64, bool)>>,
    expanded: Vec<String>,
    /// (database, collection)
    chosen: Option<(String, String)>,
    tab: MongoTab,
    filter: String,
    sort: String,
    projection: String,
    skip: usize,
    docs: Vec<Value>,
    count: u64,
    /// The document open for editing (its index; None with text: a new one).
    editing: Option<(Option<usize>, String)>,
    confirm_delete: bool,
    script: String,
    script_out: Option<Result<String, String>>,
    busy: Option<String>,
    error: Option<String>,
    tx: Sender<MongoMsg>,
    rx: Receiver<MongoMsg>,
}

impl Default for MongoView {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            open: false,
            form: mongo::Server::default(),
            password: String::new(),
            saved: None,
            server: None,
            databases: Vec::new(),
            collections: Default::default(),
            expanded: Vec::new(),
            chosen: None,
            tab: MongoTab::default(),
            filter: String::new(),
            sort: String::new(),
            projection: String::new(),
            skip: 0,
            docs: Vec::new(),
            count: 0,
            editing: None,
            confirm_delete: false,
            script: "db.getCollectionNames()".to_owned(),
            script_out: None,
            busy: None,
            error: None,
            tx,
            rx,
        }
    }
}

enum MongoAsk {
    Connect,
    Disconnect,
    Refresh,
    Expand(String),
    Choose(String, String),
    Find,
    Page(usize),
    Save,
    Delete,
    Run,
}

impl CodeApp {
    pub(crate) fn show_mongo(&mut self) {
        let v = &mut self.mongo;
        v.open = true;
        if v.server.is_none() {
            let saved = v.saved.get_or_insert_with(mongo::load_saved);
            if let Some(last) = saved.first() {
                v.form = last.clone();
            }
        }
    }

    fn mongo_spawn(
        &mut self,
        ctx: &egui::Context,
        what: &str,
        job: impl FnOnce(mongo::Server) -> MongoMsg + Send + 'static,
    ) {
        let Some(server) = self.mongo.server.clone() else { return };
        let (tx, wake) = (self.mongo.tx.clone(), repaint(ctx));
        self.mongo.busy = Some(what.to_owned());
        std::thread::spawn(move || {
            let _ = tx.send(job(server));
            wake();
        });
    }

    fn mongo_find(&mut self, ctx: &egui::Context) {
        let Some((db, coll)) = self.mongo.chosen.clone() else { return };
        let v = &self.mongo;
        let (filter, sort, projection, skip) = (v.filter.clone(), v.sort.clone(), v.projection.clone(), v.skip);
        self.mongo_spawn(ctx, "Finding…", move |s| {
            MongoMsg::Found(s.find(&db, &coll, &filter, &sort, &projection, skip))
        });
    }

    pub(crate) fn mongo_window(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.mongo.rx.try_recv() {
            let v = &mut self.mongo;
            v.busy = None;
            match msg {
                MongoMsg::Databases(Ok(d)) => {
                    v.error = None;
                    v.databases = d;
                }
                MongoMsg::Databases(Err(e)) => {
                    v.error = Some(e);
                    v.server = None;
                }
                MongoMsg::Collections(db, Ok(c)) => {
                    v.collections.insert(db, c);
                }
                MongoMsg::Collections(_, Err(e)) => v.error = Some(e),
                MongoMsg::Found(Ok((docs, count))) => {
                    v.error = None;
                    v.docs = docs;
                    v.count = count;
                }
                MongoMsg::Found(Err(e)) => v.error = Some(e),
                MongoMsg::Script(out) => v.script_out = Some(out.map(|v| mongo::pretty(&v))),
                MongoMsg::Done(Ok(done)) => {
                    self.status = Some(done);
                    self.mongo.editing = None;
                    self.mongo.confirm_delete = false;
                    self.mongo_find(ctx);
                    // The tree's count of documents, again.
                    if let Some((db, _)) = self.mongo.chosen.clone() {
                        self.mongo_spawn(ctx, "Reading…", move |s| {
                            let c = s.collections(&db);
                            MongoMsg::Collections(db, c)
                        });
                    }
                }
                MongoMsg::Done(Err(e)) => v.error = Some(e),
            }
        }
        if !self.mongo.open {
            return;
        }
        let mut open = true;
        let mut ask = None;
        window(ctx, "MongoDB", "mongo_window", &mut open).show(ctx, |ui| ask = mongo_ui(ui, &mut self.mongo));
        self.mongo.open = open;
        let Some(ask) = ask else { return };
        match ask {
            MongoAsk::Connect => {
                let v = &mut self.mongo;
                let mut server = v.form.clone();
                server.uri = server.uri.trim().to_owned();
                server.password = (!v.password.is_empty()).then(|| v.password.clone());
                let saved = v.saved.get_or_insert_with(mongo::load_saved);
                let _ = mongo::remember(saved, &server);
                v.server = Some(server);
                v.databases.clear();
                v.collections.clear();
                v.chosen = None;
                v.docs.clear();
                v.error = None;
                self.mongo_spawn(ctx, "Connecting…", |s| MongoMsg::Databases(s.databases()));
            }
            MongoAsk::Disconnect => {
                let v = &mut self.mongo;
                v.server = None;
                v.databases.clear();
                v.collections.clear();
                v.chosen = None;
                v.docs.clear();
            }
            MongoAsk::Refresh => {
                self.mongo.collections.clear();
                let expanded = self.mongo.expanded.clone();
                self.mongo_spawn(ctx, "Reading…", |s| MongoMsg::Databases(s.databases()));
                for db in expanded {
                    self.mongo_spawn(ctx, "Reading…", move |s| {
                        let c = s.collections(&db);
                        MongoMsg::Collections(db, c)
                    });
                }
            }
            MongoAsk::Expand(db) => {
                if let Some(i) = self.mongo.expanded.iter().position(|d| *d == db) {
                    self.mongo.expanded.remove(i);
                } else {
                    self.mongo.expanded.push(db.clone());
                    if !self.mongo.collections.contains_key(&db) {
                        self.mongo_spawn(ctx, "Reading…", move |s| {
                            let c = s.collections(&db);
                            MongoMsg::Collections(db, c)
                        });
                    }
                }
            }
            MongoAsk::Choose(db, coll) => {
                let v = &mut self.mongo;
                v.chosen = Some((db, coll));
                v.tab = MongoTab::Documents;
                v.skip = 0;
                v.editing = None;
                v.docs.clear();
                self.mongo_find(ctx);
            }
            MongoAsk::Find => {
                self.mongo.skip = 0;
                self.mongo_find(ctx);
            }
            MongoAsk::Page(skip) => {
                self.mongo.skip = skip;
                self.mongo_find(ctx);
            }
            MongoAsk::Save => {
                let Some(((db, coll), (index, text))) = self.mongo.chosen.clone().zip(self.mongo.editing.clone())
                else {
                    return;
                };
                if let Err(e) = serde_json::from_str::<Value>(&text) {
                    self.mongo.error = Some(format!("Not JSON: {e}"));
                    return;
                }
                self.mongo_spawn(ctx, "Saving…", move |s| {
                    MongoMsg::Done(match index {
                        Some(_) => s.replace(&db, &coll, &text).map(|n| format!("Saved ({n} changed)")),
                        None => s.insert(&db, &coll, &text).map(|()| "Added the document".to_owned()),
                    })
                });
            }
            MongoAsk::Delete => {
                let Some(((db, coll), (Some(i), _))) = self.mongo.chosen.clone().zip(self.mongo.editing.clone()) else {
                    return;
                };
                let Some(id) = self.mongo.docs.get(i).and_then(|d| d.get("_id")).cloned() else { return };
                self.mongo_spawn(ctx, "Deleting…", move |s| {
                    MongoMsg::Done(s.delete(&db, &coll, &id).map(|n| format!("Deleted {n} document")))
                });
            }
            MongoAsk::Run => {
                let db = self.mongo.chosen.as_ref().map_or_else(|| "test".to_owned(), |(d, _)| d.clone());
                let script = self.mongo.script.clone();
                self.mongo.script_out = None;
                self.mongo_spawn(ctx, "Running…", move |s| MongoMsg::Script(s.run(&db, &script)));
            }
        }
    }
}

fn mongo_ui(ui: &mut egui::Ui, v: &mut MongoView) -> Option<MongoAsk> {
    let mut ask = None;
    let Some(server) = v.server.clone() else {
        ui.label(RichText::new("Connect to MongoDB").strong());
        ui.add_space(4.0);
        let mut enter = false;
        egui::Grid::new("mongo_connect").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            for (label, text, hint) in [
                ("Address", &mut v.form.uri, "mongodb://127.0.0.1:27017"),
                ("User", &mut v.form.user, "empty: none"),
                ("User's database", &mut v.form.auth_db, "admin"),
                ("Name", &mut v.form.name, "optional: Shop, staging"),
            ] {
                ui.label(label);
                let r = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(380.0));
                enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.end_row();
            }
            ui.label("Password");
            let r = ui.add(
                egui::TextEdit::singleline(&mut v.password)
                    .password(true)
                    .hint_text("kept only while open")
                    .desired_width(380.0),
            );
            enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.end_row();
        });
        if ui.button("Connect").clicked() || enter {
            ask = Some(MongoAsk::Connect);
        }
        if let Some(e) = &v.error {
            ui.colored_label(RED, e);
        }
        ui.label(dim("Runs through mongosh: the password is given to it for each run and never saved.").small());
        if mongo::client().is_none() {
            ui.colored_label(RED, mongo::MISSING);
        }
        let saved = v.saved.get_or_insert_with(mongo::load_saved).clone();
        if !saved.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Recent").small().strong());
            for s in saved {
                if ui.selectable_label(false, s.title()).on_hover_text(s.label()).clicked() {
                    v.form = s;
                    v.password.clear();
                }
            }
        }
        return ask;
    };

    ui.horizontal(|ui| {
        ui.label(RichText::new(server.title()).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Disconnect").clicked() {
                ask = Some(MongoAsk::Disconnect);
            }
            if ui.button("Refresh").clicked() {
                ask = Some(MongoAsk::Refresh);
            }
            if let Some(b) = &v.busy {
                ui.label(dim(b));
            }
        });
    });
    if let Some(e) = &v.error {
        ui.colored_label(RED, e);
    }
    ui.separator();
    let height = (ui.ctx().content_rect().height() * 0.72 - 110.0).max(220.0);
    // Held to their height: scroll areas would otherwise take all the window allows.
    ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
        ui.horizontal_top(|ui| {
            // Databases and collections.
            ui.vertical(|ui| {
                ui.set_width(250.0);
                ui.set_height(height);
                ui.set_max_height(height);
                egui::ScrollArea::vertical().id_salt("mongo_tree").auto_shrink([false, false]).show(ui, |ui| {
                    for (db, size) in &v.databases {
                        let open = v.expanded.contains(db);
                        let r = ui.selectable_label(false, format!("{} {db}", if open { "▾" } else { "▸" }));
                        if r.on_hover_text(coiman_core::build::binary_size::human(*size)).clicked() {
                            ask = Some(MongoAsk::Expand(db.clone()));
                        }
                        if !open {
                            continue;
                        }
                        match v.collections.get(db) {
                            None => {
                                ui.label(dim("    …"));
                            }
                            Some(list) if list.is_empty() => {
                                ui.label(dim("    no collections"));
                            }
                            Some(list) => {
                                for (name, count, view) in list {
                                    let chosen = v.chosen.as_ref().is_some_and(|(d, c)| d == db && c == name);
                                    ui.horizontal(|ui| {
                                        ui.add_space(14.0);
                                        let label = if *view { format!("{name}  (view)") } else { name.clone() };
                                        if ui.selectable_label(chosen, label).clicked() {
                                            ask = Some(MongoAsk::Choose(db.clone(), name.clone()));
                                        }
                                        if !view {
                                            ui.label(dim(count.to_string()).small());
                                        }
                                    });
                                }
                            }
                        }
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_height(height);
                ui.set_max_height(height);
                ui.horizontal(|ui| {
                    for (tab, label) in [(MongoTab::Documents, "Documents"), (MongoTab::Script, "Script")] {
                        if ui.selectable_label(v.tab == tab, label).clicked() {
                            v.tab = tab;
                        }
                    }
                    if let Some((db, coll)) = &v.chosen {
                        ui.label(dim(format!("{db}.{coll}")));
                    }
                });
                ui.separator();
                match v.tab {
                    MongoTab::Documents => ask = ask.take().or_else(|| documents_ui(ui, v)),
                    MongoTab::Script => {
                        ui.label(
                            dim("A mongosh script; db is the chosen database. What its last line is shows below.")
                                .small(),
                        );
                        ui.add(
                            egui::TextEdit::multiline(&mut v.script)
                                .code_editor()
                                .desired_rows(6)
                                .desired_width(f32::INFINITY),
                        );
                        if ui.button("Run").clicked() {
                            ask = Some(MongoAsk::Run);
                        }
                        match &v.script_out {
                            Some(Ok(out)) => {
                                egui::ScrollArea::both().id_salt("mongo_script_out").auto_shrink([false, false]).show(
                                    ui,
                                    |ui| {
                                        ui.label(RichText::new(out).monospace().small());
                                    },
                                );
                            }
                            Some(Err(e)) => {
                                ui.colored_label(RED, e);
                            }
                            None => {}
                        }
                    }
                }
            });
        });
    });
    ask
}

fn documents_ui(ui: &mut egui::Ui, v: &mut MongoView) -> Option<MongoAsk> {
    let mut ask = None;
    if v.chosen.is_none() {
        ui.label(dim("Choose a collection."));
        return None;
    }
    let mut enter = false;
    egui::Grid::new("mongo_query").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        for (label, text, hint) in [
            ("Filter", &mut v.filter, "{ price: { $gt: 2 } }"),
            ("Sort", &mut v.sort, "{ price: -1 }"),
            ("Fields", &mut v.projection, "{ name: 1, price: 1 }"),
        ] {
            ui.label(label);
            let r = ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
            enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Find").clicked() || enter {
            ask = Some(MongoAsk::Find);
        }
        if ui.button("Add a document").clicked() {
            v.editing = Some((None, "{\n  \n}".to_owned()));
            v.confirm_delete = false;
        }
        let shown = v.docs.len();
        if shown > 0 {
            ui.label(dim(format!("{}–{} of {}", v.skip + 1, v.skip + shown, v.count)));
        } else {
            ui.label(dim(format!("{} documents", v.count)));
        }
        if v.skip > 0 && ui.small_button("Previous").clicked() {
            ask = Some(MongoAsk::Page(v.skip.saturating_sub(mongo::LIMIT)));
        }
        if (v.skip + shown) < v.count as usize && ui.small_button("Next").clicked() {
            ask = Some(MongoAsk::Page(v.skip + mongo::LIMIT));
        }
    });
    if let Some((index, text)) = v.editing.as_mut() {
        ui.separator();
        ui.label(
            dim(if index.is_some() {
                "Edit the document (Extended JSON); its _id says which it replaces."
            } else {
                "A new document (Extended JSON)."
            })
            .small(),
        );
        egui::ScrollArea::vertical().id_salt("mongo_edit").max_height(220.0).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(text).code_editor().desired_width(f32::INFINITY).desired_rows(8));
        });
        let editing_existing = index.is_some();
        ui.horizontal(|ui| {
            if ui.button(if editing_existing { "Save" } else { "Add" }).clicked() {
                ask = Some(MongoAsk::Save);
            }
            if ui.button("Close").clicked() {
                v.editing = None;
                v.confirm_delete = false;
            }
            if editing_existing {
                if v.confirm_delete {
                    ui.colored_label(RED, "Delete this document?");
                    if ui.button("Delete").clicked() {
                        ask = Some(MongoAsk::Delete);
                    }
                    if ui.button("Keep").clicked() {
                        v.confirm_delete = false;
                    }
                } else if ui.button("Delete…").clicked() {
                    v.confirm_delete = true;
                }
            }
        });
        ui.separator();
    }
    let columns = mongo::columns(&v.docs);
    let headers: Vec<&str> = columns.iter().map(String::as_str).collect();
    let rows: Vec<Vec<String>> =
        v.docs.iter().map(|d| columns.iter().map(|c| mongo::cell(d.get(c)).unwrap_or_default()).collect()).collect();
    let mut clicked = None;
    egui::ScrollArea::both().id_salt("mongo_docs").auto_shrink([false, false]).show(ui, |ui| {
        if !rows.is_empty() {
            ui.label(dim("Click a document's first cell to open it.").small());
        }
        grid(ui, "mongo_docs_grid", &headers, &rows, Some(&mut clicked));
    });
    if let Some(i) = clicked {
        v.editing = Some((Some(i), mongo::pretty(&v.docs[i])));
        v.confirm_delete = false;
    }
    ask
}
