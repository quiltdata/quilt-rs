use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};
use serde::{Deserialize, Serialize};

use crate::components::{Layout, Spinner};
use crate::tauri;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemotePackageResult {
    namespace: String,
    notification: Option<String>,
}

#[component]
pub fn RemotePackage() -> impl IntoView {
    let notification = RwSignal::new(String::new());
    let navigate = use_navigate();
    let query = use_query_map();

    let data = LocalResource::new(move || {
        let uri = query.read().get("uri").unwrap_or_default();
        let navigate = navigate.clone();
        async move {
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Args {
                uri: String,
            }
            let result =
                tauri::invoke::<_, RemotePackageResult>("handle_remote_package", &Args { uri })
                    .await?;

            // Navigate to the installed package page
            let ns = &result.namespace;
            let path = match result.notification {
                Some(ref msg) => format!(
                    "/installed-package?namespace={ns}&filter=unmodified&notification={}",
                    urlencoding::encode(msg)
                ),
                None => format!("/installed-package?namespace={ns}&filter=unmodified"),
            };
            navigate(&path, Default::default());
            Ok::<_, String>(result)
        }
    });

    view! {
        <Layout breadcrumbs=vec![] notification=notification>
            <Suspense fallback=move || {
                view! { <Spinner /> }
            }>
                {move || Suspend::new(async move {
                    match data.await {
                        Ok(_) => view! { <Spinner /> }.into_any(),
                        Err(e) => {
                            crate::error_handler::handle_or_display(&e, notification)
                        }
                    }
                })}
            </Suspense>
        </Layout>
    }
}
