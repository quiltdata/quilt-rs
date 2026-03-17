# OAuth Login Feature Review

Review of the OAuth 2.1 Authorization Code + PKCE login implementation
across `quilt-rs` (library) and `quilt-sync` (Tauri app).

## Critical

### Code-based login removed from QuiltSync UI

The login page (`login.html`) only shows the OAuth button — the old
code-paste form was removed entirely. Not all stacks have been updated
to support OAuth/Connect yet. Users on older stacks have no way to
log in from QuiltSync.

The CLI still supports code-based login (`quilt_rs login --host H --code C`),
and the backend `Auth::login()` method that accepts a refresh token is
still present in `quilt-rs/src/auth.rs`. The gap is only in the
QuiltSync UI.

**Action:** bring back the code-paste input as a fallback on the login
page, either always visible or shown when OAuth fails / when the stack
doesn't advertise Connect support.

### Manual testing needed for token refresh flow

The automatic token refresh via `refresh_oauth_tokens` is new and
has not been tested against a real Connect server yet. Before merging,
manually verify:

1. Log in via OAuth on a Connect-enabled stack.
2. Wait for the access token to expire (or manually edit the stored
   `tokens.json` to set `expires_at` in the past).
3. Trigger an S3 operation (e.g. pull a package).
4. Confirm that QuiltSync refreshes tokens transparently without
   showing the login page.

## Issues

### ~~No token expiry check before using access tokens~~ (FIXED)

Fixed: `get_credentials_or_refresh` now checks `tokens.expires_at`
before using the access token.

### ~~No refresh token rotation~~ (FIXED)

Fixed: added `refresh_oauth_tokens` which exchanges the refresh token
for new tokens via the Connect token endpoint (RFC 6749 §6).

### Pending OAuth state has no expiry or cleanup

`OAuthState` in `quilt-sync/src-tauri/src/oauth.rs` stores pending auth
entries in a `HashMap` but never cleans them up.
If a user starts login but never completes the callback, stale entries
accumulate for the lifetime of the process.
Consider adding a timestamp and TTL check in `take_params`.

### Secrets logged at debug level

`AuthIo::write_credentials` and `AuthIo::write_tokens` in
`quilt-rs/src/io/storage/auth.rs` log the full structs (access keys,
secret keys, session tokens, access/refresh tokens) via `debug!`.
These should be redacted or removed.

### `redirect_uri` host value is not URL-encoded

`redirect_uri()` in `quilt-sync/src-tauri/src/oauth.rs` interpolates
the host directly:

```rust
format!("quilt://auth/callback?host={host}")
```

Low risk with current hostnames but fragile if a host ever contains
special characters.

### Auto-click on login page

`main.ts` auto-triggers the OAuth button the moment the login page loads.
If something goes wrong (DCR failure, popup blocker) the user sees an error
with no chance to read context first.
Re-renders also re-fire the click.

### Device flow fallback is implicit

In `uri.rs`, when `take_params` returns `Ok(None)` the code silently falls
back to the old device flow (`model::login`).
The `code` parameter is reused with different semantics
(authorization code vs. refresh token) and nothing in the logs
distinguishes which flow was actually used.

## Tech Debt

### Storage ownership in tests

`quilt-rs/src/auth.rs` has a commented-out test block noting that `Auth`
owns a `Storage` clone, so written credentials can't be read back through
a second `AuthIo`.
The TODO suggests using `Rc<Storage>` (or `Arc<Storage>`).

### Deep nesting in `navigate_after_login`

`commands.rs` `navigate_after_login` has four levels of nested `match`.
Could be flattened with early returns or the `?` operator.

## What Works Well

- PKCE implementation is correct (S256, proper verifier length, tested).
- CSRF protection via `state` parameter with mismatch detection.
- DCR client caching with stale `redirect_uri` detection and re-registration.
- Credential expiry checking on read.
- Good test coverage across all layers.
- Clean separation: `quilt-rs` handles the OAuth protocol,
  `quilt-sync` handles UI and deep links.
