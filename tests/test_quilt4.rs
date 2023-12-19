use quilt_rs::*;
use poem::web::Json;

pub async fn make_manifest(path_name: String) -> Manifest4 {
    let up = UPath::new(path_name);
    let cl = Client::new().await;
    let dom = Domain::new(cl.clone(), up.clone()).await;
    let nam = Namespace::new(dom, up.clone()).await;
    let tab = Table::new(Some(up.clone())).await;
    let manifest = Manifest4::new(
        nam.clone(),
        tab.clone(),
        Some(up.clone())
    ).await;
    manifest
}
 
#[tokio::test]
async fn test_quilt4_manifest() {
    let manifest = make_manifest("test".to_string()).await;
    let mstring = manifest.to_string();
    assert!(mstring.starts_with("Manifest4(UPath(test)"));
    let json: Json<Manifest4> = Json(manifest);
    println!("json: {:#?}", json);
    assert!(json.0.to_string().starts_with(mstring.as_str()));
}
