//! Tool Runner: an example of what plugin API 0.3 adds. It starts approved
//! programs, talks to the language server, and shows results in richer panels.

wit_bindgen::generate!({
    path: "../../../core/wit/plugin.wit",
    world: "plugin",
});

use std::sync::atomic::{AtomicU64, Ordering};

use rust_code::plugin::host;
use rust_code::plugin::types::{Block, JobResult, ServerResponse, Span, Table};

/// The job the Cancel command stops.
static LAST_JOB: AtomicU64 = AtomicU64::new(0);

fn show(title: &str, blocks: Vec<Block>) {
    let mut all = vec![Block::Heading(title.to_owned())];
    all.extend(blocks);
    host::set_panel("out", &all);
}

fn start(program: &str, args: &[&str], stdin: &str) {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    match host::start_job(program, &args, stdin) {
        Ok(id) => {
            LAST_JOB.store(id, Ordering::Relaxed);
            show("Running", vec![Block::Text(format!("job {id}: {program}"))]);
        }
        Err(why) => show("Refused", vec![Block::Text(why)]),
    }
}

fn ask(method: &str, params: &str) {
    match host::language_server_request(method, params) {
        Ok(id) => show("Asked", vec![Block::Text(format!("request {id}: {method}"))]),
        Err(why) => show("Refused", vec![Block::Text(why)]),
    }
}

fn job_finished(job: JobResult) {
    let code = job.exit_code.map_or("stopped".to_owned(), |c| c.to_string());
    let table = Table {
        headers: vec!["Job".to_owned(), "Exit".to_owned(), "Output".to_owned()],
        rows: vec![vec![job.id.to_string(), code, job.stdout.trim().to_owned()]],
    };
    let label = Span { text: "Finished ".to_owned(), color: Some(0x89D185), bold: true, monospace: false };
    let detail = Span { text: "with a table and a diff".to_owned(), color: None, bold: false, monospace: true };
    show("Job", vec![
        Block::Spans(vec![label, detail]),
        Block::Table(table),
        Block::Diff("--- before\n+++ after\n-old line\n+new line\n same line".to_owned()),
    ]);
}

fn answered(response: ServerResponse) {
    match response.error {
        Some(why) => show("Server said no", vec![Block::Text(why)]),
        None => show("Server answered", vec![Block::Code(response.answer)]),
    }
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, _argument: String) -> Result<(), String> {
        match id.as_str() {
            "version" => start("python", &["--version"], ""),
            "slow" => start("python", &["-c", "import time; time.sleep(5); print('SLOW-DONE')"], ""),
            "long" => start("python", &["-c", "import time; time.sleep(60); print('LONG-DONE')"], ""),
            "cancel" => host::cancel_job(LAST_JOB.load(Ordering::Relaxed)),
            "stdin" => start("python", &["-c", "import sys; print(sys.stdin.read().upper())"], "piped text"),
            "whoami" => start("whoami", &[], ""),
            "unlisted" => start("cmd", &["/c", "echo not allowed"], ""),
            "server" => ask("textDocument/codeLens", "{}"),
            "custom" => ask("fake/customMethod", "{\"question\": 6}"),
            _ => {}
        }
        Ok(())
    }

    fn on_event(event: Event) {
        match event {
            Event::JobFinished(job) => job_finished(job),
            Event::ServerAnswered(response) => answered(response),
            _ => {}
        }
    }
}

export!(Plugin);
