use std::path::PathBuf;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;

pub struct Output {
    archive: PathBuf,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Rolled this domain's lineage back to data.json; the refs are kept in {}.\n\
             A quilt or QuiltSync that keeps lineage in refs moves it there again the next time it opens this domain.",
            self.archive.display()
        )
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "archive": self.archive.to_string_lossy() })
    }
}

pub async fn command(m: impl Commands) -> Std {
    Std::from_result(m.lineage_rollback().await)
}

pub async fn model(local_domain: &quilt_rs::LocalDomain) -> Result<Output, Error> {
    let archive = local_domain.rollback_lineage().await?;
    Ok(Output { archive })
}

#[cfg(test)]
mod tests {
    use super::*;

    use clap::Parser;
    use quilt_rs::flow::UserMeta;
    use test_log::test;

    use crate::cli::Args;
    use crate::cli::init;
    use crate::cli::model::create_ref_store_model_in_temp_dir;

    #[test(tokio::test)]
    async fn rollback_hands_the_lineage_back_and_says_it_moves_again() -> Result<(), Error> {
        let (m, temp_dir) = create_ref_store_model_in_temp_dir().await?;
        let namespace: quilt_uri::Namespace = ("test", "kept").into();
        m.create(crate::cli::create::Input {
            namespace: namespace.clone(),
            source: None,
            message: None,
            user_meta: UserMeta::Clear,
        })
        .await?;

        let output = m.lineage_rollback().await?;

        let on_data_json = quilt_rs::LocalDomain::new(temp_dir.path());
        assert!(
            on_data_json
                .get_installed_package(&namespace)
                .await?
                .is_some()
        );
        assert!(output.archive.join("test/kept.json").is_file());
        let text = output.to_string();
        assert!(
            text.contains(&output.archive.display().to_string()),
            "{text}"
        );
        assert!(text.contains("moves it there again"), "{text}");
        assert_eq!(
            output.to_json(),
            serde_json::json!({"archive": output.archive.to_string_lossy()})
        );
        Ok(())
    }

    /// Any other command refuses a rollback that was interrupted, so this
    /// one finishes it before the home is read or the leftovers swept.
    #[test(tokio::test)]
    async fn an_interrupted_rollback_is_finished_before_the_home_is_read() -> Result<(), Error> {
        let (m, temp_dir) = create_ref_store_model_in_temp_dir().await?;
        let dot_quilt = temp_dir.path().join(".quilt");
        let config = serde_json::json!({
            "format": 1,
            "home": temp_dir.path(),
            "migration": {"rolling_back": {"archive": "refs.rolled-back-67e55044-10b1-426f-9247-bb680e5fe0c8"}},
        });
        std::fs::write(dot_quilt.join("domain.json"), serde_json::to_vec(&config)?)?;
        drop(m);

        let domain = temp_dir.path().to_string_lossy().to_string();
        let args = Args::try_parse_from(["quilt", "--domain", &domain, "lineage", "rollback"])
            .map_err(|err| Error::Test(err.to_string()))?;
        let output = init(args).await;

        assert!(matches!(output, Ok(Std::Out(_))), "{output:?}");
        assert!(!dot_quilt.join("domain.json").exists());
        assert!(
            quilt_rs::LocalDomain::new(temp_dir.path())
                .get_home()
                .await
                .is_ok()
        );
        Ok(())
    }
}
