//! On-demand live eval runner (umbrella #628). Resolves `agent.eval_providers`
//! into real providers, builds a majority-vote judge panel, produces real
//! artifacts, and grades them. Makes real API calls — run only on demand:
//!
//!   cargo run --example live_eval --features eval
//!
//! Never part of CI.

use std::sync::Arc;

use opencrabs::brain::prompt_builder::{BrainLoader, RuntimeInfo};
use opencrabs::brain::provider::Provider;
use opencrabs::brain::tools::catalog::tool_access_prompt;
use opencrabs::config::Config;
use opencrabs::eval::compaction::CompactionDataset;
use opencrabs::eval::live::resolve_eval_providers;
use opencrabs::eval::panel::panel_from_providers;
use opencrabs::eval::produce::{eval_tool_set, produce_compaction_summary, produce_response};
use opencrabs::eval::runner::VarianceReport;
use opencrabs::eval::self_awareness::SelfAwarenessScenario;

#[tokio::main]
async fn main() {
    let config = Config::load().expect("load config");
    let providers = resolve_eval_providers(&config).await;
    println!("Resolved {} eval provider(s):", providers.len());
    for p in &providers {
        println!("  - {} / {}", p.name(), p.default_model());
    }
    if providers.is_empty() {
        eprintln!("No eval providers resolved — check agent.eval_providers and keys.toml.");
        return;
    }

    // Producer = first provider (the "model under test"); judges = full panel.
    let producer: Arc<dyn Provider> = providers[0].clone();
    let producer_model = producer.default_model().to_string();
    let panel = panel_from_providers(&providers);

    // Build the REAL OpenCrabs system brain so the producer answers with its
    // actual runtime context (compiled-features line + SELF-AWARENESS / VOICE
    // directives), not a bare prompt — the whole point of measuring
    // OpenCrabs-with-the-preamble rather than the raw model.
    let loader = BrainLoader::new(opencrabs::config::opencrabs_home());
    let rt = RuntimeInfo {
        model: Some(producer_model.clone()),
        provider: Some(producer.name().to_string()),
        working_directory: None,
    };
    let system_brain = loader.build_core_brain(Some(&rt));
    let sys = Some(system_brain.as_str());
    println!(
        "\nProducer: {} / {}   Judge panel: {} model(s)   System brain: {} chars\n",
        producer.name(),
        producer_model,
        panel.len(),
        system_brain.len()
    );

    // Repeat each eval K times — a single live run is noise (the producer is
    // non-deterministic), so we report mean AND median (robust to a lone crater)
    // plus the catastrophic failure rate, and dump outlier artifacts for
    // inspection rather than averaging them silently into the score.
    const K: usize = 5;
    // A run whose panel score falls to or below this is a catastrophic failure,
    // reported separately from the typical-quality median.
    const CATASTROPHIC: f64 = 0.5;
    let outlier_path = opencrabs::config::opencrabs_home().join("eval_outliers.md");

    // 1. Compaction fidelity.
    let ds = CompactionDataset::seed();
    println!("== Compaction fidelity ({}), {K} runs ==", ds.name);
    let (mut kw, mut pn) = (Vec::new(), Vec::new());
    for i in 0..K {
        let summary =
            produce_compaction_summary(producer.as_ref(), &producer_model, &ds.messages(), sys)
                .await;
        let (k, p) = (
            ds.keyword_scorecard(&summary).overall(),
            ds.judge_scorecard(&panel, &summary).await.overall(),
        );
        println!(
            "  run {i}: {} chars  keyword={k:.2}  panel={p:.2}",
            summary.len()
        );
        if p <= CATASTROPHIC {
            save_outlier(&outlier_path, &ds.name, i, p, &summary);
        }
        kw.push(k);
        pn.push(p);
    }
    report("compaction", &kw, &pn, CATASTROPHIC);

    // 2. Capability self-awareness — produced UNDER the real system brain.
    // Give the producer its real tools so it can DO the right thing — call
    // config_manager / tool_search, or read the persisted attachment — instead
    // of narrating. Every scenario runs under BOTH tool modes: non-lazy (the
    // plain brain, all schemas) and lazy (the brain with the tool_access roster
    // appended, as live_system_brain does when lazy_tools is on), so a
    // mode-specific gap — tool-set awareness especially — is visible (#672).
    let tools = eval_tool_set();
    let lazy_brain = format!("{system_brain}{}", tool_access_prompt(false));
    let modes: [(&str, Option<&str>); 2] = [("non-lazy", sys), ("lazy", Some(lazy_brain.as_str()))];
    for (mode, brain) in modes {
        for sc in SelfAwarenessScenario::seeds() {
            println!("\n== Self-awareness [{mode}] ({}), {K} runs ==", sc.name);
            let (mut kw, mut pn) = (Vec::new(), Vec::new());
            for i in 0..K {
                let response = produce_response(
                    producer.as_ref(),
                    &producer_model,
                    &sc.prompt,
                    brain,
                    &tools,
                )
                .await;
                let kw_card = sc.keyword_scorecard(&response);
                let (k, p) = (
                    kw_card.overall(),
                    sc.judge_scorecard(&panel, &response).await.overall(),
                );
                println!(
                    "  run {i}: {} chars  keyword={k:.2}  panel={p:.2}",
                    response.len()
                );
                if i == 0 {
                    println!(
                        "    run0 head: {}",
                        response[..response.len().min(280)].replace('\n', " ")
                    );
                    for (q, v) in &kw_card.results {
                        println!(
                            "      keyword [{}] {} — {}",
                            if v.yes { "PASS" } else { "FAIL" },
                            q.dimension,
                            v.explanation.as_deref().unwrap_or("")
                        );
                    }
                }
                if p <= CATASTROPHIC {
                    save_outlier(
                        &outlier_path,
                        &format!("{mode}:{}", sc.name),
                        i,
                        p,
                        &response,
                    );
                }
                kw.push(k);
                pn.push(p);
            }
            report(
                &format!("self-awareness:{mode}:{}", sc.name),
                &kw,
                &pn,
                CATASTROPHIC,
            );
        }
    }

    println!(
        "\nOutlier artifacts (if any) appended to {}",
        outlier_path.display()
    );
}

/// Print the keyword + panel variance, plus the panel failure rate — so
/// typical quality (median) and reliability (failure rate) read separately.
fn report(name: &str, keyword: &[f64], panel: &[f64], threshold: f64) {
    println!(
        "  keyword {}",
        VarianceReport::from_scores(keyword).render()
    );
    println!("  panel   {}", VarianceReport::from_scores(panel).render());
    println!(
        "  panel failure rate (<= {threshold:.2}): {:.0}%   [{name}]",
        VarianceReport::failure_rate(panel, threshold) * 100.0
    );
}

/// Append a cratered run's artifact so we can read WHY it failed instead of
/// guessing (genuine terse output vs a length cap, etc.).
fn save_outlier(path: &std::path::Path, dataset: &str, run: usize, score: f64, artifact: &str) {
    let entry = format!(
        "\n## {dataset} — run {run} — panel {score:.2} — {} chars\n\n{artifact}\n",
        artifact.len()
    );
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(mut f) => {
            if let Err(e) = f.write_all(entry.as_bytes()) {
                eprintln!("failed to write outlier artifact: {e}");
            }
        }
        Err(e) => eprintln!("failed to open outlier file {}: {e}", path.display()),
    }
}
