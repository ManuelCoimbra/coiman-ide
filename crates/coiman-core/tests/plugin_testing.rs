//! The plugin test runner (`coiman-plugin test`) on the word-count example, its call times,
//! and a failing test's report.

use std::path::PathBuf;

use coiman_core::extensions::plugin::testing;

#[test]
fn the_runner_passes_and_fails_as_it_should() {
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples/word-count");
    let outcomes = testing::run(&example).expect("tests.toml runs");
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|o| o.failure.is_none()), "{outcomes:?}");

    // A copy whose test expects what isn't there.
    let tmp = std::env::temp_dir().join(format!("coiman-plugin-testing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    for f in ["plugin.toml", "plugin.wasm"] {
        std::fs::copy(example.join(f), tmp.join(f)).unwrap();
    }
    std::fs::write(
        tmp.join("tests.toml"),
        "[[test]]\nname = \"wrong\"\ndocument = { path = \"a.txt\", text = \"one two\" }\n\
         [[test.step]]\ncommand = \"report\"\n[[test.step]]\npanel = \"9 words\"\ntimeout = 0.5\n",
    )
    .unwrap();
    let outcomes = testing::run(&tmp).unwrap();
    let why = outcomes[0].failure.as_deref().expect("it fails");
    assert!(why.starts_with("step 2: the panel never had \"9 words\"") && why.contains("2 words, 1 lines"), "{why}");
    std::fs::write(tmp.join("tests.toml"), "").unwrap();
    assert_eq!(testing::run(&tmp).unwrap_err(), "tests.toml has no [[test]]");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn calls_are_timed() {
    use coiman_core::extensions::plugin::{PluginHost, Update, registry};
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/examples/word-count");
    let pkg = registry::read_folder(&example).unwrap();
    let id = pkg.manifest.id.clone();
    let host = PluginHost::start(|| {}).unwrap();
    host.load(pkg.manifest.clone(), pkg.wasm, pkg.manifest.permissions.clone());
    let start = std::time::Instant::now();
    while !host.poll().iter().any(|u| matches!(u, Update::Activated { .. })) {
        assert!(start.elapsed().as_secs() < 30, "never activated");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    host.run_command(&id, "report", "");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let t = host.timings()[&id];
    assert!(t.calls >= 2, "activation and the command: {t:?}");
    assert!(t.slowest <= t.total && !t.total.is_zero(), "{t:?}");
}
