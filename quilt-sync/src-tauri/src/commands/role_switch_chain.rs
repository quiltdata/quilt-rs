//! The role switch, driven across every surface it moves.
//!
//! Each surface is already pinned at its own seam: the switch command's cache
//! flush in [`super::auth`], the roster's marks in [`super::package_list`],
//! the autosync pause in [`crate::autopull::tick`]. Those tests each hand
//! their subject a fresh [`RoleCache`] and a mock wired for one answer, so
//! none of them can show that a switch *changes* anything — every one of them
//! observes a single, static role.
//!
//! These tests share one [`FakeStack`], one [`RoleCache`] and one
//! [`Watcher`] across a switch, and assert on the difference before and
//! after. That is the whole point: the state that has to be invalidated is
//! the state the surfaces read, and only a test that runs a surface twice
//! around a switch can see it.
//!
//! ## What the fake models, and what it cannot
//!
//! [`FakeStack`] carries **two** roles, not one. `role` is what the registry
//! reports for `/me` — it changes the instant a switch lands. `signing_role`
//! is what an already-built S3 client is still signing as; it catches up only
//! when the client cache is dropped. That is the real hazard the switch
//! command's `clear_remote_client_cache` call exists to defeat: expiring the
//! stored credentials leaves a cached `aws_sdk_s3::Client` holding its own
//! copy of the old role's STS credentials for up to an hour, so the switch
//! would be silently fake. Because the fake reproduces that shape, deleting
//! the flush from `switch_role_command` fails these tests rather than
//! leaving them green.
//!
//! **Which surface reads which role is not arbitrary — it is the point.**
//! On this feature, knowing exactly which cache holds what has repeatedly
//! been the difference between working and silently fake, so the fake follows
//! production exactly:
//!
//! | Surface | Production path | Reads | Writes |
//! | --- | --- | --- | --- |
//! | `readable_buckets` | GraphQL over the HTTP client, bearer token | `role` | — |
//! | `refresh_roles` | GraphQL over the HTTP client, bearer token | `role` | **expires the stored credentials on any change it sees** |
//! | `switch_role` | GraphQL mutation, same transport | sets `role` | expires the stored credentials |
//! | `get_installed_package_status` | a real S3 GET through the cached client | `signing_role` | — |
//!
//! The `refresh_roles` row is the one that has already cost this feature a
//! bug. It looks like a read and is not: the engine holds a per-session
//! baseline of the role it last saw per host, and the moment the registry
//! answers with a different one it deletes that host's stored STS
//! credentials — half a flush, leaving the caller to drop the S3 clients that
//! are still signing as the old role. Modelling it as a pure read is what let
//! `get_roles` ship observing an out-of-band switch and doing nothing about
//! it. `model_for` therefore expects `clear_remote_client_cache` on the read
//! paths too, and the catalog-switch test below fails without it.
//!
//! So the roster's **light** phase (the bucket-list pre-filter) follows a
//! switch whether or not the clients were flushed — it never touched them.
//! Only the **heavy** phase and the autosync tick, which issue real S3 calls,
//! depend on the flush. Wiring the bucket list to `signing_role` would make
//! test 1's light-phase assertion fail on a missing flush too, which sounds
//! stricter but sends the next reader debugging the roster when the actual
//! breakage is in the heavy phase and autosync.
//!
//! It models the hazard; it does not prove the fix at the SDK level. Whether
//! dropping the client from `quilt-rs`'s map really releases the AWS SDK's
//! lazily-cached identity is unpinned on both sides of the seam and stays
//! that way — see the manual checklist.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use quilt_rs::RoleInfo;
use quilt_uri::Host;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;

use super::auth::RoleCache;
use super::auth::get_roles_command;
use super::auth::switch_role_command;
use super::package_list::InstalledPackageListItem;
use super::package_list::get_installed_packages_list_data_from_model;
use super::package_list::refresh_package_status_from_model;
use super::test_support::make_installed_package;
use crate::Error;
use crate::autopull::Watcher;
use crate::autopull::reporter::LogReporter;
use crate::autopull::tick::run_once;
use crate::model::MockQuiltModel;
use crate::quilt;
use crate::telemetry::Telemetry;

const HOST: &str = "test.quilt.dev";
const OPEN: &str = "open";
const LOCKED: &str = "locked";
const READ_ONLY: &str = "ReadOnly";
const READ_WRITE: &str = "ReadWrite";
const CURATOR: &str = "Curator";

/// The two rows every test uses: one in a bucket both roles can read, one in
/// a bucket only the wider role can.
const OPEN_ROW: (&str, &str) = ("team", "open");
const LOCKED_ROW: (&str, &str) = ("team", "locked");

/// A stand-in for the registry and S3, with the one piece of state a switch
/// is supposed to move.
///
/// See the module docs for why `role` and `signing_role` are separate.
struct FakeStack {
    /// What the registry answers — `/me` and the readable-bucket query, both
    /// GraphQL calls carrying a bearer token. A switch changes this
    /// immediately; no cache stands in the way.
    role: Mutex<String>,
    /// What the cached S3 clients are still signing as. Only a client-cache
    /// flush moves it, and only real S3 calls read it.
    signing_role: Mutex<String>,
    /// Buckets each role may read, in the order the stack lists the roles.
    readable: Vec<(String, Vec<String>)>,
    /// Hosts passed to `clear_remote_client_cache`, in call order. A global
    /// clear (`None`) records as the empty string, as in `erase_auth`.
    cache_clears: Mutex<Vec<String>>,
}

impl FakeStack {
    /// `readable` lists every role the user holds, each with the buckets it
    /// can read. The first entry is the active role.
    fn new(readable: &[(&str, &[&str])]) -> Arc<Self> {
        let active = readable
            .first()
            .expect("a stack with no roles is not a stack")
            .0
            .to_string();
        Arc::new(Self {
            role: Mutex::new(active.clone()),
            signing_role: Mutex::new(active),
            readable: readable
                .iter()
                .map(|(role, buckets)| {
                    (
                        (*role).to_string(),
                        buckets.iter().map(|b| (*b).to_string()).collect(),
                    )
                })
                .collect(),
            cache_clears: Mutex::new(Vec::new()),
        })
    }

    fn available(&self) -> Vec<String> {
        self.readable.iter().map(|(role, _)| role.clone()).collect()
    }

    /// The registry's answer: the role the user has actually switched to.
    fn roles(&self) -> RoleInfo {
        RoleInfo {
            current: self.role.lock().expect("role").clone(),
            available: self.available(),
        }
    }

    fn buckets_of(&self, role: &str) -> Vec<String> {
        self.readable
            .iter()
            .find(|(held, _)| held == role)
            .map(|(_, buckets)| buckets.clone())
            .unwrap_or_default()
    }

    /// The registry's readable-bucket list. A GraphQL call in production, so
    /// it answers for the role the user has switched to — the cached S3
    /// clients are not on this path at all.
    fn readable_buckets(&self) -> Vec<String> {
        self.buckets_of(&self.role.lock().expect("role"))
    }

    /// What a real S3 call gets, which is what the *signing* role may reach —
    /// stale until the client cache is dropped.
    fn can_read(&self, bucket: &str) -> bool {
        self.buckets_of(&self.signing_role.lock().expect("signing role"))
            .iter()
            .any(|b| b == bucket)
    }

    /// Server-side switch. Deliberately leaves `signing_role` alone.
    fn switch(&self, role: &str) -> Result<RoleInfo, Error> {
        if !self.available().iter().any(|held| held == role) {
            return Err(Error::General(format!("role {role} not held")));
        }
        *self.role.lock().expect("role") = role.to_string();
        Ok(self.roles())
    }

    /// The flush. Only now do new calls sign as the new role.
    fn clear_client_cache(&self, host: Option<&Host>) {
        self.cache_clears
            .lock()
            .expect("cache clears")
            .push(host.map(ToString::to_string).unwrap_or_default());
        let role = self.role.lock().expect("role").clone();
        *self.signing_role.lock().expect("signing role") = role;
    }

    fn cache_clears(&self) -> Vec<String> {
        self.cache_clears.lock().expect("cache clears").clone()
    }

    /// Flushes recorded after the first `since`, so a test can attribute
    /// them to one step. Reading the role flushes too (see the module docs),
    /// so the total is never a clean signal on its own.
    fn cache_clears_since(&self, since: usize) -> Vec<String> {
        self.cache_clears().split_off(since)
    }
}

/// The bucket a row lives in. `team/locked` is the restricted one.
fn bucket_for(namespace: &Namespace) -> &'static str {
    if namespace == &Namespace::from(LOCKED_ROW) {
        LOCKED
    } else {
        OPEN
    }
}

fn manifest_uri_for(namespace: &Namespace) -> ManifestUri {
    ManifestUri {
        origin: Some(HOST.parse().expect("host")),
        bucket: bucket_for(namespace).to_string(),
        namespace: namespace.clone(),
        hash: "abcdef".to_string(),
    }
}

/// What S3 returns when the signing role cannot reach the bucket.
fn access_denied(bucket: &str) -> Error {
    Error::Quilt(quilt::Error::S3(quilt::S3Error::new(
        quilt::S3ErrorKind::AccessDenied(format!("s3://{bucket}/x")),
    )))
}

/// A `MockQuiltModel` that answers from `stack` at call time.
///
/// Every role-dependent expectation is a `returning` closure reading the
/// shared state, not a canned sequence — so one model instance answers
/// differently before and after the switch, which is what lets a single test
/// run a surface twice and compare.
fn model_for(stack: &Arc<FakeStack>) -> MockQuiltModel {
    let mut model = MockQuiltModel::new();

    model.expect_get_installed_packages_list().returning(|| {
        Ok(vec![
            make_installed_package(OPEN_ROW),
            make_installed_package(LOCKED_ROW),
        ])
    });
    model
        .expect_get_installed_package()
        .returning(|namespace| Ok(Some(make_installed_package(namespace.clone()))));
    model
        .expect_get_installed_package_lineage()
        .returning(|package| {
            Ok(quilt::lineage::PackageLineage::from_remote(
                manifest_uri_for(&package.namespace),
                "abcdef".to_string(),
            ))
        });

    let s3 = Arc::clone(stack);
    model
        .expect_get_installed_package_status()
        .returning(move |package, _| {
            let bucket = bucket_for(&package.namespace);
            if s3.can_read(bucket) {
                Ok(quilt::lineage::InstalledPackageStatus::default())
            } else {
                Err(access_denied(bucket))
            }
        });

    let buckets = Arc::clone(stack);
    model
        .expect_readable_buckets()
        .returning(move |_| Ok(buckets.readable_buckets()));

    let registry = Arc::clone(stack);
    model
        .expect_refresh_roles()
        .returning(move |_| Ok(registry.roles()));

    let switcher = Arc::clone(stack);
    model
        .expect_switch_role()
        .returning(move |_, role| switcher.switch(role));

    let flusher = Arc::clone(stack);
    model
        .expect_clear_remote_client_cache()
        .returning(move |host: Option<Host>| flusher.clear_client_cache(host.as_ref()));

    model
}

/// A watcher with no background task and pull enabled, so `run_once` gets
/// past its both-directions-off shortcut. Push stays off: this is about
/// denials on the read path, and an enabled push branch would drag the
/// publish flow into the fixture.
async fn pulling_watcher() -> Watcher {
    let watcher = Watcher::new_for_test(Arc::new(LogReporter));
    watcher.inner_for_test().settings.write().await.pull.enabled = true;
    watcher
}

fn row<'a>(
    packages: &'a [InstalledPackageListItem],
    namespace: &str,
) -> &'a InstalledPackageListItem {
    packages
        .iter()
        .find(|item| item.namespace == namespace)
        .unwrap_or_else(|| panic!("{namespace} in roster"))
}

/// Fetch the roster the way the Tauri command does, with no pauses to show.
///
/// The empty paused map means the `#[tauri::command]` wrapper's projection of
/// `watcher.snapshot().paused` into `paused_reasons` is **not** exercised
/// here; these tests enter one level below it.
async fn roster(model: &MockQuiltModel, roles: &RoleCache) -> Vec<InstalledPackageListItem> {
    get_installed_packages_list_data_from_model(
        model,
        roles,
        &Telemetry::default(),
        &HashMap::new(),
    )
    .await
    .expect("roster")
    .packages
}

/// The switch, called exactly as the Tauri command calls it.
async fn switch_to(
    model: &MockQuiltModel,
    roles: &RoleCache,
    watcher: &Watcher,
    role: &str,
) -> Result<(), Error> {
    switch_role_command(model, roles, watcher, &Telemetry::default(), HOST, role).await?;
    Ok(())
}

/// The load-bearing sequence, in one test: a role that cannot reach a bucket
/// leaves the row marked; the switch flushes the clients that were signing as
/// it; the next roster fetch — light phase and heavy phase both — comes back
/// clean.
///
/// The two phases are sensitive to different things, and the difference is
/// worth knowing before debugging a failure here. The **light** phase reads
/// the registry's bucket list, so it follows the switch on its own; it fails
/// only if the switch itself did not land. The **heavy** phase issues a real
/// S3 call, so it is the assertion that fails when
/// `clear_remote_client_cache` is missing or runs before the switch — the
/// cached client would still be signing as the old role.
#[tokio::test]
async fn a_switch_reaches_s3_and_unmarks_the_rows_the_new_role_can_read() {
    let stack = FakeStack::new(&[(READ_ONLY, &[OPEN]), (READ_WRITE, &[OPEN, LOCKED])]);
    let model = model_for(&stack);
    let roles = RoleCache::default();
    let watcher = pulling_watcher().await;

    let before = roster(&model, &roles).await;
    assert!(
        row(&before, "team/locked").no_access,
        "a bucket the active role cannot read must be marked"
    );
    assert!(
        !row(&before, "team/open").no_access,
        "a bucket it can read must not be"
    );

    let before_switch = stack.cache_clears().len();
    switch_to(&model, &roles, &watcher, READ_WRITE)
        .await
        .expect("switch");

    assert_eq!(
        stack.cache_clears_since(before_switch),
        vec![HOST.to_string()],
        "the switch must drop the host's cached S3 clients, or they keep signing as the old role"
    );

    let after = roster(&model, &roles).await;
    assert!(
        !row(&after, "team/locked").no_access,
        "the new role reads this bucket, so the mark must go"
    );
    assert!(
        !row(&after, "team/open").no_access,
        "the row that was always readable must stay unmarked"
    );

    // The heavy phase is the authoritative answer, and the only one here that
    // goes through a cached S3 client. This is the flush's assertion.
    let refreshed = refresh_package_status_from_model(
        &model,
        &roles,
        &Telemetry::default(),
        &Namespace::from(LOCKED_ROW),
    )
    .await
    .expect("refresh");
    assert!(
        !refreshed.no_access,
        "the per-row status call must succeed under the new role"
    );
    assert_eq!(refreshed.status, "up_to_date");
}

/// A switch made in the **web catalog** has to reach S3 as well as the UI.
///
/// This is not an edge case: the feature's own premise is that a switch is
/// server-side and global, so the desktop app can only ever *observe* one
/// made elsewhere. It observes it on a read — `get_roles`, when Settings
/// opens — and that read is not free of consequences. The engine deletes the
/// host's stored credentials the moment it sees a role it did not last see,
/// which is half a flush; the other half is dropping the S3 clients still
/// holding the old role's credentials in the SDK's identity cache.
///
/// Stopping halfway is the worst state available. The selector shows the new
/// role and the registry-backed surfaces agree with it, while every real S3
/// call is still the old role for up to an hour — so the roster reads
/// "Current role `ReadOnly` has no access" next to a switcher displaying
/// `ReadWrite`, and re-picking that same `ReadWrite` is a no-op because it is
/// already selected. Only a restart recovers.
#[tokio::test]
async fn a_switch_made_in_the_catalog_reaches_s3_when_settings_reads_the_role() {
    let stack = FakeStack::new(&[(READ_ONLY, &[OPEN]), (READ_WRITE, &[OPEN, LOCKED])]);
    let model = model_for(&stack);
    let roles = RoleCache::default();

    let before = roster(&model, &roles).await;
    assert!(
        row(&before, "team/locked").no_access,
        "ReadOnly cannot reach this bucket, so the row starts marked"
    );

    // The user switches role in the web catalog. Nothing tells the app.
    stack.switch(READ_WRITE).expect("catalog switch");
    let before_settings = stack.cache_clears().len();

    // They open Settings, which reads the role afresh — the observation.
    let data = get_roles_command(&model, &roles, HOST)
        .await
        .expect("roles");
    assert_eq!(
        data.current, READ_WRITE,
        "the selector must show the role that is now active"
    );
    assert_eq!(
        stack.cache_clears_since(before_settings),
        vec![HOST.to_string()],
        "observing the switch must also drop the clients still signing as the old role"
    );

    // The authoritative surface: a real S3 call, which is what the clients
    // sign. This is what a display-only fix leaves broken.
    let refreshed = refresh_package_status_from_model(
        &model,
        &roles,
        &Telemetry::default(),
        &Namespace::from(LOCKED_ROW),
    )
    .await
    .expect("refresh");
    assert!(
        !refreshed.no_access,
        "the role Settings just showed must be the role S3 signs as"
    );
    assert_eq!(
        refreshed.role_switch_host, None,
        "a readable row carries no switch affordance"
    );
}

/// A switch into a role that is *also* denied keeps the mark — and renames
/// it. The role name comes from the shared [`RoleCache`], so this is what
/// catches a switch that forgets to invalidate it: the row would keep
/// quoting a role the user has already left, and the advice to switch would
/// look like it had been ignored.
#[tokio::test]
async fn the_mark_names_the_role_the_user_switched_to() {
    let stack = FakeStack::new(&[(READ_ONLY, &[OPEN]), (CURATOR, &[OPEN])]);
    let model = model_for(&stack);
    let roles = RoleCache::default();
    let watcher = pulling_watcher().await;

    let before = roster(&model, &roles).await;
    assert_eq!(
        row(&before, "team/locked").no_access_reason.as_deref(),
        Some("Current role ReadOnly has no access to this bucket")
    );

    switch_to(&model, &roles, &watcher, CURATOR)
        .await
        .expect("switch");

    let after = roster(&model, &roles).await;
    let locked = row(&after, "team/locked");
    assert!(
        locked.no_access,
        "the new role cannot read it either, so the mark stands"
    );
    assert_eq!(
        locked.no_access_reason.as_deref(),
        Some("Current role Curator has no access to this bucket"),
        "the mark must name the role now active, not the one the switch left"
    );
    assert_eq!(
        locked.role_switch_host.as_deref(),
        Some(HOST),
        "another role is still held, so the affordance stays"
    );
}

/// The remedy the banner names has to be the remedy that works.
///
/// A real tick creates the pause here — not `pause_for_test` — so the pause
/// the switch clears is the one autosync actually produces, keyed the same
/// way and carrying the role name the tick resolved. And the tick is run
/// again afterwards: releasing the pause would be pointless if the very next
/// tick re-denied and re-paused.
#[tokio::test]
async fn a_switch_releases_the_pause_a_denial_created_and_the_next_tick_proceeds() {
    let stack = FakeStack::new(&[(READ_ONLY, &[OPEN]), (READ_WRITE, &[OPEN, LOCKED])]);
    let model = model_for(&stack);
    let roles = RoleCache::default();
    let watcher = pulling_watcher().await;

    run_once(&model, &roles, watcher.inner_for_test())
        .await
        .expect("first tick");

    let paused = watcher.snapshot().await.paused;
    assert_eq!(
        paused.len(),
        1,
        "only the denied namespace pauses, got {paused:?}"
    );
    assert_eq!(paused[0].namespace, "team/locked");
    assert_eq!(paused[0].reason, "roleDenied");
    assert_eq!(
        paused[0].message.as_deref(),
        Some(READ_ONLY),
        "the banner names the role the tick was refused under"
    );

    switch_to(&model, &roles, &watcher, READ_WRITE)
        .await
        .expect("switch");

    assert!(
        watcher.snapshot().await.paused.is_empty(),
        "a role denial is the one pause no manual action can clear, so the switch must"
    );

    run_once(&model, &roles, watcher.inner_for_test())
        .await
        .expect("second tick");

    assert!(
        watcher.snapshot().await.paused.is_empty(),
        "the new role reads the bucket, so the tick that follows the switch must not re-pause"
    );
}

/// A switch the stack refuses changes nothing anywhere: the clients keep
/// signing as the role that is still active, and the row keeps the mark and
/// the name it already had.
#[tokio::test]
async fn a_refused_switch_leaves_every_surface_as_it_was() {
    let stack = FakeStack::new(&[(READ_ONLY, &[OPEN])]);
    let model = model_for(&stack);
    let roles = RoleCache::default();
    let watcher = pulling_watcher().await;

    let before = roster(&model, &roles).await;
    assert!(row(&before, "team/locked").no_access);

    let before_switch = stack.cache_clears().len();
    let result = switch_to(&model, &roles, &watcher, READ_WRITE).await;
    assert!(
        result.is_err(),
        "switching to a role the user does not hold must fail"
    );
    assert!(
        stack.cache_clears_since(before_switch).is_empty(),
        "a refused switch must not drop clients that are still valid"
    );

    let after = roster(&model, &roles).await;
    assert_eq!(
        row(&after, "team/locked").no_access_reason.as_deref(),
        Some("Current role ReadOnly has no access to this bucket"),
        "nothing moved, so the mark must read exactly as before"
    );
}
