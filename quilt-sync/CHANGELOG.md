<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     Put quilt-rs and quilt-uri updates under their respective `###` section.
     Between releases a cross-crate line names the upstream -dev version and
     compares against main; the release PR points it at the released tag.
     Head unreleased changes with the Cargo.toml version: `-dev`, no date
     (e.g. [v0.22.4-dev]), not [Unreleased]. If the cycle needs a bigger bump,
     rename both. The first PR to change the crate after a release opens the
     next patch `-dev` version and heading; the release PR drops `-dev` and
     adds the date.
     Describe the change since the last release, not each PR: when a PR makes
     an unreleased entry stale, rewrite it in place and append the PR link.
     Work no user can reach yet gets one "Under the hood" line, not a feature
     entry; it earns a real entry once reachable, even behind a switch.
     Never edit released sections.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v0.22.4-dev]

### Changed

- **Sync entire package** (Settings → Experimental) now only shows the choice on the package screen. A package already set to keep its whole contents keeps doing so, in background updates too, whether or not the experiment is on (<https://github.com/quiltdata/quilt-rs/pull/986>)
- **New main page** is now **New design preview** (Settings → Experimental, still off by default): one switch for the redesigned QuiltSync rather than one per page, so each part of the new design arrives under the switch you already turned on. If you had **New main page** on, it stays on and nothing is lost. The first launch after updating may open on a light window even with a dark desktop; from the next launch on it matches your desktop again (<https://github.com/quiltdata/quilt-rs/pull/980>, <https://github.com/quiltdata/quilt-rs/pull/983>)
- **New design preview** (Settings → Experimental, off by default) now brings the new top bar to every screen that has one — Settings, sign-in, a package, commit, merge and the rest — not only to the new main page. Only the top bar changes: the row under it, with where you are and the page's own buttons, stays as it was, and so does the page itself, in light and dark alike. **Refresh** does what it always did on those screens, except the one a `quilt+s3://` link opens on while it installs, where it is unavailable (<https://github.com/quiltdata/quilt-rs/pull/984>, <https://github.com/quiltdata/quilt-rs/pull/988>)
- The new main page (Settings → Experimental → **New design preview**) is polished further. The current page is unchanged by all of it:
  - **Refresh** now spins whenever the page is reloading, including a reload the background sync started, and cannot be pressed again until that finishes. It used to spin only after you pressed it, so the page could be reloading under a button that said nothing was happening (<https://github.com/quiltdata/quilt-rs/pull/970>)
  - **Recent files** grouped by package now orders owners as owners: `acme/plate` comes before `acme-labs/plate`. Sorted as plain text, the `-` put `acme-labs` first (<https://github.com/quiltdata/quilt-rs/pull/964>)
- When setting a package's S3 bucket fails, the message now names the bucket that was refused: "Could not set the remote to my-bucket: …" instead of "Failed to set remote: …" (<https://github.com/quiltdata/quilt-rs/pull/981>)
- Under the hood, the app's interface now carries a package's namespace as the validated address type rather than as plain text, the same change v0.22.3 made on the way from the backend (<https://github.com/quiltdata/quilt-rs/pull/964>)
- Under the hood, a rebuilt package screen in the new design is in the app but switched on nowhere: no setting or build offers it, only a constant in the source. So far it has a header whose commands all work and a context pane that lists the revisions this computer holds and which of them are published, and says which of the package's files this computer keeps, lets you keep the whole package and downloads the files it has not got yet, and, for a package that changed both here and in its bucket since you last synced, lets you keep your revision as the shared one or replace it with the published one, after saying what the replacement discards; and a file list that groups the package's files by folder, opens a downloaded file on a click, and says when the package has more files than it shows (<https://github.com/quiltdata/quilt-rs/pull/966>, <https://github.com/quiltdata/quilt-rs/pull/970>, <https://github.com/quiltdata/quilt-rs/pull/973>, <https://github.com/quiltdata/quilt-rs/pull/978>, <https://github.com/quiltdata/quilt-rs/pull/980>, <https://github.com/quiltdata/quilt-rs/pull/981>, <https://github.com/quiltdata/quilt-rs/pull/982>, <https://github.com/quiltdata/quilt-rs/pull/983>, <https://github.com/quiltdata/quilt-rs/pull/986>, <https://github.com/quiltdata/quilt-rs/pull/994>, <https://github.com/quiltdata/quilt-rs/pull/993>)
- Under the hood, the component gallery holds the installed-package page's file pane and the whole page as scenes, plus a form dialog and a confirmation dialog, each with tests of its own; the interface's DOM tests share one mount helper, and new ones pin the toggle row's and segmented control's behavior; and the design guide spells out when a state reads as needing attention and when as a failure (<https://github.com/quiltdata/quilt-rs/pull/961>, <https://github.com/quiltdata/quilt-rs/pull/962>, <https://github.com/quiltdata/quilt-rs/pull/963>, <https://github.com/quiltdata/quilt-rs/pull/965>, <https://github.com/quiltdata/quilt-rs/pull/972>, <https://github.com/quiltdata/quilt-rs/pull/977>, <https://github.com/quiltdata/quilt-rs/pull/979>)

### Fixed

- After **Promote my revision** on the merge page, background sync now resumes for that package. It used to stay stopped until QuiltSync was restarted (<https://github.com/quiltdata/quilt-rs/pull/994>)
- **Refresh** on the screen a `quilt+s3://` link opens on, while it installs, no longer opens the link's file a second time: there is nothing on that screen to refresh, so the button is unavailable. Going **Back** from the package that screen takes you to, or from Settings if you leave it that way, no longer returns to it and opens the file again either (<https://github.com/quiltdata/quilt-rs/pull/988>)
- A screen that fails to load shows one top bar, not two. Settings and the screen a `quilt+s3://` link opens on drew the error page's own bar under theirs (<https://github.com/quiltdata/quilt-rs/pull/988>)
- Downloading files from a bucket without versioning, after a later revision replaced some of them, installs the rest and skips those. Before, it placed the later revision's bytes, and the file read as modified though nobody touched it. The notice after the download says how many files were skipped and names them. A `quilt+s3://` link to one such file says it is no longer on the remote instead of opening the wrong one (<https://github.com/quiltdata/quilt-rs/pull/991>)
- Downloading a package's files no longer replaces a file of your own that is already at the same place in the package folder. If you had created a file there and not yet committed it, the download wrote the remote file over it and your file was lost; now the download stops with "A local file is already at …" naming it, and your file is left as it was (<https://github.com/quiltdata/quilt-rs/pull/986>)
- A package with more than 1000 files now lists its first 1000 in path order. It used to fill the list with changed files first and drop others that sort before them, so the list skipped around the package (<https://github.com/quiltdata/quilt-rs/pull/992>)

### quilt-rs

- Updated [from v0.39.1 to v0.40.0-dev](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.39.1...main) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.22.3] - 2026-09-18

### Fixed

- Every link to a package whose name contains `&` or `#` now carries the whole name. The name went into the address unescaped, so it ended at that character — `team/a&b` arrived as `team/a`, and anything after a `#` was dropped — on every link and redirect to a package's own page, its commit page and its merge page, from both the current main page and the new one (<https://github.com/quiltdata/quilt-rs/pull/945>)
- Signing in through your browser returns you to the page you started from. It could only return you to a page it had a name for, and the page the app opens on was not one of them, so it always finished on the installed packages list — which shows only with **New main page** on (Settings → Experimental), where signing in from the new page landed you on the old one (<https://github.com/quiltdata/quilt-rs/pull/944>)
- The buttons on a package's status banner keep their size when the message beside them is long. They used to be squeezed by it, narrowing as the text grew (<https://github.com/quiltdata/quilt-rs/pull/942>)

### Changed

- Under the hood, a package's namespace now crosses between the app's backend and its interface as the validated address type it already was, rather than being flattened to plain text on the way out. No behaviour changes and the wire format is identical (<https://github.com/quiltdata/quilt-rs/pull/946>)
- The new main page (Settings → Experimental → **New main page**, still off by default) is polished further. The current page is unchanged by all of it:
  - A row in **Recent files** can now copy that file's `quilt+s3` address to your clipboard, ready to paste into a terminal or a message. The button turns to a tick to confirm, and a package that lives only on your machine shows no button, since no `quilt+s3` address names it (<https://github.com/quiltdata/quilt-rs/pull/953>)
  - A card that cannot load its own data — **Autosync**, **Accounts**, your packages, recent files — now keeps its title, says it could not load, and offers **Try again**. Each used to draw nothing, which reads as an answer: no autosync on this machine, no accounts signed in (<https://github.com/quiltdata/quilt-rs/pull/939>)
  - Typing in the search box no longer rebuilds the whole list under it. Every row a search left standing was thrown away and built again on each keystroke; at 500 packages, eight keystrokes cost 208ms of that work and now cost 17ms (<https://github.com/quiltdata/quilt-rs/pull/949>)
  - A screen reader can move through the page: it has a title, each card is a named region, bucket headings are headings, and package rows, file rows and the queue are lists. They were an undifferentiated run of text with nothing to navigate by (<https://github.com/quiltdata/quilt-rs/pull/935>)
  - Four more things a screen reader was told wrongly: a Recent files row announced as a single button, hiding the package link and the three actions on it; a card of placeholders that never said it was loading; a drop-down that read its value twice; and a timestamp on an unconfirmed row dimmed below readable contrast (<https://github.com/quiltdata/quilt-rs/pull/936>)
  - A signed-in row in **Accounts** arrives saying "Checking role…", and a screen reader is now told when the answer lands. The sub-line settled silently, leaving a reader on the provisional text with no way to learn what it became (<https://github.com/quiltdata/quilt-rs/pull/959>)
  - The countdown ring beside a sync toggle no longer announces itself to a screen reader as a progress bar of unknown progress — a value it cannot honestly report. The toggle's own label already says the interval (<https://github.com/quiltdata/quilt-rs/pull/950>)
  - Launching straight onto the new page with a dark desktop no longer flashes a white window first (<https://github.com/quiltdata/quilt-rs/pull/951>)
  - A banner's close button and the search box's clear button now draw one shared mark, where each carried its own drawing of it at its own inset. Both render at the size they always have (<https://github.com/quiltdata/quilt-rs/pull/952>)
- Under the hood, the component gallery is now arranged in tiers and holds the components, the split button, the header scene and the context pane that the installed-package page will be assembled from, each with tests of its own (<https://github.com/quiltdata/quilt-rs/pull/938>, <https://github.com/quiltdata/quilt-rs/pull/947>, <https://github.com/quiltdata/quilt-rs/pull/955>, <https://github.com/quiltdata/quilt-rs/pull/958>)

### quilt-rs

- Updated [from v0.39.0 to v0.39.1](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.39.0...quilt-rs/v0.39.1) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

### quilt-uri

- Updated [from v0.4.0 to v0.4.1](https://github.com/quiltdata/quilt-rs/compare/quilt-uri/v0.4.0...quilt-uri/v0.4.1) (see [quilt-uri/CHANGELOG.md](../quilt-uri/CHANGELOG.md))

## [v0.22.2] - 2026-09-15

### Fixed

- A pull that is interrupted — a dropped connection, an expired credential, a quit, power loss — can now be retried. It used to leave the package stuck for good: the files being updated were deleted before they were fetched again, so an interruption lost them from your folder and from tracking at once, and every retry read that gap as a conflict between your delete and the remote's change and refused. A pull now downloads and stages the whole update before it writes anything into your folder, so an interruption before the writes — which is where nearly all of the time goes — leaves the folder untouched and the retry is an ordinary update (<https://github.com/quiltdata/quilt-rs/pull/921>)
- A pull no longer disturbs a program that has a package file open. Each file is now written aside and moved into place, so a reader sees one revision's bytes or the other's and never a half-written file; a program holding the file memory-mapped, which is ordinary for HDF5, Zarr, Arrow and `numpy` `mmap_mode`, used to be killed outright by a pull arriving mid-analysis (<https://github.com/quiltdata/quilt-rs/pull/921>)
- A pull no longer reports a conflict on a file you have edited to exactly what the incoming revision holds. The two were compared by hash without accounting for the algorithm each was computed in, so byte-identical content could compare as different and block a pull that had nothing to resolve (<https://github.com/quiltdata/quilt-rs/pull/921>)

### Changed

- The new main page (Settings → Experimental → **New main page**, still off by default) is polished throughout. The current page is unchanged by all of it:
  - Each row in **Needs your attention** is now a link to the page that fixes it, and says what is wrong as a sentence: `org/dataset-c has conflicts in 2 files`, with `Publish` on the right. The buttons are gone — every one of them only opened another page — and the coloured chip that named the state is now a mark on the row's edge, since the sentence already says what the chip did (<https://github.com/quiltdata/quilt-rs/pull/940>)
  - A paused package now says why it paused, under its row in **Needs your attention**; it used to say only `Sync paused` (<https://github.com/quiltdata/quilt-rs/pull/933>)
  - Refresh shows that it is working, and takes no second press until it is done (<https://github.com/quiltdata/quilt-rs/pull/929>)
  - Anything cut short to fit — a package name, a file path, a host, a group heading — shows its full value on hover (<https://github.com/quiltdata/quilt-rs/pull/930>)
  - Buttons, drop-downs, the search box and the view switch share one height, where each used to be its own size (<https://github.com/quiltdata/quilt-rs/pull/912>)
  - The primary button, the ticked checkboxes and the countdown ring use the top bar's navy instead of a second, nearly identical one (<https://github.com/quiltdata/quilt-rs/pull/916>)
  - Refresh and Settings show their names beside their icons, as the current page's toolbar does (<https://github.com/quiltdata/quilt-rs/pull/919>)
  - The Settings icon is a gear that reads as one; the old drawing looked like a sun at that size (<https://github.com/quiltdata/quilt-rs/pull/920>)
  - Linux uses the desktop's own font instead of Roboto, and no longer flashes from one to the other while it loads (<https://github.com/quiltdata/quilt-rs/pull/918>)
  - Native drop-down popups and scrollbars follow the light or dark theme; the current page stays light whatever the system is set to (<https://github.com/quiltdata/quilt-rs/pull/927>, <https://github.com/quiltdata/quilt-rs/pull/934>)
  - Text fields, the search box, drop-downs and the view switch have a clearer edge, so an empty field is visible as a field on a white card (<https://github.com/quiltdata/quilt-rs/pull/924>)
  - Keyboard focus is visible on every control: the focus ring clears the 3:1 contrast WCAG asks of it, in both themes (<https://github.com/quiltdata/quilt-rs/pull/913>)
  - A file's age is no longer cut short to "23 hours a…", and anything older than a year reads "1 year ago" rather than a growing count of months (<https://github.com/quiltdata/quilt-rs/pull/914>)
  - The close button on a banner and the clear button in the search box are 24px squares, the minimum the other controls already met (<https://github.com/quiltdata/quilt-rs/pull/916>)
  - A banner's message and a toggle's sublabel wrap at a readable measure instead of running the width of the window (<https://github.com/quiltdata/quilt-rs/pull/931>)
- Under the hood, the components the new main page is built from are now recorded as a design system and browsable on their own, which is the groundwork the page is assembled from (<https://github.com/quiltdata/quilt-rs/pull/915>, <https://github.com/quiltdata/quilt-rs/pull/917>, <https://github.com/quiltdata/quilt-rs/pull/925>, <https://github.com/quiltdata/quilt-rs/pull/926>, <https://github.com/quiltdata/quilt-rs/pull/932>)

### Security

- QuiltSync takes rustls 0.23.45 for [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285), where TLS 1.3 handshake messages were accepted across encryption level boundaries. It arrives transitively through the AWS SDK's TLS stack; nothing in QuiltSync selects it (<https://github.com/quiltdata/quilt-rs/pull/923>)

### quilt-rs

- Updated [from v0.38.0 to v0.39.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.38.0...quilt-rs/v0.39.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.22.1] - 2026-09-11

### Added

- A redesigned main page, off by default and switched on under Settings → Experimental → **New main page**. It opens with what needs you: packages that are out of date, hold unpublished changes, or whose state could not be confirmed, each beside the one thing to do about it — and where a whole catalog or bucket is the cause, it is said once with a count instead of on every package. When nothing needs you, it says so in one line. Below are your packages with each one's state and when it last changed, searchable and groupable, or switched to a feed of recently changed files. Alongside: whether background syncing is running and when it will next run, switchable per direction, and the catalogs you are signed into with the role each is using. Switching it off returns you to the current page and loses nothing (<https://github.com/quiltdata/quilt-rs/pull/881>)
- QuiltSync's window now has a minimum size of 1024×560. Below that the main page's regions cannot all fit, and the list was crushed to nothing rather than the window refusing to shrink (<https://github.com/quiltdata/quilt-rs/pull/881>)
- The new main page follows your system's light or dark appearance. The current pages keep their own look whichever appearance your system uses (<https://github.com/quiltdata/quilt-rs/pull/881>, <https://github.com/quiltdata/quilt-rs/pull/911>)
- QuiltSync now asks before you quit it while a package is being written. Quitting used to interrupt the write silently, leaving that package part-way between two revisions with nothing running to finish it; you now get the choice to wait or go ahead. Nothing is asked when no package is being written (<https://github.com/quiltdata/quilt-rs/pull/905>)

### Fixed

- A bucket your role cannot read is now recognised as a refusal however S3 phrases it, not only when it answers `AccessDenied`. A refusal that named any other reason — an object encrypted with a key your role cannot use, an account-level block — was reported as an ordinary failure, which reads as "sign in again"; signing in hands back the same role that was refused, so there was no way out of it (<https://github.com/quiltdata/quilt-rs/pull/881>)
- The package list no longer stalls on a deployment that answers slowly. Marking a row as refused asks which role you are using, and that question had no time limit of its own: a deployment that replied to the first request and then went quiet held the main screen empty for as long as the network allowed. The row now appears as refused without naming the role (<https://github.com/quiltdata/quilt-rs/pull/881>)
- The message shown when a package cannot be created no longer begins `Quilt error:` before the sentence that tells you what went wrong (<https://github.com/quiltdata/quilt-rs/pull/881>)
- A package you are pulling no longer briefly reports conflicts in files that are simply being downloaded for it, and no longer stops syncing in the background as a result. Checking a package while it was being written compared the files already written against the revision they came from, and read every one of them as changed on both sides (<https://github.com/quiltdata/quilt-rs/pull/907>)
- QuiltSync no longer works continuously while you are not using it. Checking a package reads every file in it, and that reading looked to the folder watcher like you had changed something, so each check scheduled another one. Left idle with eleven installed packages, the watcher asked to re-read a package about 240 times a minute where the sync interval calls for 14 — constant CPU and disk, and 21 times the log volume, for an app that sits in the background all day (<https://github.com/quiltdata/quilt-rs/pull/886>)
- Files you have excluded in `.quiltignore` no longer cause a package to be re-read. A tool churning a cache directory inside a package cost a full read of that package, which then skipped the very directory it had been woken for (<https://github.com/quiltdata/quilt-rs/pull/886>)
- A host QuiltSync cannot get credentials for at all — rather than one whose credentials S3 rejects (v0.21.2) — no longer fails with a six-line Rust error chain. It now reports as the dead session it is, so the sign-in affordance appears and background sync stops retrying it in silence (<https://github.com/quiltdata/quilt-rs/pull/867>)
- A host that is not set up as a Quilt deployment — no registry in its `config.json` — no longer sends you to the sign-in screen, where nothing you can do would help. It says what is wrong and that an administrator needs to fix it (<https://github.com/quiltdata/quilt-rs/pull/867>)
- The Commit page stays open when you are signed out, instead of bouncing you to the sign-in screen and discarding the message and metadata you had typed. It already worked this way for a role that cannot read the bucket; a dead session now gets the same treatment, and the Commit action is what refuses, naming the deployment to sign in to (<https://github.com/quiltdata/quilt-rs/pull/867>)

### Changed

- Opening QuiltSync, and returning to the package list from a package, now takes a moment to check which main page you have switched on. It used to go straight there. Nothing is lost by it and the check falls back to the current page if it fails, but there is a brief spinner where there was none (<https://github.com/quiltdata/quilt-rs/pull/881>)
- While you are actively saving files, the packages list can now be up to one sync interval behind. A save brings the next check forward rather than adding one, so a burst of edits no longer means a burst of re-reads. Opening or moving to a page still reads fresh, and nothing is published until the folder has been quiet for the configured window either way (<https://github.com/quiltdata/quilt-rs/pull/886>)

### quilt-rs

- Updated [from v0.37.0 to v0.38.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.37.0...quilt-rs/v0.38.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.22.0] - 2026-09-08

### Added

- When a new revision arrives, a notification names the files it brought — new, updated, removed — where a background pull used to change your folder in silence. Notifications stack, each closes on its own button, and one raised while the window was shut is waiting when you open it. Under individual-file sync a revision's new files are listed rather than downloaded, and the notification says so
  - <https://github.com/quiltdata/quilt-rs/pull/897>
  - <https://github.com/quiltdata/quilt-rs/pull/898>
- Before you pull, the packages list and an outdated package's banner name the files the revision would bring (<https://github.com/quiltdata/quilt-rs/pull/898>)

### Changed

- A manual Pull is confirmed by that report instead of "Successfully pulled package …", which said the same thing twice over. A pull with nothing to report — a revision that moved no files — still shows the line (<https://github.com/quiltdata/quilt-rs/pull/898>)

### Fixed

- *Save diagnostics* now includes the end of the log. The last lines of a session were dropped when the app quit — the ones covering whatever you were doing when the problem started — so the archive you send support stopped just short of the part that explains it. A crash or a force-quit still loses them (<https://github.com/quiltdata/quilt-rs/pull/883>)

### quilt-rs

- Updated [from v0.36.0 to v0.37.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.36.0...quilt-rs/v0.37.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.21.2] - 2026-09-02

### Fixed

- A credential S3 refuses is no longer reported as raw AWS error text: where the response names the cause, it is told apart from a role denial, and where the bucket was reached with your own `~/.aws` credentials it says to update that file rather than offering a sign-in that cannot help. Two paths do not benefit yet — an installed package's page and the packages list keep showing last-known state, and a push reports the rejection as a role denial (<https://github.com/quiltdata/quilt-rs/pull/861>)
- Background sync tells a refused credential from a network blip instead of retrying it quietly, and counts one episode per deployment rather than one per package. What you see is unchanged: the affordance still renders per package, and there is no notification (<https://github.com/quiltdata/quilt-rs/pull/861>)
- A missing package or object now reads "Package not found: the requested version or object does not exist" instead of the AWS error text (<https://github.com/quiltdata/quilt-rs/pull/861>)

### quilt-rs

- Updated [from v0.35.0 to v0.36.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.35.0...quilt-rs/v0.36.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.21.1] - 2026-08-07

### Fixed

- The packages list stopped re-checking the remote for a package that had not changed. A package showing "behind" made a network round trip roughly once a second — for as long as its row was on screen — because the background watcher re-reports a package's status whether or not anything moved, and the row acted on every report instead of only the ones that differed (<https://github.com/quiltdata/quilt-rs/pull/834>)

### Added

- Experimental, off by default: Settings → Experimental → "Enable entire-package sync" adds a per-package choice on the installed-package page — sync individual files, or sync the entire package. Under whole-package sync a pull also downloads files a teammate added, instead of listing them and leaving them; switching it on doesn't move any bytes by itself — the toolbar offers "Download all files" for what's already listed — and switching it off stops fetching more without removing anything (<https://github.com/quiltdata/quilt-rs/pull/834>)

## [v0.21.0] - 2026-08-06

### Added

- *Save diagnostics* now includes the app's logs. The log file was filtered to errors only, so the archive you send support was effectively empty (<https://github.com/quiltdata/quilt-rs/pull/828>)

### Changed

- Actions no longer wait on telemetry: reporting ran inside the command you triggered, with no timeout, so a slow connection delayed your click (<https://github.com/quiltdata/quilt-rs/pull/826>)
- A problem you or your admin can fix — a package that fails a workflow, a role without access to a bucket, a cancelled dialog — no longer files a crash report (<https://github.com/quiltdata/quilt-rs/pull/830>)
- A crash in the app window is now reported instead of being visible only in the browser console. The window still goes blank (<https://github.com/quiltdata/quilt-rs/pull/832>)
- Crash reports include stack traces and the deployment they happened against, so a report is actionable rather than just a count (<https://github.com/quiltdata/quilt-rs/pull/831>)
- Background autosync reports what it did, including stopping because a session expired — previously it worked in silence (<https://github.com/quiltdata/quilt-rs/pull/824>)
- Two new files in the app data directory: `install_id`, a random identifier derived from nothing about you (delete it to become a new install), and `unsent_events.jsonl`, holding events that could not be sent while offline (<https://github.com/quiltdata/quilt-rs/pull/822>, <https://github.com/quiltdata/quilt-rs/pull/833>)

### quilt-rs

- Log levels rearranged and long file-list dumps replaced with summaries: a debug-level log is roughly 100× smaller (<https://github.com/quiltdata/quilt-rs/pull/828>)

## [v0.20.1] - 2026-08-03

### Changed

- The installed-package page updates in place when the package changes instead of blanking to a spinner; opening a different package still shows one (<https://github.com/quiltdata/quilt-rs/pull/814>)

### Fixed

- The installed-package page no longer reloads on a timer: with autosync on it used to refetch every 30 s whether or not anything had changed, flashing a spinner and clearing the file selection to redraw the view already on screen. It now refreshes only when the package's state actually changes (<https://github.com/quiltdata/quilt-rs/pull/813>)
- Files ticked for download on the installed-package page are no longer cleared when the package changes underneath you; the app bar's Refresh still starts clean (<https://github.com/quiltdata/quilt-rs/pull/814>)
- "Select all" shows a dash when only some files are ticked, instead of looking empty (<https://github.com/quiltdata/quilt-rs/pull/814>)

## [v0.20.0] - 2026-07-30

### Added

- Role switcher: Settings → Auth shows your active role per host and lets you change it (only when you hold more than one), taking effect on the next read or write instead of waiting for the previous role's credentials to expire — including a role you switched in the web catalog (<https://github.com/quiltdata/quilt-rs/pull/807>)

### Changed

- Pull now preserves non-conflicting local work instead of refusing on any local change: the Pull button shows up front whether pulling is safe or which files conflict (with commit → merge as the way out), and autosync pulls behind packages while keeping local edits, pausing with conflict guidance only on a real conflict (<https://github.com/quiltdata/quilt-rs/pull/800>)
- A bucket your active role cannot reach now says so wherever it surfaces — naming the role and offering to switch it, rather than showing a raw storage error or asking you to sign in again; commit is disabled up front instead of failing halfway, and autosync pauses the package until you switch role instead of retrying forever
  - <https://github.com/quiltdata/quilt-rs/pull/807>
  - <https://github.com/quiltdata/quilt-rs/pull/808>
- QuiltSync shows a revision's commit message instead of its top-hash on the version-mismatch banner (full hash on hover) (<https://github.com/quiltdata/quilt-rs/pull/781>)
- Usage analytics and crash reports now record which Quilt deployment an action concerned, so activity can be read per stack, and opening a folder is reported as three distinct actions (a package's directory, the sync home, the app data directory) instead of one event that could not tell them apart; actions that concern no deployment record none rather than an inherited one (<https://github.com/quiltdata/quilt-rs/pull/811>)

### quilt-rs

- Updated [from v0.33.0 to v0.34.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.33.0...quilt-rs/v0.34.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

### quilt-uri

- Updated [from v0.3.0 to v0.4.0](https://github.com/quiltdata/quilt-rs/compare/quilt-uri/v0.3.0...quilt-uri/v0.4.0) (see [quilt-uri/CHANGELOG.md](../quilt-uri/CHANGELOG.md))

## [v0.19.0] - 2026-07-14

### Added

- Workflow selection in the commit dialog: the bucket's configured workflows appear in a catalog-parity dropdown (by name) instead of a free-text id field, with "Open in catalog" links to the workflow config and the selected workflow's schemas; the dialog validates the message, metadata, and package name against the selected workflow as you type (advisory inline errors); with no workflow chosen it applies the bucket's default (the per-user "Default workflow" setting acting as an override); and a warning-coloured note flags when that default-wins preselection differs from the previous revision's workflow (<https://github.com/quiltdata/quilt-rs/pull/743>, <https://github.com/quiltdata/quilt-rs/pull/746>, <https://github.com/quiltdata/quilt-rs/pull/756>, <https://github.com/quiltdata/quilt-rs/pull/758>, <https://github.com/quiltdata/quilt-rs/pull/770>)
- Workflow selection in the Set-remote dialog: pick the workflow for a package's first push, with a warning when the bucket default can't be resolved, a malformed bucket config distinguished from a transient load failure, and a note that saving creates a new revision (<https://github.com/quiltdata/quilt-rs/pull/748>, <https://github.com/quiltdata/quilt-rs/pull/755>, <https://github.com/quiltdata/quilt-rs/pull/767>)

### Changed

- Committing, publishing, or setting a remote for a package that fails its bucket's workflow is refused with the reason — in the commit and Set-remote dialogs and in notifications — and autosync pauses the namespace as a conflict instead of retrying (<https://github.com/quiltdata/quilt-rs/pull/753>)
- The installed packages list marks packages that need attention — a remote error or an autosync-paused workflow conflict — in red with an inline reason (<https://github.com/quiltdata/quilt-rs/pull/764>)
- Settings lists Autosync before Commit and Push, and the "Edit commit defaults" popup warns that these settings apply to every bucket when a workflow override or default metadata is set (<https://github.com/quiltdata/quilt-rs/pull/759>)

### Fixed

- Logging out now immediately drops the in-memory S3 credential cache, so reads and writes stop working right away instead of lingering until the cached credentials expire (<https://github.com/quiltdata/quilt-rs/pull/766>)
- Committing or publishing with an empty metadata field now preserves the package's existing metadata instead of silently dropping it (<https://github.com/quiltdata/quilt-rs/pull/734>)

### quilt-rs

- Updated [from v0.32.0 to v0.33.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.32.0...quilt-rs/v0.33.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.18.2] - 2026-05-25

### Added

- Tray icon with status indicator and `Close to tray` setting in Settings → Autosync — keeps autosync running on the closed-window cadence when enabled (default off) (<https://github.com/quiltdata/quilt-rs/pull/693>)

## [v0.18.1] - 2026-05-21

### Changed

- Autosync: pull cadence and push quiet window are now independent — the popup shows two inputs (**Pull interval** and **Wait after last edit before publishing**), and autopush now waits a constant 5 min after the last edit (new key `idle_timeout_secs`, default 300 s) instead of the previous 30 s / 120 s window-dependent wait (<https://github.com/quiltdata/quilt-rs/pull/690>)

## [v0.18.0] - 2026-05-19

### Added

- Autosync: a unified background loop (formerly "Autopull") that pulls remote-tracking packages and, once the working tree has been quiet for a tick, commits and pushes mapped packages with local changes. Settings expose pull and push as independent checkboxes (existing `autopull_settings.json` migrates to `autosync_settings.json` with `push_enabled: false`, so adopters do not silently opt into autopush). An optional filesystem watcher refreshes local status on disk changes. Workflow / push failures pause the namespace and emit a new `autosync-paused` Tauri event — the list shows a toast for unexpected errors and the detail page shows a persistent banner, re-hydrated on navigation via the new `get_autosync_snapshot` command (<https://github.com/quiltdata/quilt-rs/pull/675>, <https://github.com/quiltdata/quilt-rs/pull/676>, <https://github.com/quiltdata/quilt-rs/pull/682>)

### Changed

- Renamed the `Diverged`-state entry button from "Merge" to "Resolve conflict", relabeled the on-page actions to "Promote my revision" / "Overwrite local with remote", and clarified in the intro that Quilt does not merge file contents — pick which side's revision wins; the other is discarded (<https://github.com/quiltdata/quilt-rs/pull/677>)
- Rewrote the detail-page "behind" banner: "The remote has newer revisions" (was "Your commits are behind the remote", which asserted commits the user might not have), and inlined the "commit or discard your local changes to pull" action when working-tree changes block the pull (<https://github.com/quiltdata/quilt-rs/pull/682>)

### quilt-rs

- Updated [from v0.31.1 to v0.32.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.31.1...quilt-rs/v0.32.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.17.1] - 2026-04-29

### Added

- Edit or view a package's remote (host + bucket) from the toolbar before push, with bucket validated at save time; UI standardized on "remote" terminology (<https://github.com/quiltdata/quilt-rs/pull/640>)

### Changed

- Catalog HTTPS links are now formatted on demand by the UI instead of pre-built by the backend (<https://github.com/quiltdata/quilt-rs/pull/642>)
- Migrated to the Rust 2024 edition; building from source now requires Rust 1.85+ (<https://github.com/quiltdata/quilt-rs/pull/646>)

### Fixed

- Clicking outside a notification now dismisses it (<https://github.com/quiltdata/quilt-rs/pull/636>)

### quilt-rs

- Updated [from v0.30.0 to v0.30.1](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.30.0...quilt-rs/v0.30.1) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

### quilt-uri

- Added v0.1.0, shared with the Leptos UI (see [quilt-uri/CHANGELOG.md](../quilt-uri/CHANGELOG.md))

## [v0.17.0] - 2026-04-22

### Added

- New `[Commit and Push]` action that commits local changes (if any) and pushes in a single step — available as a one-click button on the Installed Packages List, as a primary CTA on the Commit form alongside the existing `[Commit]`, and on the Installed Package page's bottom action bar alongside `[Create new revision]` (<https://github.com/quiltdata/quilt-rs/pull/634>)
- Commit and Push defaults under Settings: message template (supports `{date}`/`{time}`/`{datetime}`/`{namespace}`/`{changes}` placeholders with a live preview), default workflow picker, and default metadata (<https://github.com/quiltdata/quilt-rs/pull/634>)

### quilt-rs

- Updated [from v0.29.0 to v0.30.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.29.0...quilt-rs/v0.30.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - New `flow::publish_package` that composes commit + push in a single call
  - Error enum split into focused domain enums
  - Automatic retry with exponential backoff and request timeouts for HTTP calls; `ExpiredToken` S3 errors fixed via per-request credential refresh with single-flight per-host deduplication

## [v0.16.0] - 2026-04-16

### Changed

- Replace Askama templates, TypeScript, and npm with a Leptos WASM client-side rendered frontend (<https://github.com/quiltdata/quilt-rs/pull/606>)
- Reorganize button components into a `buttons` submodule with `ButtonKind` enum, `IconButton` and `ButtonCta` base components, and specific button components for every UI button (<https://github.com/quiltdata/quilt-rs/pull/613>)
- Update logo to Quilt.bio branding (<https://github.com/quiltdata/quilt-rs/pull/612>)
- Disable Commit button on commit page when message is empty (<https://github.com/quiltdata/quilt-rs/pull/619>)
- Highlight Commit link as primary on packages list when package has uncommitted changes (<https://github.com/quiltdata/quilt-rs/pull/619>)
- Disable Pull button with popover when package has uncommitted local changes (<https://github.com/quiltdata/quilt-rs/pull/619>)
- Extract popover CSS into shared `.qui-popover` component (<https://github.com/quiltdata/quilt-rs/pull/619>)
- Render packages list instantly from cached lineage, then refresh status per-package in background (<https://github.com/quiltdata/quilt-rs/pull/622>)

### Fixed

- Fix updater log spam by prioritizing HubSpot endpoint

### quilt-rs

- Updated [from v0.28.1 to v0.29.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.28.1...quilt-rs/v0.29.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Always certify latest on first push, fixing incorrect "Behind" status after pushing a local package with the same name as an existing remote package

## [v0.15.1] - 2026-04-08

### Changed

- Show a notification instead of an error when the installed package version differs from the requested one or is local-only (<https://github.com/quiltdata/quilt-rs/pull/605>)

## [v0.15.0] - 2026-04-07

### Added

- Add `.quiltignore` affordance: junk file detection badges, ignore/un-ignore popups, and server-side entry filtering via URL fragment parameters (<https://github.com/quiltdata/quilt-rs/pull/593>)
- Add create local package UI with optional source directory picker and set remote popup for first-push workflow (<https://github.com/quiltdata/quilt-rs/pull/596>)

### Changed

- Show release notes in an in-app popup instead of linking to private GitHub releases page (<https://github.com/quiltdata/quilt-rs/pull/603>)

### quilt-rs

- Updated [from v0.27.4 to v0.28.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.27.4...quilt-rs/v0.28.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - `.quiltignore` support, junk file detection
  - Local-only package creation and first-push workflow
  - `PackageLineage.remote` is now optional

## [v0.14.5] - 2026-03-25

### Added

- Collect diagnostic logs from the Settings page and send them via Sentry (with zip attachment) or email (<https://github.com/quiltdata/quilt-rs/pull/581>, <https://github.com/quiltdata/quilt-rs/pull/583>)

### Changed

- Replace debug toolbar with a dedicated Settings page accessible from the app bar (<https://github.com/quiltdata/quilt-rs/pull/581>)
- Simplify app initialization: remove `Globals` struct and `AppAssets` trait, use fallible `App::create()` (<https://github.com/quiltdata/quilt-rs/pull/581>)
- Login from a package error state now redirects back to that package instead of the packages list (<https://github.com/quiltdata/quilt-rs/pull/585>)

## [v0.14.4] - 2026-03-19

### Added

- Browser-based OAuth 2.1 login via `quilt://` deep link callback with code-based login as a fallback for stacks that do not support OAuth (<https://github.com/quiltdata/quilt-rs/pull/539>)
- Add `flow` field (`"oauth"` or `"legacy"`) to the `UserLoggedIn` telemetry event to track which login path was used (<https://github.com/quiltdata/quilt-rs/pull/562>)

### Fixed

- Reject unsolicited `quilt://auth/callback` deep links with a clear error instead of falling back to the legacy code-based login (<https://github.com/quiltdata/quilt-rs/pull/570>)
- Show a proper error page instead of an infinite spinner when OAuth login fails, reusing the generic error page with a "Login failed" title (<https://github.com/quiltdata/quilt-rs/pull/569>)
- Fix silent navigation failure after OAuth login: `navigate_after_login` now accepts a typed `routes::Paths` instead of a raw string; on an unexpected redirect value an `error!`-level log is emitted and the user is sent to the default page rather than being left on the login screen (<https://github.com/quiltdata/quilt-rs/pull/568>)
- Return `Err` instead of `Ok(None)` when an OAuth state entry has expired in `take_params`, preventing a timed-out callback from bypassing PKCE+CSRF verification (<https://github.com/quiltdata/quilt-rs/pull/567>)
- Evict expired OAuth state entries (TTL: 10 min) to prevent unbounded memory growth in long-running sessions (<https://github.com/quiltdata/quilt-rs/pull/558>)
- URL-encode host in OAuth redirect URI to handle special characters correctly (<https://github.com/quiltdata/quilt-rs/pull/560>)

### Changed

- Flatten `navigate_after_login` from four nested `match` blocks to early-return style; missing main window now returns `Err(Error::Window)` instead of silently succeeding (<https://github.com/quiltdata/quilt-rs/pull/564>)

### quilt-rs

- Updated [from v0.27.2 to v0.27.4](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.27.2...quilt-rs/v0.27.4) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - OAuth 2.1 Authorization Code flow with PKCE and Dynamic Client Registration
  - Redact secrets (`access_token`, `refresh_token`, etc.) from debug logs via custom `Debug` impls
  - `Auth` now holds `Arc<S>` instead of `S`, removing the `Clone` bound on `Storage`
  - Replace `read_file`/`write_file` in `Storage` with `read_byte_stream`/`write_byte_stream`; writes are now atomic (temp file + rename)

## [v0.14.3] - 2026-03-03

### Fixed

- Replace `window.prompt` with inline form for setting catalog origin,
  fixing broken prompt on macOS in Tauri
  (<https://github.com/quiltdata/quilt-rs/pull/529>)

## [v0.14.2] - 2026-03-03

### Changed

- Show update notification with Download/Dismiss buttons
  instead of auto-installing updates
  (<https://github.com/quiltdata/quilt-rs/pull/520>)
- Gracefully handle packages without catalog origin: show "Set origin" button
  instead of failing, remove bogus open.quilt.bio fallback
  (<https://github.com/quiltdata/quilt-rs/pull/523>)

### quilt-rs

- Updated [from v0.27.1 to v0.27.2](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.27.1...quilt-rs/v0.27.2)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Add `UpstreamState::Error` variant and
    `InstalledPackage::set_origin()` for packages without catalog origin

## [v0.14.1] - 2026-02-26

### Changed

- Upgrade Azure code signing action to v1 (Artifact Signing) (<https://github.com/quiltdata/quilt-rs/pull/513>)

## [v0.14.0] - 2026-02-25

### Added

- Add Windows code signing for release installers (<https://github.com/quiltdata/quilt-rs/pull/484>)

## [v0.13.2] - 2026-02-25

### Added

- Commit page now pre-fills the message field with an
  auto-generated summary of changed files (<https://github.com/quiltdata/quilt-rs/pull/504>)

## [v0.13.1] - 2026-02-19

### Fixed

- Fixed deep link handler failing on macOS/Linux
  due to `tauri://` scheme not matching `http` check (<https://github.com/quiltdata/quilt-rs/pull/491>)

### quilt-rs

- Updated [from v0.27.0 to v0.27.1](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.27.0...quilt-rs/v0.27.1)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Fixed stale Parquet manifest cache preventing app startup (<https://github.com/quiltdata/quilt-rs/pull/492>)

## [v0.13.0]

### Changed

- Updated to use quilt-rs v0.27.0 with JSONL manifest format
  migration (<https://github.com/quiltdata/quilt-rs/pull/476>)

### quilt-rs

- Updated [from v0.26.0 to v0.27.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.26.0...quilt-rs/v0.27.0)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Migrated manifest format from Parquet to JSONL for improved performance
    and compatibility

## [v0.12.0]

### Fixed

- Fixed redirect after package pull to avoid
  'package already installed' error (<https://github.com/quiltdata/quilt-rs/pull/459>)

### quilt-rs

- Updated [from v0.25.0 to v0.26.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.25.0...quilt-rs/v0.26.0)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Fixed commit logic to respect crc64Checksums configuration from host config

## [v0.11.2]

### Fixed

- Fixed Windows deep link navigation issue when app is launched
  via deep link (<https://github.com/quiltdata/quilt-rs/pull/455>)

## [v0.11.1]

### Changed

- Bumped patch release version to test auto-updater
  functionality (<https://github.com/quiltdata/quilt-rs/pull/454>)
- Minor dependency updates

## [v0.11.0]

### Added

- Added auto-updater functionality for seamless application updates (<https://github.com/quiltdata/quilt-rs/pull/447>)

## [v0.10.0]

This version increment consolidates many small changes from previous patch releases.

### Changed

- Auto-select all checkboxes on page load for file installation (<https://github.com/quiltdata/quilt-rs/pull/437>)
- Add autofocus to commit message input field (<https://github.com/quiltdata/quilt-rs/pull/436>)

### Fixed

- Fixed handling of macOS deep links on first application start (<https://github.com/quiltdata/quilt-rs/pull/433>)

## [v0.9.9](https://github.com/quiltdata/quilt-rs/releases/tag/QuiltSync/v0.9.9) - 2025-01-07

### Fixed

- Handle S3 package URIs with tags that don't have explicit hashes (<https://github.com/quiltdata/quilt-rs/pull/429>)

### quilt-rs

- Updated from [v0.24.0 to v0.25.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.24.0...quilt-rs/v0.25.0)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Support for timestamp tags in package URIs
  - Export `Tag` enum and `LATEST_TAG` constant

## [v0.9.8](https://github.com/quiltdata/quilt-rs/releases/tag/QuiltSync/v0.9.8) - 2025-12-30

### Added

- Mixpanel analytics tracking:
  - <https://github.com/quiltdata/QuiltSync/pull/363>
  - <https://github.com/quiltdata/QuiltSync/pull/366>
- More verbose debug logs for failed post-login redirects
  (<https://github.com/quiltdata/QuiltSync/pull/372>)

### Fixed

- Deep link navigation when app hasn't started yet
  (<https://github.com/quiltdata/QuiltSync/pull/372>)
- macOS startup deep link handler - now properly handles deep links on app
  launch (<https://github.com/quiltdata/QuiltSync/pull/394>)
- Better detailed error handling for route parsing
  (<https://github.com/quiltdata/QuiltSync/pull/392>)

### quilt-rs

- Updated from v0.21.1 to [v0.24.0](https://github.com/quiltdata/quilt-rs/releases/tag/quilt-rs%2Fv0.24.0)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Fixed `quilt+s3://` URL parsing with `:tag` syntax
  - Fixed hash mismatch for packages with diacritic characters
  - Support for reading manifest with CRC64/NVMe checksums
  - Support for creating packages with CRC64/NVMe object hashes

### Changed

- Updated macOS build target from macos-13 to macos-15
  (<https://github.com/quiltdata/QuiltSync/pull/393>)
- Can now pass `HostConfig` argument to commands, requesting specific checksums
  (crc64 or sha256)
- Updated GitHub Actions workflows (<https://github.com/quiltdata/QuiltSync/pull/370>)
- Minor dependency updates:
  - <https://github.com/quiltdata/QuiltSync/pull/368>
  - <https://github.com/quiltdata/QuiltSync/pull/373>
  - <https://github.com/quiltdata/QuiltSync/pull/364>
  - <https://github.com/quiltdata/QuiltSync/pull/365>

## [v0.9.7](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.7) - 2025-11-11

- Add Quilt+S3 URI resolver in the header
- Integrate Sentry error tracker

## [v0.9.6](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.6) - 2025-11-03

- Improve error handling
- Make writing logs more robust
- Minor updates of dependencies

## [v0.9.5](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.5) - 2025-10-22

- Update quilt-rs
- Minor updates of dependencies
- Make metadata editor UI more consistent with the other input fields

## [v0.9.4](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.4) - 2025-04-14

### Fixed

- Show progressbar, make it consistent with the theme
- Reload page after downloading files
- Fixed `parcel` build watcher

## [v0.9.3](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.3) - 2025-04-10

### Changed

- Migrate from `maud` to `askama` for templating

## [v0.9.2](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.2) - 2025-04-02

### Added

- Login button on S3 or Auth error

## [v0.9.1](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.1) - 2025-04-01

- Minor improvements in Metadata editor

## [v0.9.0](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.0) - 2025-04-01

- Mark the minor version change as it contains many fixes in previous patches

### Changed

- Show buttons "Add object"/"Add array"
- Make metadata editor styles consistent with the rest of the app

## [v0.8.6](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.6) - 2025-03-31

### Fixed

- quilt_rs: sort keys recursively in metadata

### Changed

- Log files end with `.log` extension
- Don't pre-set commited message

## [v0.8.5](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.5) - 2025-03-31

- Minor UI improvements
- quilt_rs: Fix serialization/deserialization of the workflow:
  - metadata schema is optional
  - metadata id can be different from workflow id

## [v0.8.4](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.4) - 2025-03-27

### Fixed

- quilt_rs: Fix sorting files entries in manifest
- Fix checkbox handler for installing specific files
- Pre-set home directory

## [v0.8.3](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.3) - 2025-03-26

### Fixed

- quilt_rs: Fix uploading modified files during push

### Changed

- Don't open empty package folder if no files were installed

## [v0.8.2](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.2) - 2025-03-25

### Breaking

- Set Home directory for mutable files first time application starts

### Added

- Open the file with default application if deep link has path

### Changed

- On deep link redirect to installed package if it exists
- Refactor UI: make it wider, move primary buttons to the bottom, add metadata editor

### Fixed

- Update quilt_rs with fixes for hashing `{ "message": null, "user_meta": null }`

## [v0.8.1](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.1) - 2025-03-03

Update `quilt_rs` to 0.9.1

### Fixed

- Fix wrong hash calculation during caching manifest (won't affect users) via `quilt_rs`

### Added

- Add button to open `.quilt` directory on Error page

## [v0.8.0](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.0) - 2025-02-27

Consolidate the vast majority of "patch" changes into a single "minor" release.

### Fixed minor bugs

## [v0.7.10](https://github.com/quiltdata/QuiltSync/releases/tag/v0.7.10) - 2025-02-27

### Changed

- Remove `Settings` page: use `&catalog` URI parameter as a source of truth
  for the catalog origin.
- Rotate logs daily and keep last 10 files only

### Fixed

- Fix/remove redirect on failed commit
- Make "workflow" field editable during commit

### Refactored

- Simplify and straighten the JS API

[View full diff](https://github.com/quiltdata/QuiltSync/compare/v0.7.9...v0.7.10)

## [v0.7.9](https://github.com/quiltdata/QuiltSync/compare/v0.7.8...v0.7.9) (2025-02-20)

- Add authentication based on `&catalog` URI parameter
- Add logs in `~/user-data-dir/com.quiltdata.quilt-sync/logs`
- Fixed duplication of deep link handling

## [v0.7.8](https://github.com/quiltdata/QuiltSync/compare/v0.7.7...v0.7.8) (2025-02-07)

- Update `quilt_rs`
- Fix aarch builds

## [v0.7.7](https://github.com/quiltdata/QuiltSync/compare/v0.7.6...v0.7.7) (2025-01-24)

- Fix duplication of commands
- Rewrite frontend: `htmx` → `typescript`
- Fix linux builds

## [v0.7.6](https://github.com/quiltdata/QuiltSync/compare/v0.7.5...v0.7.6) (2025-01-20)

## [v0.7.5](https://github.com/quiltdata/QuiltSync/compare/v0.7.3...v0.7.5) (2025-01-02)

- Update quilt-rs with `&catalog` URI parameter support
- Fix Github release workflow

## [v0.7.3](https://github.com/quiltdata/QuiltSync/compare/v0.7.2...v0.7.3) (2024-06-20)

- Update quilt-rs
- Minor updates of dependencies

## [v0.7.2](https://github.com/quiltdata/QuiltSync/compare/v0.7.1...v0.7.2) (2024-05-13)

- Update quilt-rs
- Minor updates of dependencies

## [v0.7.1](https://github.com/quiltdata/QuiltSync/compare/v0.7.0...v0.7.1) (2024-04-25)

- Use `macos-12` for Intel builds, and `macos-latest` (14) for ARM builds

## [v0.7.0](https://github.com/quiltdata/QuiltSync/compare/v0.6.14...v0.7.0) (2024-04-25)

- Update quilt-rs (v0.7.0) with fixed checksums

## [v0.6.14](https://github.com/quiltdata/QuiltSync/compare/v0.6.13...v0.6.14) (2024-04-24)

- Update quilt-rs with major refactoring <https://github.com/quiltdata/QuiltSync/pull/123>
- Minor update of dependencies

## [v0.6.13](https://github.com/quiltdata/QuiltSync/compare/v0.6.12...v0.6.13) (2024-03-25)

- <https://github.com/quiltdata/QuiltSync/pull/116>
  - Fix JS-imports
  - Fix calculating checksums by updating quilt-rs

## [v0.6.12](https://github.com/quiltdata/QuiltSync/compare/v0.6.11...v0.6.12) (2024-03-05)

- Update minor deps
  - <https://github.com/quiltdata/QuiltSync/pull/103>
  - <https://github.com/quiltdata/QuiltSync/pull/104>
- Fix build crash by removing tauri-cli installation with `postInstall` <https://github.com/quiltdata/QuiltSync/pull/105>

## [v0.6.11](https://github.com/quiltdata/QuiltSync/compare/v0.6.10...v0.6.11) (2024-03-04)

- Update quilt_rs to 0.5.6
  - <https://github.com/quiltdata/QuiltSync/pull/99>
  - <https://github.com/quiltdata/QuiltSync/pull/101>
  - Chunksums support and fixes
- Bugfixes, improvements, and refactoring
  - <https://github.com/quiltdata/QuiltSync/pull/100>
  - <https://github.com/quiltdata/QuiltSync/pull/102>
  - Offline support

## [v0.6.10](https://github.com/quiltdata/QuiltSync/compare/v0.6.9...v0.6.10) (2024-02-24)

- <https://github.com/quiltdata/QuiltSync/pull/98> Bugfixes
  - Fix inability to select checkboxes outside viewport
  - Don't throw exception when Quilt URI path is directory
  - Improve scroll UX
  - Move Commit and Merge pages under installed package breadcrumb
  - Hide "Select all" when nothing to install

## [v0.6.9](https://github.com/quiltdata/QuiltSync/compare/v0.6.8...v0.6.9) (2024-02-23)

- <https://github.com/quiltdata/QuiltSync/pull/97> Refactor UI action buttons
- <https://github.com/quiltdata/QuiltSync/pull/96> Update `quilt-rs` with new
  S3 checksums support

## [v0.6.8](https://github.com/quiltdata/QuiltSync/compare/v0.6.7...v0.6.8) (2024-02-22)

- <https://github.com/quiltdata/QuiltSync/pull/94>
  - Show "disabled" and "in-progress" states
  - Fix scrolling entries list and improve checkbox layout
  - Use warning/error colors

## [v0.6.7](https://github.com/quiltdata/QuiltSync/compare/v0.6.6...v0.6.7) (2024-02-21)

- <https://github.com/quiltdata/QuiltSync/pull/93>
  - Use inline importmaps to please Microsoft Edge
  - Improve installing paths UX and edge cases

## [v0.6.6](https://github.com/quiltdata/QuiltSync/compare/v0.6.5...v0.6.6) (2024-02-20)

- <https://github.com/quiltdata/QuiltSync/pull/92> Select and bulk install paths
- <https://github.com/quiltdata/QuiltSync/pull/86> Show deleted files
- <https://github.com/quiltdata/QuiltSync/pull/91>
  - Hide additional actions in menu: for app toolbar, and file entry
  - Enable DevTools
- <https://github.com/quiltdata/QuiltSync/pull/90> Notarize Mac builds

## [0.6.5](https://github.com/quiltdata/QuiltSync/compare/v0.6.4...v0.6.5) (2024-02-16)

- <https://github.com/quiltdata/QuiltSync/pull/82>: Add "Open Catalog" link
  to the installed packages list page
- <https://github.com/quiltdata/QuiltSync/pull/85> Sign Mac apps

## [0.6.4](https://github.com/quiltdata/QuiltSync/compare/v0.6.3...v0.6.4) (2024-02-15)

- <https://github.com/quiltdata/QuiltSync/pull/81>:
  - Increase tests coverage
  - Bugfix: prevent submitting form on Enter

## [0.6.3](https://github.com/quiltdata/QuiltSync/compare/v0.6.2...v0.6.3) (2024-02-14)

- <https://github.com/quiltdata/QuiltSync/pull/80> Increase tests coverage,
  remove temp_dir and manage to not touch real I/O in unit tests

## [0.6.2](https://github.com/quiltdata/QuiltSync/compare/v0.6.1...v0.6.2) (2024-02-13)

- <https://github.com/quiltdata/QuiltSync/pull/70> Update crate `tokio` to 1.36.0
- <https://github.com/quiltdata/QuiltSync/pull/79> Update "Golden path" to CONTRIBUTING.md

## [0.6.1](https://github.com/quiltdata/QuiltSync/compare/v0.6.0...v0.6.1) (2024-02-13)

- <https://github.com/quiltdata/QuiltSync/pull/76>
- <https://github.com/quiltdata/QuiltSync/pull/77>
- <https://github.com/quiltdata/QuiltSync/pull/78>

## [0.6.0](https://github.com/quiltdata/QuiltSync/compare/v0.5.7...v0.6.0) (2024-02-09)

- <https://github.com/quiltdata/QuiltSync/pull/75>

## [0.5.7](https://github.com/quiltdata/QuiltSync/compare/v0.5.6...v0.5.7) (2024-02-08)

## [0.5.6](https://github.com/quiltdata/QuiltSync/compare/v0.5.4...v0.5.6) (2024-02-08)

## [0.5.4](https://github.com/quiltdata/QuiltSync/compare/v0.5.3...v0.5.4) (2024-02-02)

## [0.5.3](https://github.com/quiltdata/QuiltSync/compare/v0.5.2...v0.5.3) (2024-01-30)

## [0.5.2](https://github.com/quiltdata/QuiltSync/compare/v0.5.1...v0.5.2) (2024-01-26)

## [0.5.1](https://github.com/quiltdata/QuiltSync/compare/v0.5.0...v0.5.1) (2024-01-23)

## [0.5.0](https://github.com/quiltdata/QuiltSync/compare/v0.4.15...v0.5.0) (2024-01-25)

## [0.4.15](https://github.com/quiltdata/QuiltSync/compare/v0.4.0...v0.4.15) (2024-02-18)

## [0.4.0] - (2023-12-21)

- Pure HTMX
- Mockup UI in HTML
- Add Styling

## [0.3.x] - (2023-12-19)

- Add Git Tag
- Fix release workflow
- Set identifier; with "-" instead of "\_"
- Update Tauri version / non-Draft release

## [0.3.0] - (2023-12-19)

- Add lib, integration tests
- Replace direct JavaScript with HTMX

## [0.2.0] - (2023-12-19)

- create-tauri-app
- Quilt icons
- Detailed README

## [0.1.0] - (2023-12-19)

- cargo init
- CHANGELOG
- GitHub Actions
- Dependent crates

### Changed

- Internal name to quilt_sync (snake_case)
