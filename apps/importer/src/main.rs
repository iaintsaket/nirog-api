//! Usage:
//!   `nirog-importer [--jan-aushadhi PATH] [--brands PATH] [--clean OUT_DIR/<source>.csv ...]`
//!   `nirog-importer clean --source ID RAW.csv OUT_DIR` (IDs: see `nirog_sources::clean::cleaners`)
//!
//! Import reads `DATABASE_URL` and adds prices; it never deletes. With no flags
//! it imports the bundled Jan Aushadhi list only. `clean` needs no database: it
//! writes `OUT_DIR/<source>.csv` (Nirog CSV) and `OUT_DIR/<source>.rejects.csv`.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use nirog_application::ports::{PriceSource, SourceInfo};
use nirog_application::services::import::import_source;
use nirog_persistence::PgOfferRepository;
use nirog_sources::clean::{
    NirogCsv, cleaner_for, cleaners, known_source, write_clean, write_rejects,
};
use nirog_sources::{BrandDatasetCsv, JanAushadhiCsv};

#[derive(Debug, PartialEq, Eq)]
struct Args {
    jan_aushadhi: PathBuf,
    brands: Option<PathBuf>,
    clean: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Import(Args),
    Clean {
        source: String,
        input: PathBuf,
        out_dir: PathBuf,
    },
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command> {
    let mut first = args.next();
    if first.as_deref() == Some("clean") {
        let rest: Vec<String> = args.collect();
        let [flag, source, input, out_dir] = <[String; 4]>::try_from(rest)
            .map_err(|_| anyhow::anyhow!("usage: clean --source ID RAW.csv OUT_DIR"))?;
        if flag != "--source" || known_source(&source).is_none() {
            let known: Vec<_> = cleaners().iter().map(|c| c.info().id).collect();
            bail!(
                "usage: clean --source ID RAW.csv OUT_DIR (known: {})",
                known.join(", ")
            );
        }
        return Ok(Command::Clean {
            source,
            input: input.into(),
            out_dir: out_dir.into(),
        });
    }
    let mut out = Args {
        jan_aushadhi: PathBuf::from("data/janaushadhi-products.csv"),
        brands: None,
        clean: Vec::new(),
    };
    while let Some(flag) = first.take().or_else(|| args.next()) {
        let value = args
            .next()
            .with_context(|| format!("{flag} needs a path"))?;
        match flag.as_str() {
            "--jan-aushadhi" => out.jan_aushadhi = value.into(),
            "--brands" => out.brands = Some(value.into()),
            "--clean" => out.clean.push(value.into()),
            other => bail!("unknown flag {other}"),
        }
    }
    Ok(Command::Import(out))
}

/// A cleaned file is named after its source: `onemg-2025.csv`.
fn source_of(path: &Path) -> Option<SourceInfo> {
    known_source(path.file_stem()?.to_str()?)
}

fn clean(source: &str, input: &Path, out_dir: &Path) -> Result<()> {
    let cleaner = cleaner_for(source).context("unknown source")?;
    let info = cleaner.info();
    let mut raw = File::open(input).with_context(|| input.display().to_string())?;
    let cleaned = cleaner.clean(&mut raw)?;
    std::fs::create_dir_all(out_dir)?;
    let rows_path = out_dir.join(format!("{source}.csv"));
    let rejects_path = out_dir.join(format!("{source}.rejects.csv"));
    write_clean(
        &cleaned.rows,
        &info,
        BufWriter::new(File::create(&rows_path)?),
    )?;
    write_rejects(
        &cleaned.rejects,
        BufWriter::new(File::create(&rejects_path)?),
    )?;
    let mut reasons = BTreeMap::new();
    for r in &cleaned.rejects {
        *reasons.entry(r.reason.as_str()).or_insert(0) += 1;
    }
    println!(
        "{source}: {} clean -> {}, {} rejected -> {} {reasons:?}",
        cleaned.rows.len(),
        rows_path.display(),
        cleaned.rejects.len(),
        rejects_path.display()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = match parse_args(std::env::args().skip(1))? {
        Command::Clean {
            source,
            input,
            out_dir,
        } => return clean(&source, &input, &out_dir),
        Command::Import(args) => args,
    };
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let pool = nirog_persistence::connect(&url, 2).await?;
    nirog_persistence::migrate(&pool).await?;
    let repo = PgOfferRepository::new(pool);

    let mut sources: Vec<Box<dyn PriceSource>> =
        vec![Box::new(JanAushadhiCsv::new(args.jan_aushadhi))];
    if let Some(path) = args.brands {
        sources.push(Box::new(BrandDatasetCsv::new(path)));
    }
    for path in args.clean {
        let info = source_of(&path).with_context(|| {
            format!(
                "{}: name it <source>.csv, e.g. onemg-2025.csv",
                path.display()
            )
        })?;
        sources.push(Box::new(NirogCsv::new(path, info)));
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

    fn args(s: &[&str]) -> Result<Command> {
        parse_args(s.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn parses_flags_and_defaults() -> Result<()> {
        let import = |a| match a {
            Command::Import(a) => a,
            Command::Clean { .. } => unreachable!(),
        };
        assert_eq!(import(args(&[])?).brands, None);
        let a = import(args(&[
            "--brands",
            "b.csv",
            "--jan-aushadhi",
            "j.csv",
            "--clean",
            "c/onemg-2025.csv",
        ])?);
        assert_eq!(
            a,
            Args {
                jan_aushadhi: "j.csv".into(),
                brands: Some("b.csv".into()),
                clean: vec!["c/onemg-2025.csv".into()],
            }
        );
        assert_eq!(
            args(&["clean", "--source", "onemg-2025", "raw.csv", "out"])?,
            Command::Clean {
                source: "onemg-2025".into(),
                input: "raw.csv".into(),
                out_dir: "out".into(),
            }
        );
        assert!(args(&["clean", "--source", "nope", "raw.csv", "out"]).is_err());
        assert!(args(&["clean", "raw.csv"]).is_err());
        assert!(args(&["--brands"]).is_err());
        assert!(args(&["--nope", "x"]).is_err());
        Ok(())
    }

    #[test]
    fn cleaned_file_names_its_source() {
        assert_eq!(
            source_of("out/onemg-2025.csv".as_ref()).map(|s| s.id),
            Some("onemg-2025".into())
        );
        assert!(source_of("out/unknown.csv".as_ref()).is_none());
    }
}
