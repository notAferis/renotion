//! Renotion is a thin, native shell around the Notion web app.
//!
//! There is deliberately no bundled frontend, JavaScript framework or
//! Chromium. The operating system webview renders notion.so directly and this
//! file supplies only the things a browser cannot: native menus, window state,
//! sane link/popup/download routing and deep-link handling.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use percent_encoding::percent_decode_str;
use tauri::{
    AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry,
    menu::{
        AboutMetadata, Menu, MenuBuilder, MenuItem, MenuItemBuilder, PredefinedMenuItem,
        SubmenuBuilder,
    },
    webview::{Color, DownloadEvent, NewWindowFeatures, NewWindowResponse},
};

const MAIN_WINDOW: &str = "main";
const HOME_URL: &str = "https://app.notion.com/";
const HELP_URL: &str = "https://www.notion.com/help";
const SHORTCUTS_URL: &str = "https://www.notion.com/help/keyboard-shortcuts";

const MENU_NEW_WINDOW: &str = "new-window";
const MENU_OPEN_BROWSER: &str = "open-in-browser";
const MENU_SIGN_OUT: &str = "sign-out";
const MENU_CLOSE: &str = "close-window";
const MENU_QUIT: &str = "quit";
const MENU_RELOAD: &str = "reload";
const MENU_ZOOM_IN: &str = "zoom-in";
const MENU_ZOOM_OUT: &str = "zoom-out";
const MENU_ZOOM_RESET: &str = "zoom-reset";
const MENU_FULLSCREEN: &str = "fullscreen";
#[cfg(debug_assertions)]
const MENU_DEVTOOLS: &str = "devtools";
const MENU_BACK: &str = "back";
const MENU_FORWARD: &str = "forward";
const MENU_HOME: &str = "home";
const MENU_MINIMIZE: &str = "minimize";
const MENU_HELP: &str = "help";
const MENU_SHORTCUTS: &str = "shortcuts";

/// Only these hosts are kept inside the app. Everything else is handed to the
/// system browser, which is what makes links in a Notion page feel trustworthy.
const AUTH_HOSTS: &[&str] = &[
    // Google
    "accounts.google.com",
    "accounts.youtube.com",
    "myaccount.google.com",
    "gds.google.com",
    "apis.google.com",
    // Apple
    "appleid.apple.com",
    "idmsa.apple.com",
    // Microsoft
    "microsoftonline.com",
    "microsoftonline.us",
    "login.live.com",
    "login.microsoft.com",
    "account.microsoft.com",
    "account.live.com",
    // Common enterprise identity providers. Custom domains still work because
    // a login flow is allowed to leave Notion for any host.
    "okta.com",
    "oktapreview.com",
    "auth0.com",
    "onelogin.com",
    "duosecurity.com",
    "pingidentity.com",
    "pingone.com",
    "jumpcloud.com",
];

static NEXT_WINDOW_ID: AtomicUsize = AtomicUsize::new(1);

/// Per-window zoom, because WebKit does not expose a zoom getter.
#[derive(Default)]
struct ZoomLevels(Mutex<HashMap<String, f64>>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(
            |app: &AppHandle<Wry>, _args, _cwd| focus_or_create_main(app),
        ));
    }

    builder
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(ZoomLevels::default())
        .setup(|app| {
            let menu = build_menu(app.handle())?;
            let _ = app.handle().set_menu(menu)?;

            build_notion_window(app.handle(), MAIN_WINDOW, home_url(), None)?;
            Ok(())
        })
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .run(tauri::generate_context!())
        .expect("error while running Renotion");
}

// ---------------------------------------------------------------------------
// Window lifecycle
// ---------------------------------------------------------------------------

fn home_url() -> Url {
    static URL: OnceLock<Url> = OnceLock::new();
    URL.get_or_init(|| {
        std::env::var("RENOTION_START_URL")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| HOME_URL.parse().expect("valid Notion URL"))
    })
    .clone()
}

fn debug_log(message: std::fmt::Arguments<'_>) {
    if std::env::var_os("RENOTION_DEBUG").is_some() {
        eprintln!("renotion: {message}");
    }
}

/// The one window every launch starts with. Extra windows (⌘-click in Notion,
/// File → New Window) use the same builder through `spawn_window`.
fn build_notion_window(
    app: &AppHandle<Wry>,
    label: &str,
    url: Url,
    features: Option<NewWindowFeatures>,
) -> tauri::Result<WebviewWindow<Wry>> {
    let app_new_window = app.clone();
    let app_download = app.clone();
    let is_popup = features.is_some();
    let auth_flow = Arc::new(AtomicBool::new(false));
    let auth_flow_navigation = Arc::clone(&auth_flow);
    let opener_label = label.to_owned();

    // A script-opened popup (OAuth, preview windows) must be created synchronously
    // and returned via `NewWindowResponse::Create` so the webview keeps its
    // `window.opener` relationship. WebKit loads the requested URL into it after
    // the create handler returns, so start from about:blank like Tauri's example.
    let initial_url = if is_popup {
        WebviewUrl::External("about:blank".parse().expect("valid about:blank URL"))
    } else {
        WebviewUrl::External(url)
    };

    let mut builder = WebviewWindowBuilder::new(app, label, initial_url)
        .title("Notion")
        .inner_size(1280.0, 820.0)
        .min_inner_size(440.0, 320.0)
        .center()
        .background_color(Color(255, 255, 255, 255))
        // Let the webview own HTML5 drag and drop. Tauri's default handler
        // swallows it on Windows, which breaks dragging blocks and files.
        .disable_drag_drop_handler()
        .enable_clipboard_access()
        .devtools(cfg!(debug_assertions))
        .initialization_script(injected_script())
        .on_page_load(|_window, payload| {
            debug_log(format_args!("page {:?} {}", payload.event(), payload.url()));
        })
        .on_navigation(move |url| {
            debug_log(format_args!("navigation {url}"));
            update_auth_flow(&auth_flow_navigation, url);

            // A popup is a short-lived, isolated context: OAuth providers may
            // bounce through any host (Okta, ADFS, a custom SSO domain). The
            // main window stays strict, except while a login flow is active.
            let permissive = is_popup || auth_flow_navigation.load(Ordering::Relaxed);
            handle_navigation(url, permissive)
        })
        .on_new_window(move |url, features| {
            debug_log(format_args!("new-window {url}"));

            // Ask the window that opened this popup where it currently is. A
            // login page may open an arbitrary SSO provider, so keep that popup
            // in-app; once the user is back in a workspace, normal target=_blank
            // links go to the real browser again. Looking the window up by label
            // avoids storing a handle inside its own closure.
            let opener_is_auth = app_new_window
                .get_webview_window(&opener_label)
                .and_then(|window| window.url().ok())
                .map(|url| is_notion_auth_url(&url))
                .unwrap_or(false);
            let keep_in_app = is_popup || opener_is_auth || is_allowed_url(&url);

            if is_http(&url) && !keep_in_app {
                open_external(&url);
                return NewWindowResponse::Deny;
            }

            if !(is_http(&url) || matches!(url.scheme(), "about" | "blob" | "data")) {
                if url.scheme() == "notion" {
                    if let Some(target) = notion_to_https(&url) {
                        open_external(&target);
                    }
                } else {
                    open_external(&url);
                }
                return NewWindowResponse::Deny;
            }

            let label = format!("notion-{}", NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed));

            match build_notion_window(&app_new_window, &label, url, Some(features)) {
                Ok(window) => NewWindowResponse::Create { window },
                Err(error) => {
                    eprintln!("renotion: could not open popup window: {error}");
                    NewWindowResponse::Deny
                }
            }
        })
        .on_download(move |_webview, event| handle_download(&app_download, event))
        .on_document_title_changed(|window, title| {
            let title = title.trim();
            if !title.is_empty() {
                let _ = window.set_title(title);
            }
        });

    if let Some(features) = features {
        builder = builder.window_features(features);
    }

    // A Safari-shaped user agent keeps Notion (and Google's OAuth screen) from
    // treating the system webview as an unsupported embedded browser.
    if let Some(user_agent) = user_agent() {
        builder = builder.user_agent(&user_agent);
    }

    let window = builder.build()?;

    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::WebViewExt;
        let _ = window.with_webview(|webview| {
            let gtk_webview = webview.inner();
            gtk_webview.connect_load_failed(|webview, _event, failing_uri, error| {
                debug_log(format_args!("load-failed: {failing_uri} ({error})"));
                // Only intercept top-level page loads for Notion pages, letting subresources fail naturally
                if failing_uri.starts_with("https://app.notion.com") || failing_uri.starts_with("https://www.notion.so") {
                    let html = include_str!("../resources/offline.html");
                    webview.load_alternate_html(html, failing_uri, None);
                    return true;
                }
                false
            });
        });
    }

    Ok(window)
}

fn spawn_window(app: &AppHandle<Wry>, url: Url) {
    let app = app.clone();
    let label = format!("notion-{}", NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed));

    // Building a webview inside a menu/event handler can deadlock on Windows.
    std::thread::spawn(move || {
        if let Err(error) = build_notion_window(&app, &label, url, None) {
            eprintln!("renotion: could not open window: {error}");
        }
    });
}

fn focus_or_create_main(app: &AppHandle<Wry>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        spawn_window(app, home_url());
    }
}

fn focused_window(app: &AppHandle<Wry>) -> Option<WebviewWindow<Wry>> {
    let windows = app.webview_windows();
    windows
        .values()
        .find(|window| window.is_focused().unwrap_or(false))
        .cloned()
        .or_else(|| app.get_webview_window(MAIN_WINDOW))
        .or_else(|| windows.values().next().cloned())
}

// ---------------------------------------------------------------------------
// Navigation policy
// ---------------------------------------------------------------------------

fn is_http(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

fn is_notion_host(host: &str) -> bool {
    let host = host.strip_prefix("www.").unwrap_or(host);
    host == "notion.so"
        || host == "notion.com"
        || host == "notion.site"
        || host.ends_with(".notion.so")
        || host.ends_with(".notion.com")
        || host.ends_with(".notion.site")
}

fn host_matches(host: &str, allowed: &str) -> bool {
    host == allowed || host.ends_with(&format!(".{allowed}"))
}

fn extra_allowed_hosts() -> &'static [String] {
    static HOSTS: OnceLock<Vec<String>> = OnceLock::new();
    HOSTS.get_or_init(|| {
        std::env::var("RENOTION_ALLOWED_HOSTS")
            .map(|value| {
                value
                    .split(',')
                    .map(|host| host.trim().trim_start_matches('.').to_ascii_lowercase())
                    .filter(|host| !host.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn is_allowed_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    is_notion_host(host)
        || AUTH_HOSTS.iter().any(|allowed| host_matches(host, allowed))
        || extra_allowed_hosts()
            .iter()
            .any(|allowed| host_matches(host, allowed))
}

fn is_notion_auth_url(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    // Public notion.site pages can have arbitrary slugs; never treat those as
    // part of the login flow.
    if host == "notion.site" || host.ends_with(".notion.site") || !is_notion_host(host) {
        return false;
    }

    let path = url.path().to_ascii_lowercase();
    [
        "login", "signup", "sign-up", "auth", "oauth", "saml", "sso", "popup", "callback",
        "passkey", "handoff",
    ]
    .iter()
    .any(|marker| path.contains(marker))
}

/// Remember whether this window is in the middle of a login flow. That lets a
/// top-level SAML redirect leave Notion for an arbitrary identity provider
/// without permanently turning the main window into a general-purpose browser.
fn update_auth_flow(auth_flow: &AtomicBool, url: &Url) {
    let Some(host) = url.host_str() else {
        return;
    };

    if is_notion_host(host) {
        auth_flow.store(is_notion_auth_url(url), Ordering::Relaxed);
    }
}

/// Returning `false` cancels the navigation in the webview; for external links
/// we open them in the user's browser instead.
fn handle_navigation(url: &Url, permissive: bool) -> bool {
    match url.scheme() {
        "http" | "https" => {
            if permissive || is_allowed_url(url) {
                true
            } else {
                open_external(url);
                false
            }
        }
        "mailto" | "tel" => {
            open_external(url);
            false
        }
        "notion" => {
            if let Some(target) = notion_to_https(url) {
                open_external(&target);
            }
            false
        }
        "about" | "blob" | "data" => true,
        // Never let a page navigate the native window to a local or unknown
        // scheme. Downloads still work because they are not navigations.
        _ => false,
    }
}

fn notion_to_https(url: &Url) -> Option<Url> {
    let rest = url.as_str().strip_prefix("notion://")?;
    if rest.is_empty() {
        return None;
    }

    let target = if rest.starts_with("http://") || rest.starts_with("https://") {
        rest.to_owned()
    } else {
        format!("https://{rest}")
    };

    Url::parse(&target).ok()
}

fn open_external(url: &Url) {
    if !matches!(url.scheme(), "http" | "https" | "mailto" | "tel") {
        return;
    }

    if let Err(error) = tauri_plugin_opener::open_url(url.as_str(), None::<&str>) {
        eprintln!("renotion: could not open {url}: {error}");
    }
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

/// Downloads (page exports, file attachments, PDF prints) go to the system
/// Downloads folder, like a browser. A save dialog cannot be used here because
/// `on_download` requires the destination synchronously.
fn handle_download(app: &AppHandle<Wry>, event: DownloadEvent<'_>) -> bool {
    match event {
        DownloadEvent::Requested { url, destination } => {
            if is_http(&url)
                && let Ok(directory) = app.path().download_dir()
            {
                let _ = std::fs::create_dir_all(&directory);
                let name = download_file_name(&url);
                *destination = unique_download_path(&directory, &name);
            }
            true
        }
        DownloadEvent::Finished { url, path, success } => {
            if success {
                eprintln!("renotion: downloaded {url} to {path:?}");
            } else {
                eprintln!("renotion: download failed: {url}");
            }
            true
        }
        _ => true,
    }
}

fn download_file_name(url: &Url) -> String {
    if let Some(name) = content_disposition_name(url) {
        return sanitize_filename(&name);
    }

    let name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or_default();

    if name.is_empty() {
        "notion-download".to_owned()
    } else {
        let decoded = percent_decode_str(name).decode_utf8_lossy();
        sanitize_filename(&decoded)
    }
}

fn content_disposition_name(url: &Url) -> Option<String> {
    for (key, value) in url.query_pairs() {
        let is_disposition = key.eq_ignore_ascii_case("response-content-disposition")
            || key.eq_ignore_ascii_case("content-disposition");
        if !is_disposition {
            continue;
        }
        if let Some(name) = parse_content_disposition(&value) {
            return Some(name);
        }
    }
    None
}

fn parse_content_disposition(value: &str) -> Option<String> {
    for part in value.split(';') {
        let part = part.trim();

        if let Some(rest) = part.strip_prefix("filename*=") {
            let rest = rest.trim_matches('"');
            // RFC 5987: UTF-8''export.zip
            let name = rest.rsplit("''").next().unwrap_or(rest);
            return Some(name.to_owned());
        }

        if let Some(rest) = part.strip_prefix("filename=") {
            return Some(rest.trim_matches('"').to_owned());
        }
    }
    None
}

fn sanitize_filename(name: &str) -> String {
    let name = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    let cleaned: String = name
        .chars()
        .map(|character| {
            if matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            ) || character.is_control()
            {
                '_'
            } else {
                character
            }
        })
        .collect();

    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "notion-download".to_owned()
    } else {
        cleaned.chars().take(180).collect()
    }
}

fn unique_download_path(directory: &Path, file_name: &str) -> PathBuf {
    let candidate = directory.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("notion-download");
    let extension = path.extension().and_then(|extension| extension.to_str());

    for index in 1..1000 {
        let name = match extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = directory.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    directory.join(format!("{stem}-{}", std::process::id()))
}

// ---------------------------------------------------------------------------
// User agent
// ---------------------------------------------------------------------------

fn user_agent() -> Option<String> {
    static AGENT: OnceLock<Option<String>> = OnceLock::new();

    AGENT
        .get_or_init(|| {
            if let Ok(value) = std::env::var("RENOTION_USER_AGENT")
                && !value.trim().is_empty()
            {
                return Some(value);
            }

            // WebView2 already reports a current Chromium/Edge agent, which
            // Notion supports. WebKit engines omit the Safari token, which
            // some sites (notably Google sign-in) interpret as an embedded
            // browser and reject.
            if cfg!(target_os = "windows") {
                return None;
            }

            #[cfg(target_os = "macos")]
            {
                Some("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15".to_owned())
            }

            #[cfg(target_os = "linux")]
            {
                Some("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15".to_owned())
            }

            #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
            {
                None
            }
        })
        .clone()
}

// ---------------------------------------------------------------------------
// Injected enhancements
// ---------------------------------------------------------------------------

fn injected_script() -> String {
    static SCRIPT: OnceLock<String> = OnceLock::new();

    SCRIPT
        .get_or_init(|| {
            let css = serde_json::to_string(include_str!("../inject/enhance.css"))
                .unwrap_or_else(|_| "\"\"".to_owned());
            include_str!("../inject/enhance.js").replace("__RENOTION_CSS__", &css)
        })
        .clone()
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

fn menu_item(
    app: &AppHandle<Wry>,
    id: &str,
    text: &str,
    accelerator: Option<&str>,
) -> tauri::Result<MenuItem<Wry>> {
    let mut builder = MenuItemBuilder::with_id(id, text);
    if let Some(accelerator) = accelerator {
        builder = builder.accelerator(accelerator);
    }
    builder.build(app)
}

fn about_metadata() -> AboutMetadata<'static> {
    AboutMetadata {
        name: Some("Renotion".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some("A lean, native shell around Notion.".into()),
        website: Some("https://www.notion.so".into()),
        copyright: Some("Unofficial client. Not affiliated with Notion Labs, Inc.".into()),
        ..Default::default()
    }
}

#[allow(unused_mut)]
fn build_menu(app: &AppHandle<Wry>) -> tauri::Result<Menu<Wry>> {
    let mut menu = MenuBuilder::new(app);

    #[cfg(target_os = "macos")]
    {
        let app_menu = SubmenuBuilder::new(app, "Renotion")
            .about(Some(about_metadata()))
            .separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?;
        menu = menu.item(&app_menu);
    }

    let new_window = menu_item(
        app,
        MENU_NEW_WINDOW,
        "New Window",
        Some("CmdOrCtrl+Shift+N"),
    )?;
    let open_browser = menu_item(
        app,
        MENU_OPEN_BROWSER,
        "Open Current Page in Browser",
        Some("CmdOrCtrl+Shift+O"),
    )?;
    let sign_out = menu_item(app, MENU_SIGN_OUT, "Sign Out of Notion…", None)?;

    let mut file = SubmenuBuilder::new(app, "File")
        .item(&new_window)
        .item(&open_browser)
        .separator()
        .item(&sign_out)
        .separator();

    #[cfg(target_os = "macos")]
    {
        let close = PredefinedMenuItem::close_window(app, None)?;
        file = file.item(&close);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let close = menu_item(app, MENU_CLOSE, "Close Window", Some("CmdOrCtrl+W"))?;
        let quit = menu_item(app, MENU_QUIT, "Quit Renotion", Some("CmdOrCtrl+Q"))?;
        file = file.item(&close).separator().item(&quit);
    }

    let file_menu = file.build()?;

    let mut edit = SubmenuBuilder::new(app, "Edit");

    #[cfg(target_os = "macos")]
    {
        edit = edit.undo().redo().separator();
    }

    let edit_menu = edit.cut().copy().paste().select_all().build()?;

    let reload = menu_item(app, MENU_RELOAD, "Reload", Some("CmdOrCtrl+R"))?;
    let zoom_in = menu_item(app, MENU_ZOOM_IN, "Zoom In", Some("CmdOrCtrl+="))?;
    let zoom_out = menu_item(app, MENU_ZOOM_OUT, "Zoom Out", Some("CmdOrCtrl+-"))?;
    let zoom_reset = menu_item(app, MENU_ZOOM_RESET, "Actual Size", Some("CmdOrCtrl+0"))?;

    #[cfg(target_os = "macos")]
    let fullscreen_accelerator = None;
    #[cfg(not(target_os = "macos"))]
    let fullscreen_accelerator = Some("F11");

    let fullscreen = menu_item(
        app,
        MENU_FULLSCREEN,
        "Toggle Full Screen",
        fullscreen_accelerator,
    )?;

    let mut view = SubmenuBuilder::new(app, "View")
        .item(&reload)
        .separator()
        .item(&zoom_in)
        .item(&zoom_out)
        .item(&zoom_reset)
        .separator()
        .item(&fullscreen);

    #[cfg(debug_assertions)]
    {
        let devtools = menu_item(app, MENU_DEVTOOLS, "Toggle Developer Tools", Some("F12"))?;
        view = view.separator().item(&devtools);
    }

    let view_menu = view.build()?;

    let back = menu_item(
        app,
        MENU_BACK,
        "Back",
        if cfg!(target_os = "macos") {
            None
        } else {
            Some("Alt+Left")
        },
    )?;
    let forward = menu_item(
        app,
        MENU_FORWARD,
        "Forward",
        if cfg!(target_os = "macos") {
            None
        } else {
            Some("Alt+Right")
        },
    )?;
    let home = menu_item(app, MENU_HOME, "Notion Home", None)?;

    let navigate_menu = SubmenuBuilder::new(app, "Navigate")
        .item(&back)
        .item(&forward)
        .separator()
        .item(&home)
        .build()?;

    let minimize = menu_item(app, MENU_MINIMIZE, "Minimize", Some("CmdOrCtrl+M"))?;

    #[allow(unused_mut)]
    let mut window = SubmenuBuilder::new(app, "Window").item(&minimize);

    #[cfg(target_os = "macos")]
    {
        let bring_all_to_front = PredefinedMenuItem::bring_all_to_front(app, None)?;
        window = window.separator().item(&bring_all_to_front);
    }

    let window_menu = window.build()?;

    let help_center = menu_item(app, MENU_HELP, "Notion Help Center", None)?;
    let shortcuts = menu_item(app, MENU_SHORTCUTS, "Keyboard Shortcuts", None)?;

    let mut help = SubmenuBuilder::new(app, "Help")
        .item(&help_center)
        .item(&shortcuts);

    #[cfg(not(target_os = "macos"))]
    {
        let about = PredefinedMenuItem::about(app, Some("About Renotion"), Some(about_metadata()))?;
        help = help.separator().item(&about);
    }

    let help_menu = help.build()?;

    menu.items(&[
        &file_menu,
        &edit_menu,
        &view_menu,
        &navigate_menu,
        &window_menu,
        &help_menu,
    ])
    .build()
}

fn handle_menu_event(app: &AppHandle<Wry>, id: &str) {
    match id {
        MENU_NEW_WINDOW => spawn_window(app, home_url()),
        MENU_OPEN_BROWSER => {
            if let Some(window) = focused_window(app)
                && let Ok(url) = window.url()
            {
                open_external(&url);
            }
        }
        MENU_SIGN_OUT => {
            if let Some(window) = focused_window(app) {
                let _ = window.clear_all_browsing_data();
                // Give WebKit a moment to drop the cookie jar before loading
                // the signed-out page.
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(300));
                    let _ = window.navigate(home_url());
                });
            }
        }
        MENU_CLOSE => {
            if let Some(window) = focused_window(app) {
                let _ = window.close();
            }
        }
        MENU_QUIT => app.exit(0),
        MENU_RELOAD => {
            if let Some(window) = focused_window(app) {
                let _ = window.reload();
            }
        }
        MENU_ZOOM_IN => adjust_zoom(app, 0.1),
        MENU_ZOOM_OUT => adjust_zoom(app, -0.1),
        MENU_ZOOM_RESET => set_zoom(app, 1.0),
        MENU_FULLSCREEN => {
            if let Some(window) = focused_window(app) {
                let fullscreen = window.is_fullscreen().unwrap_or(false);
                let _ = window.set_fullscreen(!fullscreen);
            }
        }
        #[cfg(debug_assertions)]
        MENU_DEVTOOLS => {
            if let Some(window) = focused_window(app) {
                window.open_devtools();
            }
        }
        MENU_BACK => {
            if let Some(window) = focused_window(app) {
                let _ = window.eval("history.back()");
            }
        }
        MENU_FORWARD => {
            if let Some(window) = focused_window(app) {
                let _ = window.eval("history.forward()");
            }
        }
        MENU_HOME => {
            if let Some(window) = focused_window(app) {
                let _ = window.navigate(home_url());
            }
        }
        MENU_MINIMIZE => {
            if let Some(window) = focused_window(app) {
                let _ = window.minimize();
            }
        }
        MENU_HELP => open_url_text(HELP_URL),
        MENU_SHORTCUTS => open_url_text(SHORTCUTS_URL),
        _ => {}
    }
}

fn open_url_text(url: &str) {
    if let Ok(url) = Url::parse(url) {
        open_external(&url);
    }
}

fn adjust_zoom(app: &AppHandle<Wry>, delta: f64) {
    let Some(window) = focused_window(app) else {
        return;
    };

    let state = app.state::<ZoomLevels>();
    let Ok(mut levels) = state.0.lock() else {
        return;
    };

    let level = levels.entry(window.label().to_owned()).or_insert(1.0);
    *level = (*level + delta).clamp(0.5, 2.0);
    let zoom = *level;

    drop(levels);
    let _ = window.set_zoom(zoom);
}

fn set_zoom(app: &AppHandle<Wry>, zoom: f64) {
    let Some(window) = focused_window(app) else {
        return;
    };

    let state = app.state::<ZoomLevels>();
    if let Ok(mut levels) = state.0.lock() {
        levels.insert(window.label().to_owned(), zoom);
    }
    let _ = window.set_zoom(zoom);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        value.parse().expect("test URL")
    }

    #[test]
    fn notion_hosts_are_recognised() {
        assert!(is_notion_host("notion.so"));
        assert!(is_notion_host("www.notion.so"));
        assert!(is_notion_host("team.notion.site"));
        assert!(is_notion_host("www.notion.com"));
        assert!(!is_notion_host("notion.so.example.com"));
        assert!(!is_notion_host("evilnotion.com"));
    }

    #[test]
    fn auth_host_matching_requires_a_label_boundary() {
        assert!(host_matches("accounts.google.com", "accounts.google.com"));
        assert!(host_matches(
            "sub.accounts.google.com",
            "accounts.google.com"
        ));
        assert!(!host_matches(
            "evilaccounts.google.com",
            "accounts.google.com"
        ));
    }

    #[test]
    fn oauth_flow_urls_are_recognised() {
        for value in [
            "https://www.notion.com/login",
            "https://app.notion.com/login",
            "https://app.notion.com/googlepopupredirect?callbackType=popup",
            "https://app.notion.com/verifyNoPopupBlockerHtmlAndRedirect?redirectUri=abc",
            "https://app.notion.com/applepopupredirect?callbackType=popup",
            "https://app.notion.com/microsoftpopupredirect?callbackType=popup",
            "https://app.notion.com/samlAuthCallback?code=abc",
            "https://identity.notion.com/authSync",
            "https://www.notion.com/browserSessionHandoffToDesktop?passkeyState=abc",
        ] {
            assert!(is_notion_auth_url(&url(value)), "{value}");
        }

        for value in [
            "https://www.notion.so/",
            "https://app.notion.com/workspace/123",
            "https://team.notion.site/login",
            "https://example.com/login",
        ] {
            assert!(!is_notion_auth_url(&url(value)), "{value}");
        }
    }

    #[test]
    fn oauth_popup_hosts_are_allowed() {
        assert!(is_allowed_url(&url(
            "https://accounts.youtube.com/accounts/CheckConnection"
        )));
        assert!(is_allowed_url(&url(
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
        )));
        assert!(is_allowed_url(&url(
            "https://example.okta.com/oauth2/v1/authorize"
        )));
        assert!(!is_allowed_url(&url("https://example.com/")));
    }

    #[test]
    fn deep_links_become_https() {
        assert_eq!(
            notion_to_https(&url("notion://www.notion.so/Page-abc"))
                .expect("translated")
                .as_str(),
            "https://www.notion.so/Page-abc"
        );
        assert_eq!(
            notion_to_https(&url("notion://notion.so/Page-abc"))
                .expect("translated")
                .as_str(),
            "https://notion.so/Page-abc"
        );
        assert!(notion_to_https(&url("notion://")).is_none());
    }

    #[test]
    fn filenames_are_taken_from_the_url_path() {
        assert_eq!(
            download_file_name(&url("https://example.com/files/My%20Export.zip")),
            "My Export.zip"
        );
        assert_eq!(
            download_file_name(&url("https://example.com/")),
            "notion-download"
        );
    }

    #[test]
    fn filenames_are_taken_from_content_disposition() {
        let value = url(
            "https://example.com/download?response-content-disposition=attachment%3B%20filename%3D%22Q3%20Report.pdf%22",
        );
        assert_eq!(download_file_name(&value), "Q3 Report.pdf");

        let encoded = url(
            "https://example.com/download?response-content-disposition=attachment%3B%20filename%2A%3DUTF-8%27%27Caf%C3%A9.zip",
        );
        assert_eq!(download_file_name(&encoded), "Café.zip");
    }

    #[test]
    fn filenames_cannot_escape_the_download_directory() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename(".."), "notion-download");
        assert_eq!(sanitize_filename("a:b*c?.txt"), "a_b_c_.txt");
    }

    #[test]
    fn injected_css_is_a_valid_js_string() {
        let script = injected_script();
        assert!(script.contains("data-renotion"));
        assert!(!script.contains("__RENOTION_CSS__"));
    }
}
