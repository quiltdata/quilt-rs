//! A timing run for comparing Tauri IPC with the WebSocket (see `WEB-MVP.md`).
//!
//! The desktop app sets `window.__QS_BENCH__` to a package's namespace when
//! started with `QUILTSYNC_BENCH`. The page then times the biggest reads and the
//! package page's render over whichever transport it uses, and hands the results
//! to the backend's `bench_report`.

use std::fmt::Write;

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast;

use crate::commands;
use crate::tauri;

const RUNS: usize = 5;
const RENDERS: usize = 5;

/// The namespace to measure, when this is a timing run.
pub fn namespace() -> Option<String> {
    let window = web_sys::window()?;
    js_sys::Reflect::get(&window, &"__QS_BENCH__".into())
        .ok()?
        .as_string()
}

fn now() -> f64 {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"performance".into()).ok())
        .and_then(|p| {
            let now = js_sys::Reflect::get(&p, &"now".into()).ok()?;
            now.dyn_into::<js_sys::Function>().ok()?.call0(&p).ok()
        })
        .and_then(|v| v.as_f64())
        .unwrap_or_else(js_sys::Date::now)
}

async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// The next frame, or the next task when the page is hidden and draws no
/// frames.
async fn next_frame() {
    let hidden = web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden());
    if hidden {
        sleep_ms(0).await;
        return;
    }
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.request_animation_frame(&resolve);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

fn present(selector: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(selector).ok().flatten())
        .is_some()
}

fn summary(times: &mut [f64]) -> String {
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    let min = times[0];
    let max = times[times.len() - 1];
    format!("median {median:.1} ms, min {min:.1}, max {max:.1}")
}

/// Hand a line to the backend's `bench_report` right away, so a run that
/// stalls still shows how far it got.
async fn report(text: String) {
    #[derive(Serialize)]
    struct Text {
        text: String,
    }
    let _ = tauri::invoke::<_, ()>("bench_report", &Text { text }).await;
}

async fn time_read<A: Serialize, R: DeserializeOwned, F, Fut>(
    out: &mut String,
    cmd: &str,
    args: &A,
    call: F,
) where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<R, String>>,
{
    report(format!("progress: {cmd} probe")).await;
    let mut decodes = Vec::with_capacity(RUNS);
    let mut wire = 0;
    for _ in 0..3 {
        match tauri::probe_decode::<A, R>(cmd, args, now).await {
            Ok((len, ms)) => {
                wire = len;
                decodes.push(ms);
            }
            Err(err) => {
                let _ = writeln!(out, "  {cmd}: probe error {err}");
                return;
            }
        }
    }
    let mut times = Vec::with_capacity(RUNS);
    for i in 0..RUNS {
        report(format!("progress: {cmd} run {i}")).await;
        let started = now();
        if let Err(err) = call().await {
            let _ = writeln!(out, "  {cmd}: error {err}");
            return;
        }
        times.push(now() - started);
    }
    let line = format!(
        "  {cmd}: {wire} bytes, call→typed {}; decode alone {}",
        summary(&mut times),
        summary(&mut decodes)
    );
    report(format!("progress:{line}")).await;
    let _ = writeln!(out, "{line}");
}

/// Run the timings, then report them. `navigate` is the router's.
pub async fn run(namespace: String, navigate: impl Fn(&str)) {
    #[derive(Serialize)]
    struct Ns {
        namespace: String,
    }
    #[derive(Serialize)]
    struct NsFilter {
        namespace: String,
        filter: Option<String>,
    }
    // Let the first page settle and its own reads finish.
    sleep_ms(3000).await;
    let mode = if tauri::is_web_mode() { "ws" } else { "ipc" };
    let mut out = format!("bench {mode} {namespace}\n");
    report(format!("progress: start {mode}")).await;

    let ns = Ns {
        namespace: namespace.clone(),
    };
    let nsf = NsFilter {
        namespace: namespace.clone(),
        filter: None,
    };
    time_read(&mut out, "get_package_page_data", &ns, || {
        commands::get_package_page_data(namespace.clone())
    })
    .await;
    time_read(&mut out, "get_installed_package_data", &nsf, || {
        commands::get_installed_package_data(namespace.clone(), None)
    })
    .await;
    time_read(&mut out, "get_main_page_packages", &(), || {
        commands::get_main_page_packages()
    })
    .await;
    time_read(&mut out, "get_installed_packages_list_data", &(), || {
        commands::get_installed_packages_list_data()
    })
    .await;

    // The package page, from the navigation to the frame after its context
    // pane (drawn once the page's data is in) appears.
    let target = format!(
        "/installed-package?namespace={}",
        urlencoding::encode(&namespace)
    );
    let mut renders = Vec::with_capacity(RENDERS);
    for _ in 0..RENDERS {
        navigate("/settings");
        sleep_ms(1500).await;
        let started = now();
        navigate(&target);
        let mut waited = 0;
        while !present("aside[aria-label='About this package']") && waited < 60_000 {
            next_frame().await;
            waited += 16;
        }
        next_frame().await;
        renders.push(now() - started);
        sleep_ms(1500).await;
    }
    let _ = writeln!(out, "  package page render: {}", summary(&mut renders));

    report(out).await;
}
