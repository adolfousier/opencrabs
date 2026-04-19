//! Test that `BrowserManager`'s Drop impl is safe on a never-launched
//! manager — the common case at app exit when nothing triggered a
//! browser launch during the session.
//!
//! We can't easily test the "launched-and-then-dropped" path without a
//! real Chrome binary (chromiumoxide's `Browser` has no test
//! constructor), but the never-launched branch covers the one that
//! matters most for process shutdown hygiene: a manager Arc with no
//! handler task and no Browser handle must drop cleanly without panic.

use crate::brain::tools::browser::BrowserManager;

#[test]
fn drop_never_launched_manager_does_not_panic() {
    let mgr = BrowserManager::new();
    drop(mgr); // explicit drop — Drop impl runs with browser=None, handler=None
}

#[test]
fn drop_cloned_managers_is_safe() {
    // Tools hold cloned Arcs of BrowserManager. Dropping the clones
    // must not double-abort or panic when the final clone goes away.
    let mgr = BrowserManager::new();
    let clone1 = mgr.clone();
    let clone2 = mgr.clone();
    drop(clone1);
    drop(clone2);
    drop(mgr); // final clone — ManagerInner Drop actually runs here
}
