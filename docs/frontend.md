# QuiltSync Frontend Architecture

> **Audience**: Contributors who need to understand how the desktop UI
> works end-to-end, from the Tauri webview to the Rust page renderers.

## Overview

QuiltSync is a Tauri v2 desktop application. The UI runs inside a
webview that loads static HTML shells, then delegates all rendering
to the Rust backend via IPC. The result is a **server-side rendering
pattern inside a desktop app**: Rust builds the entire page HTML on
every navigation, and a single TypeScript file wires up interactivity
after each render.

## Stack

| Layer | Technology | Location |
|---|---|---|
| Webview shell | Static HTML (one per page) | `quilt-sync/src/pages/*.html` |
| Interactivity | TypeScript (single file) | `quilt-sync/src/assets/js/main.ts` |
| Styles | Plain CSS (per-page and per-view) | `quilt-sync/src/assets/css/` |
| Templates | Askama (Jinja-like, compiled into Rust) | `quilt-sync/src/templates/` |
| Page renderers | Rust structs + Askama `#[derive(Template)]` | `quilt-sync/src-tauri/src/pages/` |
| UI components | Rust builder structs (button, breadcrumbs, entry, ...) | `quilt-sync/src-tauri/src/ui/` |
| Bundler | Parcel | `package.json` |
| IPC | Tauri commands (`#[tauri::command]`) | `quilt-sync/src-tauri/src/commands.rs` |

## Data Flow

### Page load cycle

Every page navigation follows the same cycle:

```text
Browser navigates to e.g. installed-package.html#namespace=foo/bar
    |
    v
DOMContentLoaded fires
    |
    v
main.ts calls invoke("load_page", { location: window.location.href })
    |                                                    (Tauri IPC)
    v
commands::load_page_command()
    |
    v
routes::Paths::from_str(location)   -- parse URL into typed enum
    |
    v
pages::load(model, path)            -- fetch data, build view struct
    |
    v
ViewXxx::create(model, ...).render() -- Askama renders HTML string
    |
    v
HTML string returned to JS via IPC
    |
    v
main.ts sets document.body.innerHTML = html
    |
    v
main.ts fires "page-is-ready" event
    |
    v
Event listeners re-attached for the new DOM
```

### User action cycle

When the user clicks a button (push, commit, pull, etc.):

```text
JS click handler reads data-* attributes from the button
    |
    v
invoke(command, data)                -- Tauri IPC
    |
    v
Rust command executes the operation (e.g. push_package)
    |
    v
Rust returns a notification HTML string (success or error)
    |
    v
JS injects notification into #notify
    |
    v
JS navigates to the next page (triggers the full page load cycle)
    or reloads the current page
```

### Popup cycle

Popups (ignore, set-remote, create-package) are **built entirely in
TypeScript** as inline HTML strings. They do not use Askama templates:

```text
JS click handler fires
    |
    v
JS builds popup HTML with string concatenation + escapeHtml()
    |
    v
JS injects HTML into #popup, makes overlay visible
    |
    v
JS attaches event listeners to popup elements
    |
    v
On submit: JS calls invoke(command, data), dismisses popup,
           triggers page reload
```

## Routing

Routes are defined in `routes.rs` as the `Paths` enum:

```rust
pub enum Paths {
    Commit(Namespace, EntriesFilter),
    InstalledPackage(Namespace, EntriesFilter),
    InstalledPackagesList,
    Login(Host, String),
    LoginError(Host, String, String),
    Merge(Namespace),
    RemotePackage(S3PackageUri),
    Settings,
    Setup,
}
```

Each variant maps to a static HTML shell file (`src/pages/*.html`)
and an Askama template (`src/templates/pages/*.html`). The URL format
uses fragments for parameters:

```text
installed-package.html#namespace=foo/bar&filter=unmodified
commit.html#namespace=foo/bar
login.html#host=open.quiltdata.com&back=installed-packages-list.html
```

Navigation from JS is a plain `window.location.assign(url)` which
triggers a full `DOMContentLoaded` -> `load_page` cycle.

## Rust-Side Rendering

### Page renderers (`src-tauri/src/pages/`)

Each page has a `ViewXxx` struct that:

1. Fetches data from the model (package status, entries, etc.)
2. Builds UI component structs (buttons, breadcrumbs, entries)
3. Holds an Askama `#[derive(Template)]` inner struct that references
   the `.html` template
4. Exposes a `.render()` method that returns `Result<String, Error>`

Example pattern:

```rust
// pages/installed_package.rs

pub struct ViewInstalledPackage { /* data fields */ }

#[derive(Template)]
#[template(path = "./pages/installed-package.html")]
struct TmplInstalledPackage<'a> {
    layout: Layout<'a>,
    entries: Vec<entry::ViewEntry>,
    // ...
}

impl ViewInstalledPackage {
    pub async fn create(model, namespace, filter) -> Result<Self> { /* ... */ }
    pub fn render(self) -> Result<String> {
        let tmpl = TmplInstalledPackage { /* ... */ };
        Ok(tmpl.render()?)
    }
}
```

### UI components (`src-tauri/src/ui/`)

Reusable Askama sub-templates with builder-pattern Rust structs:

- `btn::TmplButton` -- buttons with icon, label, href, data
  attributes, JS selector class
- `crumbs::TmplBreadcrumbs` -- navigation breadcrumb trail
- `entry::ViewEntry` -- file entry row with checkbox, size, status
- `layout::Layout` -- page shell (appbar, toolbar, notification
  area, popup overlay, action bar)
- `uri::TmplUri` -- package URI display
- `notify::TmplNotify` -- success/error notification snippets

### Layout template (`src/templates/components/layout.html`)

Every page extends the layout template, which provides:

- App bar (logo, package URI, refresh button, settings button)
- Toolbar (breadcrumbs, secondary action buttons)
- Notification area (`#notify`)
- Page content area (`{% block children %}`)
- Action bar (primary action button)
- Popup overlay (`#popup`)

## TypeScript Side

### Single-file architecture

All client-side logic lives in `src/assets/js/main.ts` (~1,400
lines). It is responsible for:

- **Page lifecycle**: calling `load_page` on `DOMContentLoaded`,
  replacing `body.innerHTML`, dispatching `page-is-ready`
- **Event binding**: the `page-is-ready` handler attaches ~30 click
  listeners using the `listen()` helper, which finds elements by CSS
  selector and reads `data-*` attributes
- **Command execution**: `execPageCommand()`, `execFormCommand()`,
  and `execInlineCommand()` call Tauri `invoke()` with different
  post-action behaviors (navigate, reload, or just show notification)
- **Popup builders**: `showIgnorePopup()`, `showSetOriginForm()`,
  `showCreatePackageForm()`, `showSetRemoteForm()` build HTML strings
  and attach event listeners manually
- **Auto-update UI**: check for updates, show download/install
  notifications

### Key helpers

```typescript
// Find elements and wire up click -> invoke pattern
listen(SELECTOR, ["data-attr1", "data-attr2"], async (data, button) => {
    await execPageCommand(COMMAND, data, optionalRedirectRoute);
});

// Execute a Tauri command that returns notification HTML
async function execPageCommand(command, data, redirect?) { /* ... */ }

// Replace body with Rust-rendered HTML
async function loadCurrentPage() {
    const page = await invoke("load_page", { location: window.location.href });
    document.body.innerHTML = page;
    window.dispatchEvent(new Event("page-is-ready"));
}
```

## CSS Organization

```text
src/assets/css/
  styles.css              -- entry point (imports theme + layout + spinner)
  theme.css               -- CSS custom properties (colors, spacing, fonts)
  layout.css              -- top-level grid
  spinner.css             -- loading spinner (shown before Rust renders)
  components/             -- reusable component styles
    button.css
    entries-filter.css
    ignore-popup.css
  pages/                  -- page-specific styles
    commit.css
    installed-package.css
    settings.css
    ...
  views/                  -- layout section styles
    actionbar.css
    appbar.css
    breadcrumbs.css
    entry.css
    toolbar.css
    ...
  external/               -- vendored font-face files
    400.css, 500.css, 700.css
```

Styles are loaded via `@import url(...)` inside Askama templates
(e.g. `layout.html` imports view CSS, page templates import
page-specific CSS). There is no CSS scoping or encapsulation --
all selectors are global.

## Size Inventory

| Layer | Files | Lines |
|---|---|---|
| TypeScript (`main.ts`) | 1 | ~1,400 |
| CSS (custom, excluding fonts) | ~25 | ~1,400 |
| Askama templates | ~25 | ~670 |
| Rust page renderers (`pages/`) | ~8 | ~1,500 |
| Rust UI components (`ui/`) | ~8 | ~800 |

## Known Limitations

1. **Single monolithic JS file** -- all selectors, event listeners,
   and popup builders live in one 1,400-line file with no way to
   scope behavior to a specific page.

2. **Re-attach-everything pattern** -- after every `innerHTML`
   replacement, `page-is-ready` binds ~30 listeners to the document.
   Most won't find matching elements on the current page.

3. **Two templating systems** -- Askama on the Rust side for pages,
   raw HTML strings in TypeScript for popups and notifications.

4. **No component model** -- there is no encapsulation of markup +
   style + behavior as a unit. CSS selectors are global, and the
   connection between a Rust UI struct, an Askama template, a CSS
   file, and a JS selector constant is implicit.

5. **Full-page reloads** -- every user action re-renders the entire
   page from Rust, even for small state changes.

6. **No type safety across the Rust/JS boundary** -- Rust renders
   HTML with `data-*` attributes that JS reads manually. A renamed
   attribute silently breaks behavior at runtime.
