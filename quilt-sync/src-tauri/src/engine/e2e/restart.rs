//! Tests that stop the engine and start it again on the same folder.
//!
//! A restart frees its port for a moment, and an engine another test starts
//! then could take it, so each runs alone (see `.config/nextest.toml`).

use super::*;

#[test]
fn an_open_page_reconnects_after_quiltsync_restarts() {
    test(|base| async move {
        let mut quiltsync = QuiltSync::start(&base, true).await;
        quiltsync.create_package("e2e/before").await;
        let driver = browser().await;
        driver.goto(quiltsync.url("/")).await?;
        driver.query(containing("a", "e2e/before")).first().await?;
        mark(&driver).await?;

        let (origin, token) = (quiltsync.origin.clone(), quiltsync.token.clone());
        quiltsync.stop();
        driver
            .query(containing("*", "Reconnecting"))
            .first()
            .await?;
        // Made while the engine is away, so the page learns of it only by
        // reading again once back.
        let domain = quiltsync.domain.clone();
        off_runtime(move || {
            let namespace = quilt_uri::Namespace::try_from("e2e/after").expect("a namespace");
            let model = crate::engine::model::Model::create(&domain);
            tokio::runtime::Runtime::new()
                .expect("a runtime")
                .block_on(crate::engine::model::package_create(
                    &model, namespace, None, None,
                ))
                .expect("a package made with no engine");
        });
        quiltsync.resume();
        assert_eq!(quiltsync.origin, origin, "the same port");
        assert_eq!(quiltsync.token, token, "the same token");

        driver.query(containing("a", "e2e/after")).first().await?;
        driver.query(containing("a", "e2e/before")).first().await?;
        assert!(still_marked(&driver).await?, "the page reloaded");
        driver.quit().await
    });
}

#[test]
fn a_page_with_the_old_link_says_to_open_the_printed_one() {
    test(|base| async move {
        let mut quiltsync = QuiltSync::start(&base, true).await;
        let driver = browser().await;
        driver.goto(quiltsync.url("/")).await?;
        driver.query(text("No packages yet")).first().await?;

        // Without its token file, the engine makes a new token as it starts.
        let token = quiltsync.token.clone();
        quiltsync.stop();
        std::fs::remove_file(quiltsync.domain.join("web-token"))?;
        quiltsync.resume();
        assert_ne!(quiltsync.token, token);

        driver
            .query(containing("*", "Open the link it printed when it started."))
            .first()
            .await?;
        let page = driver.find(By::Tag("body")).await?.text().await?;
        assert!(!page.contains("tray"), "{page}");
        driver.quit().await
    });
}
