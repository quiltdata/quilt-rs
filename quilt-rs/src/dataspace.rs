//! A dataspace: a folder of working folders, with the contract, `data.toml`,
//! that says what should happen to the packages they hold, and the lock,
//! `data.lock.toml`, that records which revisions were used.

pub mod contract;
pub mod lock;
