//! The always-loaded preamble teaches file splitting and delegation (#1352):
//! a 40 KB single-file write is a minutes-long tool-call argument that gets
//! cut, and the answer to a turn that reasons for many minutes is a sub-agent
//! or background task, never a longer timeout.

use crate::brain::prompt_builder::BRAIN_PREAMBLE;
use crate::utils::prompt_analyzer::PromptAnalyzer;

#[test]
fn the_preamble_carries_the_split_rule_with_its_cap() {
    let block = BRAIN_PREAMBLE
        .split("LARGE FILES ARE WRITTEN IN PARTS")
        .nth(1)
        .expect("the block exists");
    assert!(block.contains("~300 lines"), "the cap is stated");
    assert!(
        block.contains("importmap"),
        "vendored libraries bypass the model"
    );
    assert!(
        block.contains("edit_file"),
        "single-file HTML grows by inserts"
    );
    assert!(block.contains("`wc -c`"), "the written file is checked");
}

#[test]
fn the_preamble_says_long_work_is_delegated_not_stretched() {
    let block = BRAIN_PREAMBLE
        .split("LARGE FILES ARE WRITTEN IN PARTS")
        .nth(1)
        .expect("the block exists");
    assert!(block.contains("spawn_agent"));
    assert!(block.contains("never a longer timeout"));
}

#[test]
fn the_block_stays_within_its_budget() {
    // #779 tracks preamble bloat; this block was filed at ~15 lines.
    let block = BRAIN_PREAMBLE
        .split("LARGE FILES ARE WRITTEN IN PARTS")
        .nth(1)
        .expect("the block exists")
        .split("UPDATES — the normal path")
        .next()
        .expect("the next block follows");
    let lines = block.lines().filter(|l| !l.trim().is_empty()).count();
    assert!(lines <= 15, "{lines} lines");
}

#[test]
fn the_write_hint_carries_the_cap_too() {
    let result = PromptAnalyzer::new()
        .analyze_and_transform("create a file index.html with a three.js scene");
    assert!(result.contains("TOOL HINT"));
    assert!(result.contains("`write_file` tool"));
    assert!(result.contains("~300 lines"), "{result}");
}
