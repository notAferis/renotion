# Contributing to Renotion

Thank you for your interest in contributing to Renotion! Renotion is a lean, native desktop shell around Notion built with Rust and Tauri v2.

---

## Guiding Principles

1. **Deliberately Thin**: Renotion does not re-implement Notion, run an internal Node server, or bundle Chromium. We leverage the operating system's native webview (WebKitGTK on Linux, WKWebView on macOS, WebView2 on Windows).
2. **Minimal Injected Footprint**: Any scripts injected into the Notion runtime must remain lightweight and well-scoped to avoid introducing rendering or latency overhead.
3. **Privacy & Trust**: External links route to the default system browser; only necessary authentication and Notion hosts stay in-app.

---

## Development Setup

### Prerequisites

- [Rust](https://rustup.rs) (stable)
- Tauri system dependencies for your platform:
  - **Arch Linux**: `sudo pacman -S --needed base-devel webkit2gtk-4.1 gtk3 libsoup3`
  - **Debian / Ubuntu**: `sudo apt install build-essential libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev`
  - **Fedora**: `sudo dnf install webkit2gtk4.1-devel gtk3-devel libsoup3-devel`
- Node.js (v18+) for running `@tauri-apps/cli` builds.

### Running Locally

```sh
cd src-tauri
cargo run
```

Or with debug logging:
```sh
RENOTION_DEBUG=1 cargo run
```

### Checking & Building

```sh
# Type check and lint
cargo check --manifest-path src-tauri/Cargo.toml

# Optimized release binary
cargo build --release --manifest-path src-tauri/Cargo.toml
```

---

## Submitting Pull Requests

1. **Fork and branch**: Create a feature branch off `main` (e.g. `feature/my-enhancement` or `fix/my-bug`).
2. **Code style**: Format Rust code with `cargo fmt` and ensure `cargo check` compiles without warnings or errors.
3. **Verify offline resilience**: If modifying navigation or webview lifecycles, verify that launching or navigating offline renders the fallback shell rather than a blank browser error.
4. **Descriptive commits**: Write clear commit messages summarizing what was changed and why.
5. **Open a PR**: Submit your pull request to the `main` branch with a clear description and any screenshots if UI changes were made.
