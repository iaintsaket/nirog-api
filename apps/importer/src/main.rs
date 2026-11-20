//! Usage: `nirog-importer [--jan-aushadhi PATH] [--brands PATH]`
//! Reads `DATABASE_URL`. With no flags, imports the bundled Jan Aushadhi list only.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use nirog_application::ports::PriceSource;
use nirog_application::services::import::import_source;
use nirog_persistence::PgOfferRepository;
use nirog_sources::{BrandDatasetCsv, JanAushadhiCsv};

#[derive(Debug, PartialEq, Eq)]
struct Args {
    jan_aushadhi: PathBuf,
    brands: Option<PathBuf>,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args> {
    let mut out = Args {
        jan_aushadhi: PathBuf::from("data/janaushadhi-products.csv"),
        brands: None,
    };
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .with_context(|| format!("{flag} needs a path"))?;
        match flag.as_str() {
            "--jan-aushadhi" => out.jan_aushadhi = value.into(),
            "--brands" => out.brands = Some(value.into()),
            other => bail!("unknown flag {other}"),
        }
    }
    Ok(out)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let pool = nirog_persistence::connect(&url, 2).await?;
    nirog_persistence::migrate(&pool).await?;
    let repo = PgOfferRepository::new(pool);

    let mut sources: Vec<Box<dyn PriceSource>> =
        vec![Box::new(JanAushadhiCsv::new(args.jan_aushadhi))];
    if let Some(path) = args.brands {
        sources.push(Box::new(BrandDatasetCsv::new(path)));
    }
    for source in &sources {
        let report = import_source(source.as_ref(), &repo).await?;
        println!(
            "{}: recorded {}, skipped {}, duplicates {}",
            source.info().id,
            report.imported,
            report.skipped,
            report.duplicates
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Result<Args> {
        parse_args(s.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn parses_flags_and_defaults() -> Result<()> {
        assert_eq!(args(&[])?.brands, None);
        let a = args(&["--brands", "b.csv", "--jan-aushadhi", "j.csv"])?;
        assert_eq!(
            a,
            Args {
                jan_aushadhi: "j.csv".into(),
                brands: Some("b.csv".into())
            }
        );
        assert!(args(&["--brands"]).is_err());
        assert!(args(&["--nope", "x"]).is_err());
        Ok(())
    }
}
