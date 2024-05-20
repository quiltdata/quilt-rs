use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::RwLock;

use aws_config::BehaviorVersion;
use aws_types::region::Region;
use lazy_static::lazy_static;

use crate::Error;
use crate::Res;

async fn find_bucket_region(client: &reqwest::Client, bucket: &str) -> Res<String> {
    let response = client
        .head(format!("https://s3.amazonaws.com/{bucket}"))
        .send()
        .await?;

    match response.headers().get("x-amz-bucket-region") {
        Some(location) => Ok(location.to_str()?.into()),
        None => Err(Error::MissingHTTPHeader("x-amz-bucket-region".to_string())),
    }
}

lazy_static! {
    static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::new();
    static ref BUCKET_REGIONS: RwLock<HashMap<String, Region>> = RwLock::new(HashMap::new());
    static ref REGION_CLIENTS: RwLock<HashMap<Region, aws_sdk_s3::Client>> =
        RwLock::new(HashMap::new());
}

async fn get_region_for_bucket(bucket: &str) -> Res<Region> {
    {
        let map = BUCKET_REGIONS.read().unwrap();
        if let Some(region) = map.get(bucket) {
            return Ok(region.clone());
        }
    }

    let region = find_bucket_region(&HTTP_CLIENT, bucket).await?;

    let mut map = BUCKET_REGIONS.write().unwrap();
    match map.entry(bucket.to_owned()) {
        Entry::Occupied(entry) => Ok(entry.get().clone()),
        Entry::Vacant(entry) => Ok(entry.insert(Region::new(region)).clone()),
    }
}

async fn get_client_for_region(region: aws_types::region::Region) -> aws_sdk_s3::Client {
    {
        let map = REGION_CLIENTS.read().unwrap();
        if let Some(client) = map.get(&region) {
            return client.clone();
        }
    }

    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region.clone())
        .load()
        .await;
    let client = aws_sdk_s3::Client::new(&config);

    let mut map = REGION_CLIENTS.write().unwrap();

    match map.entry(region) {
        Entry::Occupied(entry) => entry.get().clone(),
        Entry::Vacant(entry) => entry.insert(client).clone(),
    }
}

pub async fn get_client_for_bucket(bucket: &str) -> Res<aws_sdk_s3::Client> {
    let region = get_region_for_bucket(bucket).await?.clone();
    Ok(get_client_for_region(region).await)
}
