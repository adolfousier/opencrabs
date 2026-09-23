//! Class guard for the config-freeze family (#1696, #1700; ancestor #1249).
//!
//! Five issues in this family have been closed separately (#262, #1155, #1249,
//! #1317, #1318) and every closure looked complete, because each test pinned
//! one path. #1249's own test file covers a directly-constructed `AgentService`
//! and never mentions RSI or `ChannelFactory`, which is why the RSI snapshot
//! (#1696) and the channel freeze (#1700) stayed open a month after that fix.
//!
//! The class is not "someone cloned a `Config`". It is: a long-lived task or
//! struct holds something DERIVED from one config read, past the next change
//! notification. Two derived shapes exist in this tree:
//!
//!   (a) a `Config` captured by a spawned loop, which is #1696 (RSI); and
//!   (b) an `Arc<dyn Provider>` field built once, which is #1700 (channels) and
//!       also the single line that froze A2A, WhatsApp/Trello and the
//!       secondary-profile cron factory, all of which reach their provider
//!       through `ChannelFactory` rather than through a `Config`.
//!
//! A census on shape (a) alone passes today and pins nothing. Both halves are
//! asserted here so a sixth fragment cannot land the way the third and fourth
//! did. Known gap: a tuple struct holding a bare provider is caught by the
//! declaration-line scan, but a provider reached through a type alias is not.

use std::path::{Path, PathBuf};

fn read_src(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    // An unreadable directory is skipped on purpose. The guard's own
    // file-count assertion is what catches a moved or renamed path.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rs_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
}

/// `(name, text)` for every struct declaration in `src`, braced bodies included
/// in full so a field cannot hide behind a line ending mid-declaration.
fn struct_decls(src: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim_start();
        let toks: Vec<&str> = trimmed.split_whitespace().collect();
        let Some(pos) = toks.iter().position(|tok| *tok == "struct") else {
            continue;
        };
        if !toks[..pos]
            .iter()
            .all(|t| t.starts_with("pub") || *t == "unsafe")
        {
            continue;
        }
        let Some(name) = toks.get(pos + 1).map(|s| s.to_string()) else {
            continue;
        };
        if raw[..raw.find("struct").unwrap_or(0)].contains("//") {
            continue;
        }
        if !raw.contains('{') {
            // Tuple or unit struct: no `name: Type` lines exist to scan, so
            // synthesize one per element. Without this, `struct P(Arc<dyn
            // Provider>);` slips through the field scan while the doc comment
            // claims it is caught.
            let mut synth = String::new();
            if let (Some(open), Some(close)) = (raw.find('('), raw.rfind(')'))
                && close > open
            {
                for (n, field) in raw[open + 1..close].split(',').enumerate() {
                    let field = field.trim();
                    if !field.is_empty() {
                        synth.push_str(&format!("{n}: {field},\n"));
                    }
                }
            }
            out.push((name, synth));
            continue;
        }
        let mut depth: i32 = 0;
        let mut started = false;
        let mut body = String::new();
        for line in lines[i..].iter() {
            for ch in line.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        started = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            body.push_str(line);
            body.push('\n');
            if started && depth <= 0 {
                break;
            }
        }
        out.push((name, body));
    }
    out
}

fn normalized_field_type(line: &str) -> Option<String> {
    let t = line.trim();
    if t.starts_with("//") || t.starts_with('*') || t.contains("=>") || t.starts_with("fn ") {
        return None;
    }
    let colon = t.find(':')?;
    let after = t[colon + 1..].trim();
    if after.is_empty() || after.contains('(') && !after.contains('>') {
        return None;
    }
    Some(after.chars().filter(|c| !c.is_whitespace()).collect())
}

fn is_bare_provider(n: &str) -> bool {
    let n = n.trim_end_matches(',');
    n.starts_with("Arc<dynProvider>") || n.starts_with("Box<dynProvider>")
}

/// Shape (b): no struct in `src/channels/` may hold a provider as a bare `Arc`.
///
/// This is the assertion that #1700's own text does not ask for, and it is the
/// one that would have caught A2A, WhatsApp/Trello and the secondary-profile
/// cron factory at once: they share the factory's field rather than a Config.
#[test]
fn no_channel_struct_holds_a_bare_provider_arc() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/channels");
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    assert!(
        files.len() > 5,
        "expected to scan src/channels/, found only {} .rs files — the guard \
         would be vacuous if the path moved",
        files.len()
    );

    let mut offenders = Vec::new();
    for file in &files {
        let src = std::fs::read_to_string(file).unwrap_or_default();
        let rel = file
            .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
            .unwrap_or(file)
            .display()
            .to_string();
        for (name, body) in struct_decls(&src) {
            for line in body.lines() {
                if let Some(ty) = normalized_field_type(line)
                    && is_bare_provider(&ty)
                {
                    offenders.push(format!("{rel}: struct {name} -> {}", line.trim()));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "#1700 class guard: a bare provider field is built once and can never \
         be reloaded. Wrap it (RwLock/Mutex) or reach it through \
         ChannelFactory's slot. Offenders:\n{}",
        offenders.join("\n")
    );
}

/// Shape (a): the RSI engine must not capture a `Config` in its spawned loop.
#[test]
fn rsi_engine_takes_no_config_and_reads_it_live() {
    let src = read_src("src/brain/rsi.rs");

    assert!(
        !src.contains("config_clone"),
        "#1696: `config_clone` is the snapshot that froze the RSI provider and \
         its fallback chain at process start"
    );

    // The spawner must not be handed a Config at all. A parameter is how the
    // snapshot got in, and removing it is what makes the regression impossible
    // to reintroduce by accident.
    let start = src
        .find("fn spawn_rsi_engine(")
        .expect("spawn_rsi_engine must exist");
    let brace = src[start..].find('{').expect("spawn_rsi_engine body") + start;
    let sig = &src[start..brace];
    assert!(
        !sig.to_lowercase().contains("config"),
        "#1696: spawn_rsi_engine must not accept a Config; the cycle reads the \
         live mirror instead. Signature: {sig}"
    );

    // The cycle must be fed from Config::current(), read once per boundary so a
    // save landing mid-cycle cannot tear the provider against the chain. The
    // CALL site, not the definition: skip any occurrence introduced by `fn`.
    let call = src
        .match_indices("run_rsi_agent_cycle(")
        .map(|(i, _)| i)
        .find(|&i| !src[..i].trim_end().ends_with("fn"))
        .expect("the RSI cycle must be called");
    let near: Vec<&str> = src[..call].lines().rev().take(6).collect();
    let near = near.join("\n");
    assert!(
        near.contains("Config::current()"),
        "#1696: the cycle config must come from Config::current(); window: {near}"
    );
}

/// The reload must have more than one production caller. One caller is the
/// exact state that let #1249 close while RSI and channels stayed frozen.
#[test]
fn fallback_reload_has_production_callers_beyond_the_tui() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&root, &mut files);

    let mut sites = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
            .unwrap_or(file)
            .display()
            .to_string();
        if rel.contains("/tests/") || rel.ends_with("/tests.rs") {
            continue;
        }
        let src = std::fs::read_to_string(file).unwrap_or_default();
        for (i, line) in src.lines().enumerate() {
            if line.contains(".reload_fallback_providers(") {
                sites.push(format!("{rel}:{}", i + 1));
            }
        }
    }

    assert!(
        sites.len() >= 2,
        "#1249 left exactly one production caller (the TUI service). A second \
         is required so channel/A2A/cron agents reload too; found {}: {:?}",
        sites.len(),
        sites
    );
}

/// The registry is the only way an agent leaves the factory, so registration
/// cannot be bypassed by a new constructor.
#[test]
fn every_factory_builder_registers_and_only_one_constructs() {
    let src = read_src("src/channels/factory.rs");

    for marker in ["Arc::downgrade", "live_agents"] {
        assert!(
            src.contains(marker),
            "#1700: the factory must keep a Weak registry ({marker} missing) or \
             nothing can reload the agents it built"
        );
    }

    // Only `_full` may construct. A second `AgentService::new` elsewhere in the
    // file would produce an unregistered agent, which is the freeze again.
    let constructions = src.matches("AgentService::new(").count();
    assert_eq!(
        constructions, 1,
        "#1700: exactly one construction point keeps the registry complete; \
         found {constructions}"
    );
}

/// The ConfigWatcher must actually call the reload. Wiring is what #1249 had
/// for one service and nothing else.
#[test]
fn config_watcher_hands_the_rebuild_to_the_factory() {
    let src = read_src("src/cli/ui.rs");
    assert!(
        src.contains(".reload_providers(&cfg, primary)"),
        "#1700: the watcher must rebuild the primary ONCE and pass the same Arc \
         to the TUI service and the factory registry"
    );
}

/// A guard that cannot fail is decoration. The detection itself is under test
/// here, with crafted inputs, so it is proven to fire on the shape it forbids
/// and to stay quiet on the shape the fix introduced. Without this, the scan
/// above would pass for as long as it matched nothing at all.
#[test]
fn bare_provider_scanner_detects_what_it_forbids() {
    let cases: &[(&str, bool)] = &[
        ("struct Bare { provider: Arc<dyn Provider>, }", true),
        (
            "struct Pub { pub(crate) provider: Arc<dyn Provider>, }",
            true,
        ),
        ("struct Boxed { provider: Box<dyn Provider>, }", true),
        ("struct Tup(Arc<dyn Provider>);", true),
        (
            "struct Locked { provider: std::sync::RwLock<Arc<dyn Provider>>, }",
            false,
        ),
        (
            "struct Guarded { provider: Mutex<Arc<dyn Provider>>, }",
            false,
        ),
        ("struct Plain { name: String, turns: usize, }", false),
        // A constructor parameter is not a field. ChannelFactory::new takes
        // exactly one, and the real scan must stay quiet about it.
        (
            "impl C { fn new(provider: Arc<dyn Provider>) -> Self { todo!() } }",
            false,
        ),
    ];

    for (src, expect_offender) in cases {
        let found = struct_decls(src).into_iter().any(|(_name, body)| {
            body.lines()
                .filter_map(normalized_field_type)
                .any(|ty| is_bare_provider(&ty))
        });
        assert_eq!(
            found, *expect_offender,
            "scanner returned {found}, expected {expect_offender} for: {src}"
        );
    }
}
