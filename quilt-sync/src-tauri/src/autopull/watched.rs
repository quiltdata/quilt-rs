//! The walks of watched packages, kept so the tick can skip its own.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use quilt_uri::Namespace;

use crate::quilt::lineage::InstalledPackageStatus;
use crate::quilt::lineage::PackageLineage;

/// The last walk of each package the folder watcher vouches for: watched, on a
/// local volume, with no events lost.
///
/// A walk is reusable while no signal came after it started, for at most
/// [`MAX_AGE`]: the tick's own walks are what run the stat cache's daily full
/// hash, and they catch a write no event reported. Marks come from one
/// counter, so a walk that started before a signal, a new watch or a lost one
/// is older than it, even across a package dropped and watched again.
#[derive(Default)]
pub(crate) struct WatchedWalks(Mutex<State>);

const MAX_AGE: Duration = Duration::from_hours(1);

#[derive(Default)]
struct State {
    counter: u64,
    packages: BTreeMap<Namespace, Package>,
}

struct Package {
    home: PathBuf,
    /// When the package was last heard from, or started being watched.
    stale_at: u64,
    walk: Option<Walk>,
}

struct Walk {
    started: u64,
    at: Instant,
    lineage: PackageLineage,
    status: InstalledPackageStatus,
}

/// A walk in progress, from [`WatchedWalks::begin`]. `None` inside when the
/// package is not vouched for, and the walk is not kept.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WalkStart(Option<u64>);

impl State {
    fn next(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }
}

impl WatchedWalks {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Vouch for exactly `live`, each at its home. A package new to it, or
    /// at a new home, starts with no walk.
    pub(crate) fn vouch_for(&self, live: &BTreeMap<Namespace, PathBuf>) {
        let mut state = self.state();
        state
            .packages
            .retain(|ns, package| live.get(ns) == Some(&package.home));
        for (namespace, home) in live {
            if !state.packages.contains_key(namespace) {
                let stale_at = state.next();
                state.packages.insert(
                    namespace.clone(),
                    Package {
                        home: home.clone(),
                        stale_at,
                        walk: None,
                    },
                );
            }
        }
    }

    /// Stop vouching for `namespace`: its watch is gone, or its events can't be
    /// trusted until the next reconcile.
    pub(crate) fn forget(&self, namespace: &Namespace) {
        self.state().packages.remove(namespace);
    }

    pub(crate) fn clear(&self) {
        self.state().packages.clear();
    }

    /// The watcher heard `namespace` change: its kept walk is stale.
    pub(crate) fn on_signal(&self, namespace: &Namespace) {
        let mut state = self.state();
        let mark = state.next();
        if let Some(package) = state.packages.get_mut(namespace) {
            package.stale_at = mark;
        }
    }

    /// Call just before reading the lineage a walk is matched against.
    pub(crate) fn begin(&self, namespace: &Namespace) -> WalkStart {
        let mut state = self.state();
        let mark = state.next();
        WalkStart(state.packages.contains_key(namespace).then_some(mark))
    }

    /// Keep a finished walk, if its package is vouched for since it began.
    pub(crate) fn record(
        &self,
        namespace: &Namespace,
        start: WalkStart,
        lineage: PackageLineage,
        status: &InstalledPackageStatus,
    ) {
        let Some(started) = start.0 else {
            return;
        };
        let mut state = self.state();
        let Some(package) = state.packages.get_mut(namespace) else {
            return;
        };
        if package.stale_at > started || package.walk.as_ref().is_some_and(|w| w.started > started)
        {
            return;
        }
        package.walk = Some(Walk {
            started,
            at: Instant::now(),
            lineage,
            status: status.clone(),
        });
    }

    /// The kept walk of `namespace`, when nothing was heard since it started.
    pub(crate) fn reusable(
        &self,
        namespace: &Namespace,
    ) -> Option<(PackageLineage, InstalledPackageStatus)> {
        let state = self.state();
        let package = state.packages.get(namespace)?;
        let walk = package.walk.as_ref()?;
        (walk.started > package.stale_at && walk.at.elapsed() < MAX_AGE)
            .then(|| (walk.lineage.clone(), walk.status.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns() -> Namespace {
        ("acme", "demo").into()
    }

    fn live() -> BTreeMap<Namespace, PathBuf> {
        BTreeMap::from([(ns(), PathBuf::from("/pkg"))])
    }

    fn walked(walks: &WatchedWalks, start: WalkStart) {
        walks.record(
            &ns(),
            start,
            PackageLineage::default(),
            &InstalledPackageStatus::default(),
        );
    }

    #[test]
    fn a_walk_is_kept_until_a_signal() {
        let walks = WatchedWalks::default();
        walks.vouch_for(&live());
        walked(&walks, walks.begin(&ns()));
        assert!(walks.reusable(&ns()).is_some());

        walks.on_signal(&ns());
        assert!(walks.reusable(&ns()).is_none());
    }

    #[test]
    fn a_signal_during_a_walk_makes_it_stale() {
        let walks = WatchedWalks::default();
        walks.vouch_for(&live());
        let start = walks.begin(&ns());
        walks.on_signal(&ns());
        walked(&walks, start);
        assert!(walks.reusable(&ns()).is_none());
    }

    #[test]
    fn a_walk_from_before_the_watch_is_not_kept() {
        let walks = WatchedWalks::default();
        let start = walks.begin(&ns());
        walks.vouch_for(&live());
        walked(&walks, start);
        assert!(walks.reusable(&ns()).is_none());

        walks.vouch_for(&live());
        let start = walks.begin(&ns());
        walks.forget(&ns());
        walks.vouch_for(&live());
        walked(&walks, start);
        assert!(
            walks.reusable(&ns()).is_none(),
            "the watch was lost and made again while it walked"
        );
    }

    #[test]
    fn a_new_home_drops_the_walk() {
        let walks = WatchedWalks::default();
        walks.vouch_for(&live());
        walked(&walks, walks.begin(&ns()));
        walks.vouch_for(&BTreeMap::from([(ns(), PathBuf::from("/elsewhere"))]));
        assert!(walks.reusable(&ns()).is_none());
    }

    #[test]
    fn an_old_walk_is_not_reused() {
        let walks = WatchedWalks::default();
        walks.vouch_for(&live());
        walked(&walks, walks.begin(&ns()));
        walks
            .state()
            .packages
            .get_mut(&ns())
            .unwrap()
            .walk
            .as_mut()
            .unwrap()
            .at -= MAX_AGE;
        assert!(walks.reusable(&ns()).is_none());
    }

    #[test]
    fn a_package_no_longer_vouched_for_has_no_walk() {
        let walks = WatchedWalks::default();
        walks.vouch_for(&live());
        walked(&walks, walks.begin(&ns()));
        walks.vouch_for(&BTreeMap::new());
        assert!(walks.reusable(&ns()).is_none());
    }
}
