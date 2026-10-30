use std::path::Path;

fn main() {
    // `tauri::generate_context!` panics at compile time when `frontendDist` is
    // missing, and git cannot track an empty directory — so it used to be held open
    // by a `dist/.gitkeep` that the frontend build kept deleting. Create it here
    // instead: every command that compiles this crate runs the build script first.
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/dist");
    std::fs::create_dir_all(&dist)
        .unwrap_or_else(|err| panic!("could not create {}: {err}", dist.display()));
    // A release build embeds the UI for its server; without it the app would
    // serve an empty page.
    let index = dist.join("index.html");
    assert!(
        std::env::var("PROFILE").as_deref() != Ok("release") || index.exists(),
        "{} is missing: run `trunk build --release` in quilt-sync/ui first",
        index.display()
    );

    tauri_build::build();
}
