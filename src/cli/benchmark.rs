use std::path::PathBuf;

use async_stream::stream;
use multihash::Multihash;
use tracing::log;

use quilt_rs::manifest::Row;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;
use crate::perf::Measure;

#[derive(Debug)]
pub struct Input {
    pub dest: PathBuf,
    pub number: i32,
}

pub struct Output {
    pub dest: PathBuf,
    pub perf: Measure,
    pub top_hash: String,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output: Vec<String> = Vec::new();
        output.push(format!("Manifest written to {:?}", &self.dest));
        output.push(format!("With hash {}", &self.top_hash));
        output.push(format!("And it took {}", &self.perf.elapsed()));
        write!(f, "{}", output.join("\n"))
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.benchmark(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

async fn benchmark(
    local_domain: &quilt_rs::LocalDomain,
    dest: PathBuf,
    number: i32,
) -> Result<(PathBuf, String), Error> {
    let mut i = 0;
    let stream = stream! {
        let mut chunk = Vec::new();
        while i < number {
            let name = PathBuf::from(format!("file://{}", i));
            let row= Row {
                name,
                hash: Multihash::wrap(0xb510, b"pedestrian").expect("Unexpected"),
                ..Row::default()
            };
            chunk.push(Ok(row));

            if (i > 0 && i % 100_000 == 0) || (i == number -1) {
                yield(Ok(chunk));
                chunk = vec![];
            }

            if i > 0 && i % 10_000 == 0 && i < 100_000 {
                log::debug!("Another 10k rows written, {}", i);
            }
            if i > 0 && i % 100_000 == 0 && i < 1_000_000 {
                log::debug!("Another 100k rows written, {}", i);
            }
            if i > 0 && i % 1_000_000 == 0 {
                log::debug!("Another million rows written, {}", i);
            }
            i += 1;
        }
    };
    Ok(local_domain.build_manifest(dest, Box::pin(stream)).await?)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { dest, number }: Input,
) -> Result<Output, Error> {
    let perf = Measure::start();
    let (dest, top_hash) = benchmark(local_domain, dest, number).await?;
    Ok(Output {
        dest,
        perf,
        top_hash,
    })
}
