/// The `DataYaml` struct represents configuration data for a Quilt-RS project.
/// It provides methods for creating a new instance of `DataYaml` and extracting its contents from the YAML file.
/// It will also read the following environment variables:
/// - `QUILT_DATA_HOME`
/// - `QUILT_WORK_ROOT`
/// and store them in the `env` field (unless overriden by the YAML file).
/// 
/// Every Entry contains a user, timestamp, action, and either a uri or a folder (or both).
/// Typically, `quilt-rs` methods will log uri entries in a registry,
/// and `quilt4` methods will log folder entries in a working directory.
/// TBD: should those be two disjoint data structures?
/// 
/// NOTE: Unlike 'lineage' DataYaml stores package actions against an 'opaque' URI,
/// so that the format is independent of the various fields it may contain.

use figment::{
    providers::{Env, Format, Yaml},
    Figment,
};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub enum Action {
    Push,
    Pull,
    Get,
    Put,
    Commit,
}

#[derive(Deserialize)]
pub struct Entry {
    pub user: String,
    pub timestamp: String,
    pub action: Action,
    pub uri: Option<String>,
    pub folder: Option<String>,
}

#[derive(Deserialize)]
pub struct Environment {
    pub data_home: String,
    pub work_root: String,
}

#[derive(Deserialize)]
pub struct DataYaml {
    pub version: String,
    pub env: Environment,
    pub uris: HashMap<String, Entry>, // quilt-rs actions, by uri
    pub folders: HashMap<String, Entry>, // quilt4 actions, by folder
}

impl DataYaml {
    pub fn new() -> Self {
        let data_yaml: DataYaml = Figment::new()
            .merge(Env::prefixed("QUILT_"))
            .merge(Env::raw().only(&["AWS_ACCESS_KEY", "AWS_SECRET_ACCESS_KEY"]))
            .join(Yaml::file("data.yaml"))
            .extract()
            .unwrap();
        data_yaml
    }
}
