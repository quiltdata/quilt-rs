//! The small v2 pages — login, setup, error, an unknown address and the deep
//! link relay — one cell per state.
//!
//! Each region is the page's own (`LoginCard`, `SetupCard`, `ErrorSlate`)
//! over fixture props, so the routed pages and this scene cannot drift.
//! Callbacks are dropped. Each cell is a whole window at the app's smallest
//! size.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::kit::{Banner, BannerVariant, Button, PageLayout, Spinner, SpinnerVariant, icons};
use crate::pages::error_v2::{ErrorSlate, ErrorSlateSkeleton};
use crate::pages::login_v2::{LoginCard, LoginCardSkeleton, SignIn};
use crate::pages::setup_v2::{SetupCard, SetupCardSkeleton};

const HOST: &str = "open.quiltdata.com";

fn noop() -> Callback<()> {
    Callback::new(|()| ())
}

fn actions(refresh_disabled: bool) -> AnyView {
    view! {
        <Button leading_visual=icons::sync() disabled=refresh_disabled on_click=|_| ()>
            "Refresh"
        </Button>
        <Button leading_visual=icons::gear() on_click=|_| ()>
            "Settings"
        </Button>
    }
    .into_any()
}

/// A whole window at the app's smallest size.
fn window(heading: &'static str, banner: Option<AnyView>, transit: bool, body: AnyView) -> AnyView {
    view! {
        <div class="g-window" style="width:640px; --q-frame-height:560px; max-width:100%">
            <PageLayout
                heading=heading
                banner=banner.unwrap_or_else(|| ().into_any())
                actions=actions(transit)
                replace=transit
            >
                {body}
            </PageLayout>
        </div>
    }
    .into_any()
}

fn login(code: &str, running: Option<SignIn>) -> AnyView {
    view! {
        <LoginCard
            host=HOST
            catalog_url=format!("https://{HOST}/code")
            code=RwSignal::new(code.to_string())
            running=Signal::stored(running)
            on_browser=noop()
            on_open_catalog=noop()
            on_submit=noop()
        />
    }
    .into_any()
}

fn setup(directory: &str, choosing: bool, saving: bool) -> AnyView {
    view! {
        <SetupCard
            directory=RwSignal::new(directory.to_string())
            choosing=Signal::stored(choosing)
            saving=Signal::stored(saving)
            on_choose=noop()
            on_save=noop()
        />
    }
    .into_any()
}

fn critical(words: &'static str) -> AnyView {
    view! { <Banner variant=BannerVariant::Critical on_dismiss=Callback::new(|_| ())>{words}</Banner> }.into_any()
}

const NOTE: &str = "The pages a reader passes through: Log in, Set up on the first run, Error, \
    an address no route matches, and the deep link relay. Routed at /login, /setup, /error, any \
    unknown address and /remote-package under New design preview. A failed command says so in \
    the band under the appbar.";

#[component]
#[allow(clippy::too_many_lines, reason = "one cell per state, read as a list")]
pub fn SmallPagesScene() -> impl IntoView {
    view! {
        <Scene title="The small pages v2" note=NOTE>
            <Cell wide=true label="Log in — at rest, no code typed: Submit waits">
                {window("Log in", None, false, login("", None))}
            </Cell>
            <Cell wide=true label="Log in — a code typed">
                {window("Log in", None, false, login("ABC123", None))}
            </Cell>
            <Cell wide=true label="Log in — browser sign-in out">
                {window("Log in", None, false, login("", Some(SignIn::Browser)))}
            </Cell>
            <Cell wide=true label="Log in — a code that did not sign in">
                {window(
                    "Log in",
                    Some(critical("Could not log in. The code is not valid.")),
                    false,
                    login("ABC123", None),
                )}
            </Cell>
            <Cell wide=true label="Log in — loading">
                {window("Log in", None, false, view! { <LoginCardSkeleton /> }.into_any())}
            </Cell>
            <Cell wide=true label="Set up — the suggested directory">
                {window("Set up QuiltSync", None, false, setup("/Users/a-user/QuiltSync", false, false))}
            </Cell>
            <Cell wide=true label="Set up — saving">
                {window("Set up QuiltSync", None, false, setup("/Users/a-user/QuiltSync", false, true))}
            </Cell>
            <Cell wide=true label="Set up — a directory that could not be made">
                {window(
                    "Set up QuiltSync",
                    Some(critical("Could not set up the directory. Permission denied.")),
                    false,
                    setup("/Volumes/read-only/QuiltSync", false, false),
                )}
            </Cell>
            <Cell wide=true label="Set up — loading">
                {window("Set up QuiltSync", None, false, view! { <SetupCardSkeleton /> }.into_any())}
            </Cell>
            <Cell wide=true label="Error — with a host: all four ways on">
                {window(
                    "Error",
                    None,
                    false,
                    view! {
                        <ErrorSlate
                            title="Login failed"
                            message="The catalog did not accept the sign-in. Log in again to continue."
                            on_reload=noop()
                            on_dot_quilt=noop()
                            on_login=noop()
                            on_home=noop()
                        />
                    }
                        .into_any(),
                )}
            </Cell>
            <Cell wide=true label="Error — loading">
                {window("Error", None, false, view! { <ErrorSlateSkeleton /> }.into_any())}
            </Cell>
            <Cell wide=true label="Page not found">
                {window(
                    "Page not found",
                    None,
                    false,
                    view! {
                        <ErrorSlate
                            title="Page not found"
                            message="The page you are looking for does not exist."
                            on_home=noop()
                        />
                    }
                        .into_any(),
                )}
            </Cell>
            <Cell wide=true label="Deep link — running: Refresh disabled">
                {window(
                    "Opening link",
                    None,
                    true,
                    view! { <Spinner variant=SpinnerVariant::Region aria_label="Opening link" /> }
                        .into_any(),
                )}
            </Cell>
            <Cell wide=true label="Deep link — failed, in the same frame">
                {window(
                    "Opening link",
                    None,
                    true,
                    view! {
                        <ErrorSlate
                            title="Error"
                            message="Package team/plate-07 was not found in s3://example-bucket."
                            on_reload=noop()
                            on_dot_quilt=noop()
                            on_home=noop()
                        />
                    }
                        .into_any(),
                )}
            </Cell>
        </Scene>
    }
}
