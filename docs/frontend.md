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
    buttons appear/disappear, Commit highlights, Pull's enabled state settles
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

## Design System: Radix Values, Primer Names

> **Scope**: this section governs the `ui/src/kit/` component kit and
> `ui/assets/css/kit/` tokens. The rest of this document still describes
> the v1 page code, which predates the kit.

### The decision

Two external authorities, one for each half of the problem:

| Half | Authority | What we take |
|---|---|---|
| **Values** | [Radix Colors](https://www.radix-ui.com/colors) | The hex scales, vendored verbatim as tier 1 |
| **Names** | [GitHub Primer](https://primer.style) | Token vocabulary and component names |
| **Code** | neither | Hand-written Leptos; no component library |

Radix publishes scales that are algorithmically consistent and
contrast-tested, but names them by number -- `--green-3` says nothing
about what it is for. Primer publishes a semantic vocabulary that says
exactly that -- `--bgColor-success-muted` -- but ships React components
we cannot use.

Taking values from one and names from the other means **every naming
argument has an external answer**. Nobody has to win a debate about
whether a pale green background is "subtle", "light", "tint", "washed",
or "muted": Primer already chose, and we follow. The cost is that our
divergences must be deliberate and written down, which is the rest of
this section.

Verified against Primer on 2026-08-07:
`primer.style/foundations/primitives/{color,size}` and
`primer.style/components`.

### Primer's conventions, as verified

**Tokens are property-first**, then tone, then role:

```text
--{property}Color-{tone}-{role}      e.g. --bgColor-success-muted
--{property}Color-{role}             e.g. --fgColor-muted
```

- Properties: `fgColor`, `bgColor`, `borderColor`
- Tones: `accent`, `attention`, `closed`, `danger`, `done`, `draft`,
  `neutral`, `open`, `severe`, `sponsors`, `success`, `upsell`
- Roles: `muted` (pale tint), `emphasis` (solid), `default`, `inset`,
  `inverse`, `disabled`, `onEmphasis` / `onInverse` (text over a solid)
- **There is no `subtle` role.** `subtle` belonged to `primer/css` v18
  (`--color-success-subtle`) and was retired when primitives moved to
  the property-first shape. Ours predates that check.
- Sizes are **value-named**, not index-named: `--base-size-8` is 8px
- Radii: `--borderRadius-small` 3px, `-medium` / `-default` 6px,
  `-large` 12px, `-full`
- Interaction states live in **component** tokens, not global ones:
  `--control-bgColor-hover`, `--button-primary-bgColor-rest`
- Focus: `--focus-outlineColor`

**Components** are unabbreviated nouns; `variant` names the visual axis;
`aria-label` is required on icon-only controls; and structured content
arrives as compound children (`FormControl.Caption`) rather than as
string props.

### Component names

| Ours | Primer | |
|---|---|---|
| `Button` | `Button` | ✅ |
| `IconButton` | `IconButton` | ✅ |
| `Select` | `Select` | ✅ |
| `TextInput` | `TextInput` | ✅ |
| `Dialog` | `Dialog` | ✅ |
| `Spinner` | `Spinner` | ✅ |
| `Card` | `Card` | ✅ (experimental in Primer) |
| `RelativeTime` | `RelativeTime` | ✅ |
| `Banner` | `Banner` | ✅ was `Notification` |
| `PageLayout` | `PageLayout` | ✅ was `Layout` |
| `Blankslate` | `Blankslate` | ✅ was `EmptyState` |
| `SegmentedControl` | `SegmentedControl` | ✅ was `ViewToggle` |
| `SkeletonBox` | `SkeletonBox` | ✅ was `Skeleton` |
| `GroupHeading` | `ActionList.GroupHeading` | ✅ was `GroupHeader` |
| `StateLabel` | `StateLabel` / `Label` | ⚠️ see below |
| `Field` | `FormControl` | ❌ tracked as `qhq-kt31` |
| `SearchInput` | — (`TextInput` + `trailingAction`) | ours |
| `ToggleRow` | — (wraps `ToggleSwitch`) | ours |
| `ListToolbar` | — | ours |
| `Countdown` | — | ours |
| `PackageRow`, `QueueRow`, `CauseRow`, `HostRow`, `FileRow`, `ZeroLine` | — | ours |

The bottom group is domain composition, not kit primitives. Primer's
nearest equivalents are `ActionList.Item` and `Timeline.Item`, neither of
which describes a row that carries a namespace, a state and an action.
These keep our names.

**`StateLabel` is a hybrid, deliberately.** Primer has two components
where we have one:

- Primer's `StateLabel` takes a `status` from a closed vocabulary
  (`issueOpened`, `pullMerged`, …), pairs each with an icon, and fills
  the background solid.
- Primer's `Label` takes an open `variant` for colour, tints the
  background pale, and has no icon.

Ours names a state from a closed vocabulary and carries a per-tone glyph,
which is `StateLabel`'s semantics; but it tints pale rather than filling
solid, which is `Label`'s treatment. We keep the name `StateLabel`
because the *closed vocabulary* is the load-bearing property -- and we
keep the pale treatment because the label repeats on every row of a list,
where ten solid fills would be louder than the content.

### Token names

| Ours | Primer equivalent | |
|---|---|---|
| `--q-fg-default` | `--fgColor-default` | ✅ words |
| `--q-fg-muted` | `--fgColor-muted` | ✅ words |
| `--q-fg-disabled` | `--fgColor-disabled` | ✅ words |
| `--q-fg-on-emphasis` | `--fgColor-onEmphasis` | ✅ words |
| `--q-border-default` | `--borderColor-default` | ✅ words |
| `--q-border-muted` | `--borderColor-muted` | ✅ words |
| `--q-{tone}-fg` | `--fgColor-{tone}` | ✅ words |
| `--q-accent-emphasis` | `--bgColor-accent-emphasis` | ✅ words |
| `--q-focus-ring` | `--focus-outlineColor` | ✅ concept |
| `--q-overlay-{hover,active,selected}` | `--control-bgColor-{hover,active,selected}` | ⚠️ alpha overlay vs resolved colour |
| `--q-border-strong` | `--borderColor-emphasis` | ❌ `strong` |
| `--q-canvas-{default,subtle,inset}` | `--bgColor-{default,muted,inset}` | ❌ `canvas` retired |
| `--q-{tone}-subtle` | `--bgColor-{tone}-muted` | ❌ `subtle` retired |
| `--q-{tone}-border` | `--borderColor-{tone}-muted` | ❌ property in the role slot |
| `--q-{tone}-fg-on-subtle` | — | ours; see below |
| `--q-canvas-page`, `--q-container` | — | ours |
| `--q-space-{1..12}` | `--base-size-{4..48}` | ❌ index vs value |
| `--q-radius` | `--borderRadius-small` | ❌ value diverges |
| `--q-text-{body,lead,title}` | `--text-body-size-medium`, … | ⚠️ collapsed |

Every name in the ❌ rows is a **structural** mismatch, not a spelling
one: our tokens are tone-first (`--q-success-subtle`) where Primer's are
property-first (`--bgColor-success-muted`). Tone-first has no room for
the property, which is why our fill role is called `subtle` and our
border role is called `border` -- two words on different axes sharing one
slot. Primer's shape has no such collision.

### Deliberate divergences

These do **not** get fixed. Each was decided against a measurement or an
explicit product call, and Primer is the weaker authority in each case:

| Divergence | Why |
|---|---|
| `--q-radius: 4px` everywhere, against Primer's 3 / 6 / 12px scale | A single radius was chosen deliberately -- there was no case where two radii were justifiable. 4px is the settled value. |
| `--q-{tone}-fg-on-subtle` exists at all | Primer has one `--fgColor-{tone}` per tone. Ours needs two: step 11 is what a glyph wants on a step-3 fill, and step 12 is what *text* needs to hold 4.5:1 there. Primer's single token cannot express a measurement we took. |
| No `emphasis` role for `success` / `attention` / `danger` | No solid tone fill passes AA with any foreground we have. A token that cannot be used legally should not exist. |
| `TextInput` takes `invalid: bool`, not `validationStatus: 'error' \| 'success'` | We have no success validation state and no plan for one. |
| `Field` takes `error: Option<String>`, not a `Validation` child with a required `variant` | Same reason, plus we have no compound-children pattern in Leptos. |
| `required` renders the word "Required", not Primer's `*` | An asterisk is a convention you have to have learned. |
| Prefix `--q-` on every token | Primer's are unprefixed and would collide in a webview that also loads vendored Radix scales. |
| The Radix scales themselves | Primer's own palette is not published as a scale we can vendor, and Radix's is contrast-tested per step. This is the whole point of splitting the two authorities. |

### Outstanding mismatches

Applied 2026-08-07 (`qhq-o2ov`), beyond the six component renames above:

- `Banner`'s tone prop is `variant: BannerVariant`, not `kind` --
  `variant` is Primer's word for the visual axis and is what `Button`,
  `Spinner` and `IconButton` already call it. Its `Error` variant became
  `Critical`, Primer's word for the tone.
- `Button.icon` → `leading_visual`. Primer's word, and it removes a
  collision: `icon` on `IconButton` is the entire content, while on
  `Button` it was a leading slot. One word, two meanings, one kit.
- `IconButtonVariant::Bare` → `Invisible`, `::Framed` → `Default`.
  `invisible` is Primer's established word for a chromeless button.
- `Blankslate`'s props took the words from Primer's sub-components:
  `title` → `heading`, `body` → `description`, `action` →
  `primary_action`. Renaming the component and leaving the props would
  have been a half-match.

Still outstanding:

1. **`Field` → `FormControl`** -- `qhq-kt31`, which also covers the
   `for`/`id` threading that the wrapping-`<label>` trick currently
   avoids, and moving `label` off `Select` / `SearchInput` /
   `SegmentedControl` and onto the control wrapper.
2. **`ButtonVariant`** lacks Primer's `Danger` and `Invisible`. Add when
   a call site needs one, not before.
3. **Token shape** -- adopting property-first
   (`--q-bgColor-success-muted`, `--q-borderColor-success-muted`,
   `--q-fgColor-success`, `--q-fgColor-success-onMuted`) across
   `_tokens.scss` and every `*.module.scss` that reads tier 2. This is
   the largest and the one with the clearest payoff: it retires
   `subtle`, `canvas` and `strong` in one pass and makes the
   fill-vs-border collision impossible to reintroduce.

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
