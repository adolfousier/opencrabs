---
name: browser-cdp
description: "Native CDP browser automation reference. Headless/headed Chrome control, screenshots, JS evaluation. (/browser-cdp, browser automation, cdp, scraping)"
---

# Native Browser Automation (CDP)

Built-in browser control via Chrome DevTools Protocol — no Node.js, no Playwright, pure Rust.

## Tools

| Tool | Required | Optional | What |
|------|----------|---------|------|
| `browser_navigate` | `url` | `headless` | Navigate to URL |
| `browser_click` | `selector` | — | Click element by CSS selector |
| `browser_type` | `text` | `selector` | Type text into input |
| `browser_screenshot` | — | `selector` | Screenshot (returns file path) |
| `browser_eval` | `script` | — | Execute JavaScript |
| `browser_content` | — | `selector`, `text_only` | Extract text/HTML |
| `browser_wait` | — | `selector`, `timeout_secs`, `delay_secs` | Wait for element or delay |
| `browser_find` | — | `pattern`, `mode`, `limit` | Inventory (no `pattern`) or find elements |

## Shadow DOM

Selector resolution is shadow-DOM aware by default — nothing to enable.

| Path | Open shadow roots | Closed shadow roots |
|------|-------------------|---------------------|
| `browser_find` modes `css`, `text`, `aria`, and the no-pattern inventory | Searched; hits are marked `[shadow]` | Not enumerable — `el.shadowRoot` is `null` for a closed root by spec |
| `browser_find` mode `xpath` | Not searched | Not searched |
| `browser_click`, `browser_wait`, `browser_screenshot`, `browser_act` (click) | Resolved | Resolved (CDP pierces where JS cannot) |
| `browser_type`, `browser_act` (fill / select / pre-flight) | Resolved | Not resolved (JS path) |
| `browser_content` with a `selector` | Resolved | Not resolved (JS path) |
| `browser_content` with no `selector` | Light DOM only (uncapped output; piercing it would be an output-sizing change) | Light DOM only |

Rules of thumb:

- **XPath cannot cross a shadow boundary.** That is an XPath spec limit, not
  an OpenCrabs one. On a web-component page use `css` or `text` mode instead.
- **A closed root resolves but never enumerates.** If you know the selector,
  `browser_click` reaches it over CDP; `browser_find` will never show it to you.
- **Plain lookups run first.** A page with no shadow DOM pays zero extra CDP
  round-trips, so nothing gets slower for the common case.
- A hand-written CSS selector for a shadow element will not work in a raw
  `browser_eval` (`document.querySelector` stops at the boundary) even though it
  works in `browser_click`. Take the `[data-opencrabs-match="N"]` selector from
  `browser_find` instead of composing your own.

## Headless vs Headed

- **Headless (default):** No visible window. Fast, low resources.
- **Headed:** Visible Chrome window. Pass `headless: false` to `browser_navigate`.

## Usage Tips

- Compose workflows: navigate → inventory/find → click → type → screenshot
- **Inventory mode:** call `browser_find` with NO `pattern` to get every visible
  interactive element on the page, each with a stable `[data-opencrabs-match="N"]`
  selector. Prefer this over `browser_screenshot` when you have just landed and do
  not yet know what to click — text beats pixels for deciding where to act.
- Screenshots return file paths. Use `<<IMG:path>>` to send to channels.
- JavaScript evaluation for complex DOM manipulation and SPAs.
- No credentials stored. Navigate to login page and use click/type for auth (with user approval).
- Requires Chrome/Chromium installed. Feature-gated: `browser` flag.

## Chrome Launch Failures

1. **SingletonLock exists:** `rm -f ~/.opencrabs/chrome-profile/SingletonLock ~/.opencrabs/chrome-profile/SingletonCookie ~/.opencrabs/chrome-profile/SingletonSocket` then retry.
2. **DevTools data dir conflict:** `pkill -f chrome` then retry.
3. **Channel closed:** Kill Chrome, delete lock files, retry.
