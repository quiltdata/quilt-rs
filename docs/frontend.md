# QuiltSync Frontend Architecture

> **Audience**: Contributors who need to understand how the desktop UI
> works end-to-end, from the Tauri webview to the Leptos components.

## Overview

QuiltSync is a Tauri v2 desktop application. The UI is a
**client-side rendered Leptos WASM app** running inside the webview.
Rust compiles to WebAssembly via Trunk, and Leptos handles routing,
reactivity, and DOM rendering entirely in the browser. The Tauri
backend exposes data through `#[tauri::command]` handlers that return
serializable structs -- the frontend owns all rendering.

## Stack

| Layer | Technology | Location |
|---|---|---|
| Framework | Leptos 0.8 (CSR mode) | `quilt-sync/ui/src/` |
| Routing | leptos\_router 0.8 | `ui/src/main.rs` |
| WASM bridge | wasm-bindgen + serde-wasm-bindgen | `ui/src/tauri.rs` |
| Styles | Plain CSS (global, no scoping) | `ui/assets/css/` |
| Build tool | Trunk | `ui/Trunk.toml` |
| IPC | Tauri commands (`#[tauri::command]`) | `src-tauri/src/commands.rs` |

## Directory Structure

```text
quilt-sync/ui/
├── Cargo.toml
├── Trunk.toml
├── index.html                  # Entry point (CSS links, Trunk directives)
├── assets/
│   ├── css/
│   │   ├── styles.css          # Global resets
│   │   ├── theme.css           # CSS custom properties
│   │   ├── layout.css          # Layout helpers
│   │   ├── spinner.css         # Loading indicator
│   │   ├── components/         # Reusable component styles
│   │   ├── pages/              # Per-page styles
│   │   ├── views/              # Layout section styles (appbar, toolbar, ...)
│   │   └── external/           # Vendored webfont files
│   ├── img/
│   │   └── icons/              # SVG icons
│   └── js/
│       └── json-editor.js      # Third-party JSON editor (commit metadata)
└── src/
    ├── main.rs                 # App root, router, legacy URL redirect
    ├── components.rs           # Module re-exports
    ├── components/
    │   ├── buttons.rs          # Button module: ButtonKind enum, re-exports
    │   ├── buttons/            # IconButton/ButtonCta bases + specific buttons
    │   ├── layout.rs           # Layout, Notification, BreadcrumbItem
    │   ├── spinner.rs          # Loading spinner
    │   └── update_checker.rs   # Auto-update polling
    ├── pages.rs                # Module re-exports
    ├── pages/                  # One file per page (9 pages)
    │   ├── installed_packages_list.rs
    │   ├── installed_package.rs
    │   ├── commit.rs
    │   ├── settings.rs
    │   ├── merge.rs
    │   ├── login.rs
    │   ├── setup.rs
    │   ├── error.rs
    │   └── remote_package.rs
    ├── commands.rs             # Typed Tauri command wrappers + response DTOs
    ├── tauri.rs                # Low-level WASM-to-JS invoke bridge
    └── error_handler.rs        # Error parsing and redirect logic
```

## Data Flow

### Page load cycle

Every page follows the same reactive pattern:

```text
Browser navigates to e.g. /installed-package?namespace=foo/bar
    |
    v
leptos_router matches route, mounts page component
    |
    v
Component creates LocalResource (async data fetch)
    |
    v
Suspense renders <Layout> + <Spinner /> while loading
    |
    v
LocalResource calls commands::get_page_data() [typed wrapper]
    |                                              (Tauri IPC)
    v
tauri::invoke() → wasm-bindgen → window.__TAURI__.core.invoke()
    |
    v
Rust #[tauri::command] handler returns serializable struct
    |
    v
serde-wasm-bindgen deserializes response into Rust DTO
    |
    v
Suspend::new resolves, Leptos renders the page reactively
```

### Two-phase loading (Installed Packages List)

The packages list page uses a two-phase approach so the list renders
instantly instead of blocking behind network calls and file hashing:

```text
Phase 1 — Light (cached lineage)
    get_installed_packages_list_data()
        |
        v
    For each package, read lineage.json from disk
        |
        v
    Derive upstream status from cached hashes
    (From<PackageLineage> for UpstreamState — no network, no hashing)
        |
        v
    Return list with has_changes = false for every package
        |
        v
    Leptos renders the full list immediately

Phase 2 — Heavy (per-package, async)
    For each PackageItem, spawn_local calls refresh_package_status()
        |
        v
    Tauri command fetches latest hash from S3 (network)
    and walks local files to detect changes (hashing)
        |
        v
    Returns fresh status + has_changes
        |
        v
    RwSignal updates trigger reactive UI changes:
    buttons appear/disappear, Commit highlights, Pull disables
```

While the heavy phase is in flight, each row shows a small spinner
and the menu buttons pulse at reduced opacity. A hover tooltip
reads "Syncing with remote and scanning local files for changes...".

### User action cycle

When the user clicks a button (push, commit, pull, etc.):

```text
Leptos event handler fires (on:click)
    |
    v
ui_locked.set(true) — disables UI via reactive signal
    |
    v
spawn_local(async { commands::action(...).await })
    |                                        (Tauri IPC)
    v
Rust command executes the operation (e.g. push_package)
    |
    v
Returns Ok(success_message) or Err(error_message)
    |
    v
notification.set(Some(Notification::Success(msg)))
    or
notification.set(Some(Notification::Error(msg)))
    |
    v
Navigate to next page or reload current page
```

### Popup cycle

Popups (ignore, set-remote, create-package) are **Leptos
components** controlled by a `RwSignal<bool>`:

```text
User clicks trigger button
    |
    v
show_popup.set(true) — signal drives conditional rendering
    |
    v
<Show when=move || show_popup.get()>
    <PopupComponent on_submit=... on_cancel=... />
</Show>
    |
    v
Popup renders over overlay, user fills form
    |
    v
On submit: spawn_local calls Tauri command,
           show_popup.set(false), page reloads
```

## Routing

Routes are defined in `main.rs` using leptos\_router:

| Path | Component | Query params |
|---|---|---|
| `/` | redirect | → `/installed-packages-list` |
| `/installed-packages-list` | `InstalledPackagesList` | |
| `/installed-package` | `InstalledPackage` | `namespace`, `filter` |
| `/commit` | `Commit` | `namespace` |
| `/merge` | `Merge` | `namespace` |
| `/login` | `Login` | `host`, `back` |
| `/error` | `Error` | `host`, `back`, `message` |
| `/settings` | `Settings` | |
| `/setup` | `Setup` | |
| `/remote-package` | `RemotePackage` | `uri` |

Query parameters are read via `use_query_map()`. Navigation uses
`use_navigate()` for client-side transitions.

## Component Pattern

Every page component follows the same structure:

```rust
#[component]
pub fn PageName() -> impl IntoView {
    let notification = RwSignal::new(None);
    let ui_locked = RwSignal::new(false);

    // Async data fetch — runs on mount
    let data = LocalResource::new(move || async move {
        commands::get_page_data(params).await
    });

    view! {
        // Show spinner while loading
        <Suspense fallback=move || view! {
            <Layout notification=notification>
                <Spinner />
            </Layout>
        }>
            {move || Suspend::new(async move {
                match data.await {
                    Ok(d) => view! {
                        <PageContent data=d notification ui_locked />
                    }.into_any(),
                    Err(e) => {
                        // Redirect to login/setup, or show error
                        error_handler::handle(e, notification)
                    }
                }
            })}
        </Suspense>
    }
}
```

Key conventions:

- **Data fetching**: `LocalResource` triggers on mount; the
  component awaits it inside `Suspend::new`
- **Loading state**: `Suspense` with `<Spinner />` fallback
- **Error handling**: `error_handler.rs` parses structured errors
  and redirects to `/login` or `/setup` when needed
- **UI locking**: `RwSignal<bool>` passed to `Layout`, which adds
  a `.disabled` CSS class during async operations
- **Derived state**: `Memo::new` for filtered/computed views
- **Async actions**: `leptos::task::spawn_local` for button handlers

## IPC Bridge

Two layers connect Leptos components to Tauri commands:

### Low-level (`tauri.rs`)

```rust
pub async fn invoke<A, R>(cmd: &str, args: &A) -> Result<R, String>
```

Calls `window.__TAURI__.core.invoke()` via wasm-bindgen, serializing
args with serde-wasm-bindgen and deserializing the response.

### Typed wrappers (`commands.rs`)

Each Tauri command has a corresponding async function with proper
arg/return types:

```rust
pub async fn get_installed_package_data(
    namespace: &str,
    filter: &str,
) -> Result<InstalledPackageData, String> { ... }

pub async fn package_push(namespace: &str) -> Result<String, String> { ... }
```

Response DTOs (e.g. `InstalledPackageData`, `CommitData`,
`SettingsData`) are defined here with `#[derive(Deserialize)]`.
The backend returns **data structs**, not pre-rendered HTML -- the
Leptos components own all rendering.

## Layout Component

`components/layout.rs` provides the shared page shell:

```text
+--[appbar]----------------------------------------------+
| [logo]  [package URI]                  [refresh] [gear] |
+--[toolbar]---------------------------------------------+
| [breadcrumbs...]              [optional toolbar actions] |
+--[notification bar]------------------------------------+
| Success or error message (dismissible)                  |
+---------------------------------------------------------+
|                                                         |
|   [page content — children]                             |
|                                                         |
+---------------------------------------------------------+
```

### Notification

```rust
pub enum Notification {
    Success(String),
    Error(String),
}
```

Messages are rendered as text nodes (not `inner_html`), so they
are auto-escaped by Leptos. Users dismiss notifications by clicking
the overlay.

### Breadcrumbs

```rust
pub enum BreadcrumbItem {
    Link(BreadcrumbLink),   // Navigable parent page
    Current(String),        // Non-linked current page label
}
```

### Toolbar Actions

```rust
pub struct ToolbarActions(pub Box<dyn FnOnce() -> AnyView>);
```

Pages pass button components (e.g. `buttons::Push`, `buttons::Remove`)
to appear to the right of breadcrumbs. All buttons are defined in
`components/buttons/` — each is a thin wrapper around `IconButton`
(leading icon) or `ButtonCta` (trailing icon, always large).
`ButtonKind` centralizes icon paths and labels for icon buttons.

### UI Lock

When `ui_locked` signal is `true`, the layout adds a CSS class that
disables all interaction — used during async operations to prevent
double-submission.

## Auto-Update Checker

`components/update_checker.rs` renders an update notification bar
at the top of the app (outside the router). It polls
`commands::check_for_update()` on mount and offers Download, Install,
and Dismiss actions. Dismissal is persisted in localStorage for 5
minutes.

## CSS Organization

```text
ui/assets/css/
├── theme.css               # CSS custom properties (colors, spacing, fonts)
├── layout.css              # Layout helpers
├── spinner.css             # Loading spinner
├── components/             # Reusable component styles
│   ├── button.css
│   ├── entries-filter.css
│   ├── ignore-popup.css
│   └── popover.css
├── pages/                  # Per-page styles
│   ├── commit.css
│   ├── installed-package.css
│   ├── installed-packages-list.css
│   └── ...
├── views/                  # Layout section styles
│   ├── appbar.css
│   ├── breadcrumbs.css
│   ├── entry.css
│   ├── notify.css
│   ├── toolbar.css
│   └── ...
└── external/               # Vendored webfont @font-face files
    ├── 400.css
    ├── 500.css
    └── 700.css
```

All CSS is loaded via `<link>` tags in `index.html`. Trunk copies
the `assets/` directory to `dist/` at build time. There is no CSS
scoping -- all selectors are global, using a `qui-*` naming
convention.

## Key Differences from Previous Architecture

The frontend was migrated from Askama server-side templates +
TypeScript to Leptos client-side WASM. Key changes:

| Aspect | Before (Askama + TS) | After (Leptos) |
|---|---|---|
| Rendering | Backend builds HTML strings | Frontend renders reactively |
| Interactivity | Single `main.ts` re-attaches listeners | Leptos event handlers |
| Type safety | `data-*` attributes parsed manually | Typed Rust DTOs end-to-end |
| Routing | Static `.html` files + `#fragment` params | leptos\_router + query strings |
| State | None (full re-render on every action) | Reactive signals (`RwSignal`, `Memo`) |
| Popups | HTML strings built in TypeScript | Leptos components with signal toggles |
| Notifications | Raw HTML via `inner_html` (XSS risk) | Typed `Notification` enum (auto-escaped) |
| Build | Parcel (npm) | Trunk (no npm/node dependency) |
| UI disable | Imperative `lock_ui()` DOM manipulation | Reactive `ui_locked` signal |
