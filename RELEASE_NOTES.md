# Release Notes - Renotion v0.1.0

Renotion is an unofficial, deliberately thin desktop shell around **Notion** for Linux, macOS, and Windows.

---

## What's New in v0.1.0

### 🚀 Lean Desktop Experience
- Opens Notion (`app.notion.com`) in the operating system's native webview (WebKitGTK on Linux, WKWebView on macOS, WebView2 on Windows) without bundled Chromium or Node.js.
- Extremely low memory footprint compared to traditional Electron wrappers.
- Native menus, window-state preservation, single-instance routing, and downloads routed to your native Downloads directory.

### 📶 Offline Resilience & Fallback Shell
- **Graceful Offline Screen**: When launched without an internet connection, Renotion presents a clean, dark/light theme aware fallback shell instead of a raw browser error.
- **Auto-Reconnection**: Automatically probes connectivity and seamlessly reloads the workspace as soon as connection is restored.
- **In-App Offline Indicator**: Displays an unobtrusive floating notification pill when connectivity drops while using the app, indicating cached content status.
- **Native WebKit Error Interception**: Integrates directly with WebKitGTK's `connect_load_failed` signal on Linux to intercept network errors and render the fallback shell.

---

## Downloads & Installation

### Linux (Debian / Ubuntu / Pop!_OS)
Download the `.deb` package from the `release/` directory or releases tab and install via `dpkg` or `apt`:

```sh
sudo dpkg -i Renotion_0.1.0_amd64.deb
# Or:
sudo apt install ./Renotion_0.1.0_amd64.deb
```
