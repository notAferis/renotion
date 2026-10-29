# Renotion

An unofficial, deliberately thin desktop shell around **Notion** for Linux,
macOS and Windows (desktop only).

Renotion does not reimplement Notion and does not bundle a browser. It opens
the Notion web app (`app.notion.com`) in the operating system's own webview
(WebKitGTK, WKWebView or WebView2) and adds the pieces a plain browser tab
cannot provide: a native window, native menus and shortcuts, window-state
persistence, single-instance behaviour, native link routing and downloads that
land in your Downloads folder.

```
Notion web app  ──►  system webview (WKWebView / WebView2 / WebKitGTK)
                        ▲
                        │  ~1 small Rust process, no bundled Chromium,
                        │  no Node.js, no frontend bundle, no IPC bridge
                     Renotion
```

## Why this is lean

| | Electron wrapper | Renotion |
|---|---|---|
| Browser engine | Bundled Chromium (~150 MB) | OS webview, already installed |
| Runtime | Chromium + Node.js | One small Rust process |
| Frontend build | Bundler + dependencies | None — Notion is loaded directly |
| Disk / RAM | High, duplicated per app | Webview supplied by the OS; no second engine |
| Native menus, links, downloads | Custom Electron code | OS-native APIs |

There is **no `package.json`, no `node_modules` and no compiled frontend**.
`tauri.conf.json` points `frontendDist` at `https://app.notion.com/`, so the
release binary embeds no assets at all. The only injected code is a small
document-start script (about 2 KB) that fixes deep links and mouse navigation.

## Features

- Native window with the page title kept in sync with Notion.
- Native menu bar with platform accelerators:
  - New window (`Ctrl/Cmd+Shift+N`)
  - Reload, zoom in/out/reset, full screen
  - Back / forward, Notion home
  - Open the current page in your real browser (`Ctrl/Cmd+Shift+O`)
  - Sign out / reset the Notion session
  - Developer tools in debug builds
- External links open in the system browser; Notion links stay in the app
  (login providers are kept in-app while signing in).
- `notion://` "open in app" links are rewritten to normal in-app links.
- OAuth and SSO popups (Google, Apple, Microsoft, SAML) are created as related
  webviews, so `window.opener` survives and the provider can hand the session
  back to the Notion window.
- Mouse back/forward buttons work.
- Window position, size, maximized and fullscreen state are restored.
- Launching the app again focuses the existing instance.
- Downloads (page exports, attachments, PDFs) are saved to the system
  Downloads folder, with names recovered from signed URLs' filename hints when
  possible, and de-duplicated (`Export (1).zip`).
- Clipboard, HTML5 drag and drop, and multiple windows are left to the webview,
  so Notion behaves like it does in Safari/Edge.

## Build

Prerequisites: [Rust](https://rustup.rs) and the Tauri system dependencies for
your platform. On Arch Linux:

```sh
sudo pacman -S --needed base-devel webkit2gtk-4.1 gtk3 libsoup3
```

Run a development build:

```sh
cd src-tauri
cargo run
```

Build an optimized binary:

```sh
cd src-tauri
cargo build --release
# binary: target/release/renotion
```

To produce installers (deb/rpm/AppImage, dmg, msi), install the Tauri CLI and
run:

```sh
cargo install tauri-cli --locked
cd src-tauri
cargo tauri build
```

The release profile already uses `lto = true`, `codegen-units = 1`,
`opt-level = "s"`, `panic = "abort"` and `strip = true`.

## Signing in

Notion's login buttons use OAuth popups. Renotion creates those popups as
related webviews, so `window.opener` survives and the provider can hand the
session back to the Notion window. Google, Apple, Microsoft and SAML/SSO flows
stay inside the app, including custom enterprise identity providers reached
during a login flow.

Renotion sends a Safari-shaped user agent on WebKit platforms so providers do
not mistake the system webview for an unsupported embedded browser. If a
provider still rejects it, override the user agent at launch:

```sh
RENOTION_USER_AGENT="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36" renotion
```

An identity provider that is normally opened outside a recognised login flow can
be kept in-app explicitly with a comma-separated list of domains:

```sh
RENOTION_ALLOWED_HOSTS="sso.example.com,login.example.net" renotion
```

Anything outside a login flow and not on the allowlist opens in the system
browser by design.

## Configuration

| Variable | Purpose |
|---|---|
| `RENOTION_USER_AGENT` | Override the webview user agent. |
| `RENOTION_ALLOWED_HOSTS` | Comma-separated extra hostnames (and subdomains) allowed to navigate inside the app. |
| `RENOTION_START_URL` | Override the initial URL (useful for testing login or public pages). |
| `RENOTION_DEBUG` | Print navigation, popup and page-load diagnostics to stderr. |

## What is intentionally *not* here

- **No Tauri IPC on notion.so.** `withGlobalTauri` is off and no capability
  grants remote pages access to the Rust core, so a compromised web page cannot
  reach the file system through Renotion. All native features are driven from
  Rust.
- **No DOM polling.** Banner hiding is pure CSS, so it costs nothing at
  runtime. There is no MutationObserver scanning Notion's tree.
- **No custom proxy, no request rewriting, no analytics.** Notion's own
  service worker and HTTP cache are reused across launches.
- **No offline mirror.** If Notion cannot be reached, the webview shows its
  normal network error; data remains on Notion's servers.

## Known limitations

- Unofficial project; not affiliated with Notion Labs, Inc. Use at your own
  risk and respect Notion's terms.
- Notion controls its own DOM. The injected CSS hides desktop-app promos in
  navigation chrome, but a future redesign can make those selectors stale.
- Native notifications and dock/taskbar badges are not bridged. Notion's
  in-app reminders still work.
- On some NVIDIA + WebKitGTK setups, WebKit's DMA-BUF renderer can cause blank
  windows. Work around it with
  `WEBKIT_DISABLE_DMABUF_RENDERER=1 renotion` at the cost of some GPU
  acceleration.
- No auto-updater.

## Project layout

```
src-tauri/
  src/lib.rs          window lifecycle, menus, policy, downloads, zoom
  src/main.rs         thin entry point
  inject/enhance.css  CSS-only chrome fixes
  inject/enhance.js   deep links, mouse back/forward
  tauri.conf.json     remote frontend, minimal security surface
```

The research notes and the reasoning behind these choices are in
[docs/native-shell-research.md](docs/native-shell-research.md).
