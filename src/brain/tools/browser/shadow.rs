//! Shadow-DOM aware JS source builders for the browser tools.
//!
//! Every selector path in this module used to resolve against the light
//! DOM only: `document.querySelectorAll` never enters a shadow root, and
//! CDP's `DOM.querySelector` rooted at the document node does not cross a
//! shadow boundary either. On a web-component-heavy page that made every
//! control inside a custom element invisible to `browser_find` and
//! unreachable for `browser_click` / `browser_type` / `browser_act`.
//!
//! This module owns the JS half of the fix: a small preamble of helpers
//! that walk the *composed* tree (light DOM plus every OPEN shadow root)
//! instead of a single document. The CDP half lives in
//! [`super::manager::resolve_element`], which is the only path that can
//! reach a CLOSED shadow root — `el.shadowRoot` is `null` for a closed
//! root by spec, so JS structurally cannot enumerate one, while
//! `DOM.getDocument { pierce: true }` returns it.
//!
//! Everything here is pure: fns return JS source strings and perform no
//! I/O, so tests pin the emitted shape exactly like `build_find_js` does.

/// Hard cap on elements visited by one composed-tree walk. A page that
/// generates nodes faster than we consume them (infinite scroll, a
/// mutation-observer loop) must not hang the `Runtime.evaluate` call.
pub(crate) const MAX_WALK_NODES: usize = 20_000;

/// Hard cap on shadow roots collected in one pass. Cheap insurance on
/// top of the cycle guard: a root can only be visited once, but a page
/// can still host more roots than is worth enumerating.
pub(crate) const MAX_ROOTS: usize = 500;

/// The `__ocDeep` preamble: composed-tree helpers, declared as `const`s
/// in the caller's function scope rather than on `window`, so we leave no
/// globals behind on the page.
///
/// Emitted helpers (names pinned by `src/tests/browser_shadow_test.rs`):
/// - `__ocRoots()` — `[document, ...every open shadow root]`, cycle-guarded.
/// - `__ocQueryAll(sel, limit)` — `querySelectorAll` across all roots.
/// - `__ocQueryOne(sel)` — first match across all roots, or `null`.
/// - `__ocWalk()` — every Element in the composed tree, in per-root order.
/// - `__ocComposedContains(a, b)` — is `a` a composed-tree ancestor of `b`?
/// - `__ocHitTest(el, x, y)` — `elementFromPoint` scoped to `el`'s root.
/// - `__ocClearStamps()` — deep `data-opencrabs-match` cleanup.
/// - `__ocInShadow(el)` — does `el` live behind a shadow boundary?
pub(crate) fn deep_helpers_js() -> String {
    format!(
        r#"
        const __OC_MAX_NODES = {MAX_WALK_NODES};
        const __OC_MAX_ROOTS = {MAX_ROOTS};
        // Breadth-first collection of the document plus every OPEN shadow
        // root reachable from it. `querySelectorAll('*')` on a root does
        // NOT descend into nested shadow trees, so each node is scanned
        // exactly once across the whole pass — linear, not quadratic.
        // `seen` is the cycle guard: a root can never be queued twice.
        const __ocRoots = () => {{
            const roots = [document];
            const seen = new Set(roots);
            for (let i = 0; i < roots.length; i++) {{
                if (roots.length >= __OC_MAX_ROOTS) break;
                const hosts = roots[i].querySelectorAll('*');
                for (const h of hosts) {{
                    const sr = h.shadowRoot;
                    if (sr && !seen.has(sr)) {{ seen.add(sr); roots.push(sr); }}
                    if (roots.length >= __OC_MAX_ROOTS) break;
                }}
            }}
            return roots;
        }};
        // Deep querySelectorAll. An invalid selector is allowed to throw
        // exactly as `document.querySelectorAll` would, so callers that
        // report "invalid selector" keep reporting it.
        const __ocQueryAll = (sel, limit) => {{
            const out = [];
            for (const r of __ocRoots()) {{
                for (const el of r.querySelectorAll(sel)) {{
                    out.push(el);
                    if (out.length >= limit) return out;
                }}
            }}
            return out;
        }};
        const __ocQueryOne = (sel) => {{
            const r = __ocQueryAll(sel, 1);
            return r.length ? r[0] : null;
        }};
        // Every Element in the composed tree. Ordered document-first then
        // per shadow root; within a root it is exact document order.
        const __ocWalk = () => {{
            const out = [];
            for (const r of __ocRoots()) {{
                const base = r === document ? document.body : r;
                if (!base) continue;
                const w = document.createTreeWalker(base, NodeFilter.SHOW_ELEMENT);
                let n;
                while ((n = w.nextNode())) {{
                    out.push(n);
                    if (out.length >= __OC_MAX_NODES) return out;
                }}
            }}
            return out;
        }};
        // `Node.contains` stops at a shadow boundary, so climb the
        // composed tree by hand: parentNode inside a tree, then the
        // root's `host` to hop out of it.
        const __ocComposedContains = (a, b) => {{
            let n = b;
            let guard = 0;
            while (n && guard++ < __OC_MAX_ROOTS) {{
                if (n === a) return true;
                n = n.parentNode
                    || (n.getRootNode ? n.getRootNode().host : null)
                    || null;
            }}
            return false;
        }};
        // `document.elementFromPoint` RETARGETS to the shadow host, so a
        // shadow-inner element never equals its own hit result and would
        // be dropped as occluded. `ShadowRoot` implements
        // `DocumentOrShadowRoot`, so hit-test inside the element's own
        // root and the comparison is apples to apples.
        const __ocHitTest = (el, x, y) => {{
            const root = el.getRootNode();
            const from = (root && root.elementFromPoint) ? root : document;
            return from.elementFromPoint(x, y);
        }};
        const __ocClearStamps = () => {{
            for (const r of __ocRoots()) {{
                r.querySelectorAll('[data-opencrabs-match]').forEach(
                    el => el.removeAttribute('data-opencrabs-match'));
            }}
        }};
        const __ocInShadow = (el) => {{
            const r = el.getRootNode ? el.getRootNode() : document;
            return !!r && r !== document;
        }};
        "#
    )
}

/// Wrap a JS statement body in an arrow IIFE with the deep helpers in
/// scope. Used by the tools whose script is a standalone expression
/// (`content.rs`) or its own function body (`type_text.rs`, `act.rs`);
/// `find.rs` splices [`deep_helpers_js`] into its existing wrapper
/// instead, so it keeps one IIFE rather than two.
pub(crate) fn with_deep_helpers(body: &str) -> String {
    let helpers = deep_helpers_js();
    format!("(() => {{{helpers}\n{body}\n}})()")
}
