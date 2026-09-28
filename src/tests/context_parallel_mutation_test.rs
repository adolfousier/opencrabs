//! Parallel `session_context` mutations all persist (#600).
//!
//! `session_context` loads the store once on entry to `execute` and saves it
//! from five match arms, with nothing held across the pair. Two mutations in
//! one parallel batch therefore both read the same baseline and the later save
//! replaces the file outright — while BOTH return a success receipt carrying
//! their own stale count. That false receipt is what makes this worse than a
//! lost write.
//!
//! `ContextStore::save` already writes a uniquely-named temp file and renames it
//! over the target (#906). That makes each write atomic and says nothing about
//! the window between the read and that write: uniqueness of the temp name is
//! orthogonal to lost updates.
//!
//! Measured on the real daemon before the fix (HEAD `cde5b4d4e`,
//! 2026-09-26T22:21Z): three `add_fact` calls in one parallel block, all three
//! reporting `Total facts: 2`, and 1 of 3 markers surviving on disk. The fix
//! takes the existing per-path advisory lock BEFORE the read — `path_lock`, the
//! same mechanism `edit.rs` uses for #593 and `mutate_plan` uses for #506.
//!
//! HARNESS NOTE. `with_profile_home_async` scopes a `tokio::task_local!`, and a
//! task-local is NOT inherited by `tokio::spawn`. A spawned leg therefore starts
//! with the REAL home and resolves a different store than the fixture seeded, so
//! every spawned leg re-enters the scope through `in_profile`. Same trap
//! `plan_mutation_lock_test.rs` documents for #506.
//!
//! Fixtures are synthetic and carry no user identifiers.

use crate::brain::tools::Tool;
use crate::brain::tools::ToolExecutionContext;
use crate::brain::tools::context::ContextTool;
use crate::config::profile::{home_for_profile, with_profile_home_async};
use std::path::PathBuf;
use uuid::Uuid;

/// A throwaway profile home, so nothing touches the real
/// `~/.opencrabs/agents/session/`. Removed on drop, including on a panicking
/// test.
struct TempProfile(String);

impl TempProfile {
    fn new() -> Self {
        Self(format!("ctx-parallel-mutation-test-{}", Uuid::new_v4()))
    }

    /// The profile name, owned so it can be moved into a spawned task.
    fn name(&self) -> String {
        self.0.clone()
    }

    /// Run `fut` with the profile home pointed at this profile.
    async fn scoped<F, T>(&self, fut: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        with_profile_home_async(Some(&self.0), fut).await
    }
}

impl Drop for TempProfile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(home_for_profile(Some(&self.0)));
    }
}

/// Enter `profile`'s home inside a SPAWNED task — see the harness note above.
async fn in_profile<F, T>(profile: String, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    with_profile_home_async(Some(&profile), fut).await
}

/// The store path `get_store_path` derives.
///
/// Duplicated because that fn is private to the tool module; kept in ONE place
/// here so a drift in the layout fails every leg at once rather than silently
/// pointing one of them at a different file.
fn store_path(sid: Uuid) -> PathBuf {
    crate::config::opencrabs_home()
        .join("agents")
        .join("session")
        .join(format!("context_{}.json", sid))
}

/// Read the persisted store as the ARTIFACT, not through the tool: a test that
/// asks the tool what it wrote can pass while the file disagrees.
fn facts_on_disk(sid: Uuid) -> Vec<String> {
    let path = store_path(sid);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("store {} must exist: {e}", path.display()));
    let v: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("store must parse: {e}"));
    v["facts"]
        .as_array()
        .expect("store must carry a facts array")
        .iter()
        .map(|f| f.as_str().unwrap_or_default().to_string())
        .collect()
}

/// The count an `add_fact` receipt reports, parsed from the receipt text.
fn receipt_total_facts(receipt: &str) -> usize {
    receipt
        .lines()
        .find_map(|l| l.strip_prefix("Total facts: "))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("receipt must carry 'Total facts: N', got: {receipt}"))
}

/// The issue's own reproduction: three `add_fact` calls dispatched together.
///
/// Before the fix all three loaded the same baseline, the last save won, and two
/// of three facts were gone while every call had already been handed a success
/// receipt reporting the same stale count.
///
/// These legs are real `tokio::spawn`s on a multi-thread runtime, so each gets
/// its own worker thread and a blocking wait cannot starve a sibling. That
/// makes this the WEAKER of the two legs: it passed on the deployed build whose
/// lock wait was still inline and still losing writes. The cooperative shape
/// the daemon actually uses is covered by
/// `parallel_mutations_all_persist_in_a_cooperative_batch` below — the leg that
/// fails when the wait goes back inline.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn parallel_mutations_all_persist() {
    let tp = TempProfile::new();
    tp.scoped(async {
        let sid = Uuid::new_v4();

        // Seed one fact sequentially, so each racing call is a read-modify-write
        // on an existing document — the measured repro started from a one-fact
        // store and saw "Total facts: 2" on every call.
        let seed = ContextTool
            .execute(
                serde_json::json!({ "operation": "add_fact", "fact": "SEED" }),
                &ToolExecutionContext::new(sid),
            )
            .await
            .expect("seed add_fact");
        assert!(seed.success, "seed failed: {:?}", seed.error);
        assert_eq!(receipt_total_facts(&seed.output), 1);

        const N: usize = 3;
        // Release all three legs at the same instant, so their read-modify-write
        // windows genuinely overlap rather than depending on spawn timing. A
        // regression leg that only sometimes exercises the race is not a leg.
        let gate = std::sync::Arc::new(tokio::sync::Barrier::new(N));
        let mut handles = Vec::new();
        for i in 0..N {
            let gate = gate.clone();
            handles.push(tokio::spawn(in_profile(tp.name(), async move {
                gate.wait().await;
                let ctx = ToolExecutionContext::new(sid);
                ContextTool
                    .execute(
                        serde_json::json!({
                            "operation": "add_fact",
                            "fact": format!("PARALLEL-{i}"),
                        }),
                        &ctx,
                    )
                    .await
            })));
        }

        let mut receipts = Vec::new();
        for h in handles {
            let r = h
                .await
                .expect("a leg panicked")
                .expect("execute returned Err");
            assert!(
                r.success,
                "a parallel add_fact reported failure: {:?}",
                r.error
            );
            receipts.push(r.output);
        }

        // Every call reported success, so every mutation must be on disk.
        let facts = facts_on_disk(sid);
        assert_eq!(
            facts.len(),
            N + 1,
            "{} of {N} parallel mutations persisted (seed + winners = {:?})",
            facts.len().saturating_sub(1),
            facts
        );
        for i in 0..N {
            let want = format!("PARALLEL-{i}");
            assert!(
                facts.contains(&want),
                "{want} was lost — store holds {facts:?}"
            );
        }

        // Distinct counts are the PROOF of serialisation, not a nicety. Each
        // receipt reports the store's size at ITS commit, so three identical
        // counts mean three calls shared one baseline — which is the defect, and
        // its measured symptom was exactly that: "Total facts: 2" on every call.
        let mut counts: Vec<usize> = receipts.iter().map(|r| receipt_total_facts(r)).collect();
        counts.sort_unstable();
        assert_eq!(
            counts,
            vec![2, 3, 4],
            "receipts reported {counts:?} — equal counts mean the calls shared a \
             baseline and only one mutation survived"
        );
    })
    .await;
}

/// The lock is deliberately advisory: a contended writer waits, then proceeds.
/// What must never happen is proceeding SILENTLY, so the notice is the second
/// half of the fix and this leg pins it deterministically — by holding the lock
/// ourselves rather than hoping for a race.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_contended_write_is_reported_not_swallowed() {
    let tp = TempProfile::new();
    tp.scoped(async {
        let sid = Uuid::new_v4();

        // Seed first, so the store file exists and both sides resolve the same
        // canonical lock key.
        let seed = ContextTool
            .execute(
                serde_json::json!({ "operation": "add_fact", "fact": "SEED" }),
                &ToolExecutionContext::new(sid),
            )
            .await
            .expect("seed add_fact");
        assert!(seed.success, "seed failed: {:?}", seed.error);

        // Hold the store's lock ourselves, so the tool cannot take it.
        let held = crate::brain::tools::path_lock::acquire(&store_path(sid));
        assert!(
            held.as_ref().is_some_and(|l| l.is_held()),
            "fixture could not take the lock, so this leg would prove nothing"
        );

        let r = ContextTool
            .execute(
                serde_json::json!({ "operation": "add_fact", "fact": "CONTENDED" }),
                &ToolExecutionContext::new(sid),
            )
            .await
            .expect("execute");

        assert!(
            r.success,
            "a contended write still completes rather than failing: {:?}",
            r.error
        );
        assert!(
            r.output.contains("another agent was writing"),
            "a contended write must report the overlap, got: {}",
            r.output
        );
        assert_eq!(receipt_total_facts(&r.output), 2);

        drop(held);
        assert_eq!(facts_on_disk(sid).len(), 2);
    })
    .await;
}

/// The production scheduler: `parallel_tools.rs` drives a batch with
/// `stream::iter(..).map(..).buffered(max_concurrent)`, which polls every
/// future on ONE task.
///
/// The sibling leg above drives its legs on separate worker threads, so it
/// cannot see a defect that only appears under cooperative scheduling — and
/// the lock's wait IS such a defect: it is a blocking sleep, so on one task it
/// stalls the sibling future holding the lock, the holder never finishes, and
/// every waiter burns its full deadline and then proceeds unlocked.
///
/// Measured on the deployed `13342973e` (2026-09-27T00:19Z), which is why this
/// leg exists: three mutations in one batch, all three read the same baseline
/// and reported `Total facts: 4`, two facts lost, and the batch elapsed 1.72 s
/// against a holder that could not progress. A regression to an inline wait
/// fails here rather than in production.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn parallel_mutations_all_persist_in_a_cooperative_batch() {
    use futures::StreamExt;

    let tp = TempProfile::new();
    tp.scoped(async {
        let sid = Uuid::new_v4();

        let seed = ContextTool
            .execute(
                serde_json::json!({ "operation": "add_fact", "fact": "SEED-BATCH" }),
                &ToolExecutionContext::new(sid),
            )
            .await
            .expect("seed add_fact");
        assert!(seed.success, "seed failed: {:?}", seed.error);
        assert_eq!(receipt_total_facts(&seed.output), 1);

        const N: usize = 3;
        // No barrier and no spawn: `buffered` polls all N futures in the first
        // poll cycle, so their read-modify-write windows overlap by
        // construction — the same way the daemon starts a batch.
        let receipts: Vec<_> = futures::stream::iter(0..N)
            .map(|i| async move {
                ContextTool
                    .execute(
                        serde_json::json!({
                            "operation": "add_fact",
                            "fact": format!("BATCH-{i}"),
                        }),
                        &ToolExecutionContext::new(sid),
                    )
                    .await
            })
            .buffered(N)
            .collect()
            .await;

        for r in &receipts {
            let r = r.as_ref().expect("execute returned Err");
            assert!(
                r.success,
                "a batched add_fact reported failure: {:?}",
                r.error
            );
        }

        let facts = facts_on_disk(sid);
        assert_eq!(
            facts.len(),
            N + 1,
            "{} of {N} batched mutations persisted (seed + winners = {:?})",
            facts.len().saturating_sub(1),
            facts
        );
        for i in 0..N {
            let want = format!("BATCH-{i}");
            assert!(facts.contains(&want), "{want} lost; store holds {facts:?}");
        }
    })
    .await;
}
