use std::{
    collections::{hash_map::Entry, HashMap},
    sync::RwLock,
};

use aws_config::BehaviorVersion;
use aws_types::region::Region;
use lazy_static::lazy_static;

pub async fn find_bucket_region(client: &reqwest::Client, bucket: &str) -> Result<String, String> {
    let response = client
        .head(format!("https://s3.amazonaws.com/{bucket}"))
        .send()
        .await;

    match response {
        Ok(content) => match content.headers().get("x-amz-bucket-region") {
            Some(location) => Ok(location.to_str().map_err(|err| err.to_string())?.into()),
            None => Err("failed to find location header".into()),
        },
        Err(err) => Err(err.to_string()),
    }
}

lazy_static! {
    // static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::new();
    static ref BUCKET_REGIONS: RwLock<HashMap<String, Region>> = RwLock::new(HashMap::new());
    static ref REGION_CLIENTS: RwLock<HashMap<Region, aws_sdk_s3::Client>> = RwLock::new(HashMap::new());
}

pub async fn get_region_for_bucket(bucket: &str) -> Result<Region, String> {
    {
        let map = BUCKET_REGIONS.read().unwrap();
        if let Some(region) = map.get(bucket) {
            return Ok(region.clone());
        }
    }

    let http_client = reqwest::Client::new(); // TODO: use a global here, too.
    let region = find_bucket_region(&http_client, &bucket).await?;

    let mut map = BUCKET_REGIONS.write().unwrap();
    match map.entry(bucket.to_owned()) {
        Entry::Occupied(entry) => Ok(entry.get().clone()),
        Entry::Vacant(entry) => Ok(entry.insert(Region::new(region)).clone()),
    }
}

pub async fn get_client_for_region(region: aws_types::region::Region) -> aws_sdk_s3::Client {
    {
        let map = REGION_CLIENTS.read().unwrap();
        if let Some(client) = map.get(&region) {
            return client.clone();
        }
    }

    let config = aws_config::defaults(BehaviorVersion {}).region(region.clone()).load().await;
    let client = aws_sdk_s3::Client::new(&config);

    let mut map = REGION_CLIENTS.write().unwrap();

    match map.entry(region) {
        Entry::Occupied(entry) => entry.get().clone(),
        Entry::Vacant(entry) => entry.insert(client).clone(),
    }
}

pub async fn get_client_for_bucket(bucket: &str) -> Result<aws_sdk_s3::Client, String> {
    let region = get_region_for_bucket(bucket).await?.clone();
    Ok(get_client_for_region(region).await)
}
