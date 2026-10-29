# Making a Notion shell fast and native

Research notes behind Renotion's architecture. The goal was not "ship a
Notion clone", but "make notion.so feel like a native app while staying as
small and predictable as possible".

## 1. Runtime choice: system webview, not Electron

Electron gives you a known Chromium and Node.js, but every wrapper ships its
own browser. A minimal Electron app is routinely >100 MB on disk and uses a
few hundred MB of RAM before the web app loads. Tauri 2 uses the webview that
is already part of the OS:

- Linux: WebKitGTK
- macOS: WKWebView
- Windows: WebView2 (Chromium/Edge)

That removes the bundled browser and the Node runtime, and lets the OS share
the webview process and caches. For a site as heavy as Notion, the renderer is
the expensive part; bundling a second Chromium only makes it more expensive.

Alternative considered: a real Chromium embedded via CEF. It would make the
rendering engine identical on every platform, but the install size and
memory cost are the Electron problem again. Not worth it for one site.

## 2. Do not build a frontend at all

The usual Tauri project is a Rust shell plus a Vite/React app. A wrapper does
not need one. Tauri's `frontendDist` accepts a remote URL and, in that case,
**no assets are embedded** in the binary
([Tauri config reference](https://v2.tauri.app/reference/config)).

So `frontendDist` is `https://www.notion.so/`, the main window is created with
`WebviewUrl::External`, and the repository has no `package.json`,
`node_modules`, bundler config or generated `dist/`.

Benefits:

- no JS toolchain, no supply-chain surface from npm;
- no double navigation (local splash → remote app);
- no extra HTML/JS to parse at startup;
- the webview's HTTP cache and Notion's service worker are used exactly as in
  a browser.

A local "offline/retry" page was considered. To show it on a network failure we
would have to either load a local shell page and iframe Notion (Notion sends
frame-blocking headers), or detect the failure from Rust with an extra HTTP
stack. Both add complexity for an edge case. The webview's own error page is
acceptable.

## 3. Keep IPC off the remote origin

The web app is remote code. Granting it Tauri IPC would mean any XSS or
compromised third-party script in Notion could call native APIs. Renotion
therefore:

- sets `withGlobalTauri: false`;
- ships **no capability** for remote pages;
- implements every native feature (menus, links, downloads, zoom, window
  state) in Rust;
- injects only passive JavaScript that talks to nothing.

This is both a security decision and a performance one: there is no IPC
traffic, no event bridge and no polling.

## 4. Native behaviour that a browser tab cannot provide

Places where the webview needs help, and how they are handled:

| Problem | Solution |
|---|---|
| Native menu bar / shortcuts | `tauri::menu` with platform accelerators |
| Window size/position lost on restart | `tauri-plugin-window-state` |
| Launching twice creates two apps | `tauri-plugin-single-instance` |
| External links navigate away from Notion | `on_navigation` → system browser |
| `window.open` / target=_blank popups are lost | `on_new_window` → related webview (`NewWindowResponse::Create`) |
| `notion://` links open/fail outside the app | click interception + scheme rewrite |
| Downloads go nowhere | `on_download` → system Downloads folder |
| Window title stuck on "Notion" | `on_document_title_changed` |
| Mouse back/forward buttons are inert | injected `mousedown` handler |
| Drag-and-drop blocks/files swallowed by Tauri | `disable_drag_drop_handler()` |
| Closing the window loses the session | persistent webview data directory (default) |

Notable details:

- **Navigation allowlist.** `notion.so`, `notion.com`, `notion.site` and their
  subdomains stay in-app, plus Google/Apple/Microsoft auth hosts and common
  enterprise identity providers. Everything else is passed to
  `tauri-plugin-opener`, with an env var for additional SSO domains.
- **Popups are created synchronously as related webviews.** `on_new_window`
  returns `NewWindowResponse::Create`, and the builder applies
  `window_features(features)` so WebKit/WKWebView/WebView2 keep the
  `window.opener` relationship. This is required for OAuth: the provider's
  callback posts the result back to the window that opened it. Popups are
  permissive while they are open (they may traverse an arbitrary SSO domain),
  but they stay isolated: there is no Tauri IPC on remote pages. A normal
  `target=_blank` link from a non-login page still goes to the system browser.
- **Downloads** cannot show a synchronous save dialog from `on_download`, so
  the destination is chosen immediately: `download_dir()`, filename recovered
  from `response-content-disposition` / URL path, sanitized and de-duplicated.
  Browser-like, and no lost downloads.
- **Window title** follows the document title, which is one of the small
  signals that makes a wrapper feel native.

## 5. Notion-specific findings

### OAuth sign-in

Notion does not send the login click straight to the provider. It first opens a
popup at `app.notion.com/verifyNoPopupBlockerHtmlAndRedirect`, which forwards to
`app.notion.com/googlepopupredirect` (or `applepopupredirect`,
`microsoftpopupredirect`), then to the provider. After consent the provider
returns a `code` to `app.notion.com/googlepopupcallback` (or the equivalent),
and that callback window hands the session back through `window.opener`.

The original implementation denied every popup and recreated it as an
independent app window. That lost `window.opener`, so the callback had nobody to
notify and the flow looped. Creating the popup as a related webview via
`NewWindowResponse::Create` + `window_features` fixes Google, Apple, Microsoft
and SAML/SSO: the callback can complete the login in the window that started it.

Google still rejects genuinely embedded user agents
(`disallowed_useragent`). WebKitGTK and WKWebView user agents omit the Safari
tokens, so Renotion sends a Safari-shaped UA on WebKit platforms; WebView2
already reports a supported Chromium/Edge UA. If a provider changes its policy,
`RENOTION_USER_AGENT` is the escape hatch.

### Deep links

Notion uses `notion://` links for "open in app". In a wrapper this either does
nothing or launches another client. Rewriting `notion://host/path` to
`https://host/path` inside the page keeps the navigation in the current window.
The Rust navigation policy also translates `notion://` navigations that the
page triggers without a click.

### Desktop-app promos

The sidebar advertises the Notion desktop app. Inside a desktop app that is
noise, but DOM automation is fragile and expensive. A two-rule CSS file scoped
to `[class*="sidebar"]` / `[class*="topbar"]` hides only navigation links whose
`href` contains `/desktop`. No MutationObserver, no polling, no layout reads.
Notion's current CSP includes `style-src 'unsafe-inline'`, so the injected
`<style>` element is honoured. If that changes, the cosmetic rules stop working
but the app keeps functioning.

### Drag and drop

Tauri installs a native drag/drop handler by default. On Windows that replaces
the webview's HTML5 drag/drop path, which breaks Notion's block and file
dragging. `disable_drag_drop_handler()` gives the page the native behaviour.
The app does not need Tauri drag events.

## 6. Build and startup budget

- No frontend assets to embed → smaller binary, faster cold start.
- Release profile: `lto`, `codegen-units = 1`, `opt-level = "s"`,
  `panic = "abort"`, `strip` → compact single binary.
- Window is created immediately with a matching background colour; no splash
  screen or artificial delay.
- Persistent webview data directory means Notion's JS/CSS and service worker
  cache survive restarts, so subsequent launches are as warm as a browser tab.
- Injected script is ~2 KB, runs once per top-level navigation on Notion hosts
  only, and installs one `<style>` element plus two delegated listeners.
- `background_throttling` is left at the platform default so background
  windows do not burn CPU while remaining able to sync.

## 7. Measured on the development machine

Linux x86_64, release profile from this repository:

- stripped single binary: **4.3 MB**;
- second launch hands off to the running instance in **~90 ms** and exits;
- no bundled frontend files, no Node runtime, no `node_modules`;
- startup cost is the webview plus Notion; Renotion itself does not add a
  second browser engine.

Renderer RAM is dominated by Notion's own web app and is not a meaningful
differentiator against Electron on the same page. The wins are disk size,
process count, startup work and native integration.

## 8. Linux graphics caveat

WebKitGTK + some NVIDIA drivers can produce blank or glitchy windows due to the
DMA-BUF renderer. Tauri documents the workaround
([Linux graphics issues](https://v2.tauri.app/develop/debug/linux-graphics)):

```sh
WEBKIT_DISABLE_DMABUF_RENDERER=1 renotion
```

Renotion deliberately does not set this unconditionally: it disables a fast
path for everyone to fix a minority of machines. It is documented as an
opt-in escape hatch.

## 9. What was rejected

- **Electron**: duplicated Chromium, larger binary and memory footprint.
- **Iframe wrapper**: Notion is not frameable; would also break cookies,
  downloads and popups.
- **Browser extension / PWA**: cannot provide a native window, menus, global
  shortcuts or managed session.
- **Custom proxy / HTML rewriting**: adds latency and a maintenance burden for
  no user-visible gain.
- **Remote IPC for injected scripts**: expands the attack surface of remote
  code for features that can be implemented in Rust.
- **DOM automation for theming**: Notion already has a theme setting; fighting
  it would add observers and break on redesigns.

## References

- Tauri configuration (`frontendDist` accepts a remote URL):
  <https://v2.tauri.app/reference/config>
- Tauri webview builder hooks (`on_navigation`, `on_new_window`, `on_download`,
  `initialization_script`, `disable_drag_drop_handler`):
  <https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html>
- Window state plugin:
  <https://docs.rs/tauri-plugin-window-state>
- Google's embedded-webview OAuth policy:
  <https://auth0.com/blog/google-blocks-oauth-requests-from-embedded-browsers>
- Tauri Linux graphics workarounds:
  <https://v2.tauri.app/develop/debug/linux-graphics>
- Notion for web / supported browsers:
  <https://www.notion.com/help/notion-for-web>
