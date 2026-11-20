use async_trait::async_trait;
use nirog_application::RepoError;
use nirog_application::ports::OfferRepository;
use nirog_domain::{MatchKey, Offer, OfferId, Pack, PackUnit, Paise, SourceKind};
use sqlx::{PgPool, Row, postgres::PgRow};

use crate::db;

macro_rules! select_offers {
    ($rest:literal) => {
        concat!(
            "SELECT id, source, name, manufacturer, price_paise, pack_count, pack_unit, match_key, salts_label FROM offers ",
            $rest
        )
    };
}

#[derive(Debug, Clone)]
pub struct PgOfferRepository {
    pool: PgPool,
}

impl PgOfferRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn to_offer(row: &PgRow) -> Result<Offer, RepoError> {
    let bad = |what: &str| RepoError(format!("corrupt offer row: {what}"));
    let count: i32 = row.try_get("pack_count").map_err(db)?;
    Ok(Offer {
        id: OfferId(row.try_get("id").map_err(db)?),
        source: SourceKind::parse(row.try_get("source").map_err(db)?).map_err(|_| bad("source"))?,
        name: row.try_get("name").map_err(db)?,
        manufacturer: row.try_get("manufacturer").map_err(db)?,
        price: Paise(row.try_get("price_paise").map_err(db)?),
        pack: Pack {
            count: u32::try_from(count).map_err(|_| bad("pack_count"))?,
            unit: PackUnit::parse(row.try_get("pack_unit").map_err(db)?)
                .map_err(|_| bad("pack_unit"))?,
        },
        key: MatchKey::parse(row.try_get("match_key").map_err(db)?)
            .ok_or_else(|| bad("match_key"))?,
        salts_label: row.try_get("salts_label").map_err(db)?,
    })
}

/// Queries shorter than this match the start of a name or salt only: an infix
/// match on two letters is noisy and cannot use the trigram index.
const PREFIX_ONLY_BELOW: usize = 3;

/// Escapes LIKE wildcards so user input is matched literally.
fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
        .to_lowercase();
    if query.chars().count() < PREFIX_ONLY_BELOW {
        format!("{escaped}%")
    } else {
        format!("%{escaped}%")
    }
}

#[async_trait]
impl OfferRepository for PgOfferRepository {
    async fn search(&self, query: &str, limit: u32) -> Result<Vec<Offer>, RepoError> {
        // Trigram similarity means little for two letters, so short queries rank
        // brand-name hits first, shortest name first ("Dolo 650" before "Dolokind Plus").
        let short = query.chars().count() < PREFIX_ONLY_BELOW;
        let sql = if short {
            select_offers!(
                "WHERE lower(name) LIKE $1 OR lower(salts_label) LIKE $1
                 ORDER BY (lower(name) LIKE $1) DESC, length(name), unit_price_millipaise, id
                 LIMIT $2"
            )
        } else {
            select_offers!(
                "WHERE lower(name) LIKE $1 OR lower(salts_label) LIKE $1
                 ORDER BY greatest(similarity(lower(name), $2), similarity(lower(salts_label), $2)) DESC,
                          unit_price_millipaise, id
                 LIMIT $3"
            )
        };
        let mut q = sqlx::query(sql).bind(like_pattern(query));
        if !short {
            q = q.bind(query.to_lowercase());
        }
        let rows = q
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.iter().map(to_offer).collect()
    }

    async fn closest_names(&self, query: &str, limit: u32) -> Result<Vec<String>, RepoError> {
        // `<%` is pg_trgm's word-similarity operator: it scores the query against
        // the best-matching part of a name, so "Tablet" suffixes don't dilute it. Index-served.
        let sql = "SELECT v FROM (
                SELECT name AS v, word_similarity($1, lower(name)) AS s FROM offers WHERE $1 <% lower(name)
                UNION ALL
                SELECT salts_label, word_similarity($1, lower(salts_label)) FROM offers WHERE $1 <% lower(salts_label)
             ) t GROUP BY v ORDER BY max(s) DESC, length(v), v LIMIT $2";
        // The default cutoff (0.6) misses one-letter typos such as "dollo" for "Dolo" (0.57).
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("SELECT set_config('pg_trgm.word_similarity_threshold', '0.5', true)")
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let names = sqlx::query_scalar(sql)
            .bind(query.to_lowercase())
            .bind(i64::from(limit))
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(names)
    }

    async fn by_key(&self, key: &MatchKey) -> Result<Vec<Offer>, RepoError> {
        let sql = select_offers!("WHERE match_key = $1 ORDER BY unit_price_millipaise, id");
        let rows = sqlx::query(sql)
            .bind(key.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.iter().map(to_offer).collect()
    }

    async fn key_exists(&self, key: &MatchKey) -> Result<bool, RepoError> {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM offers WHERE match_key = $1)")
            .bind(key.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(db)
    }

    async fn replace_source(&self, source: SourceKind, offers: &[Offer]) -> Result<u64, RepoError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query("DELETE FROM offers WHERE source = $1")
            .bind(source.as_str())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let mut inserted = 0;
        // Chunked UNNEST inserts: one round trip per 1,000 rows.
        for chunk in offers.chunks(1000) {
            let mut cols: [Vec<String>; 6] = Default::default();
            let (mut price, mut count, mut unit_price) = (Vec::new(), Vec::new(), Vec::new());
            let mut unit = Vec::new();
            for o in chunk {
                if o.source != source {
                    return Err(RepoError(format!(
                        "offer {} is not from {}",
                        o.id.0,
                        source.as_str()
                    )));
                }
                cols[0].push(o.id.0.clone());
                cols[1].push(o.source.as_str().to_owned());
                cols[2].push(o.name.clone());
                cols[3].push(o.manufacturer.clone());
                cols[4].push(o.key.as_str().to_owned());
                cols[5].push(o.salts_label.clone());
                price.push(o.price.0);
                count.push(
                    i32::try_from(o.pack.count)
                        .map_err(|_| RepoError("pack count too large".into()))?,
                );
                unit.push(o.pack.unit.as_str().to_owned());
                unit_price.push(o.unit_price_millipaise());
            }
            let [ids, sources, names, makers, keys, labels] = cols;
            let res = sqlx::query(
                "INSERT INTO offers (id, source, name, manufacturer, match_key, salts_label,
                                     price_paise, pack_count, pack_unit, unit_price_millipaise)
                 SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[], $5::text[], $6::text[],
                                      $7::bigint[], $8::int[], $9::text[], $10::bigint[])",
            )
            .bind(ids)
            .bind(sources)
            .bind(names)
            .bind(makers)
            .bind(keys)
            .bind(labels)
            .bind(price)
            .bind(count)
            .bind(unit)
            .bind(unit_price)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            inserted += res.rows_affected();
        }
        tx.commit().await.map_err(db)?;
        Ok(inserted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nirog_domain::{Form, Release, Salt};

    pub(crate) fn offer(
        id: &str,
        source: SourceKind,
        name: &str,
        paise: i64,
        count: u32,
        mg: u64,
    ) -> Offer {
        let salts = [Salt::mg("paracetamol", mg * 1000)];
        Offer {
            id: OfferId(id.into()),
            source,
            name: name.into(),
            manufacturer: "Maker".into(),
            price: Paise(paise),
            pack: Pack {
                count,
                unit: PackUnit::Unit,
            },
            key: MatchKey::new(&salts, Form::Tablet, Release::Standard, PackUnit::Unit)
                .unwrap_or_else(|| unreachable!()),
            salts_label: format!("Paracetamol {mg}mg"),
        }
    }

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_pattern("50%_x\\"), "%50\\%\\_x\\\\%");
        assert_eq!(like_pattern("Dolo"), "%dolo%");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn replace_search_and_key_lookup(pool: PgPool) -> Result<(), RepoError> {
        let repo = PgOfferRepository::new(pool);
        let ja = offer(
            "ja:1",
            SourceKind::JanAushadhi,
            "Paracetamol 650",
            1500,
            10,
            650,
        );
        let dolo = offer("brand:1", SourceKind::Brand, "Dolo 650", 3400, 15, 650);
        let other = offer("brand:2", SourceKind::Brand, "Calpol 500", 2000, 15, 500);
        assert_eq!(
            repo.replace_source(SourceKind::JanAushadhi, std::slice::from_ref(&ja))
                .await?,
            1
        );
        assert_eq!(
            repo.replace_source(SourceKind::Brand, &[dolo.clone(), other.clone()])
                .await?,
            2
        );

        assert_eq!(repo.search("DOLO", 10).await?, vec![dolo.clone()]);
        assert_eq!(repo.search("paracetamol", 1).await?.len(), 1);
        assert!(repo.search("100%", 10).await?.is_empty());
        // Two-letter queries match the start of a name or salt only.
        assert_eq!(repo.search("do", 10).await?, vec![dolo.clone()]);
        assert!(repo.search("ol", 10).await?.is_empty());
        // Misspellings get the closest names instead.
        assert_eq!(repo.closest_names("dollo", 5).await?, vec!["Dolo 650"]);
        // Cheapest per unit first: JA 150 paise/tab beats Dolo ~227.
        assert_eq!(
            repo.by_key(&dolo.key).await?,
            vec![ja.clone(), dolo.clone()]
        );
        assert!(repo.key_exists(&other.key).await?);

        // Replacing one source leaves the other alone.
        assert_eq!(repo.replace_source(SourceKind::Brand, &[]).await?, 0);
        assert_eq!(repo.by_key(&dolo.key).await?, vec![ja]);
        assert!(!repo.key_exists(&other.key).await?);
        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn replace_rejects_mixed_sources_and_rolls_back(pool: PgPool) -> Result<(), RepoError> {
        let repo = PgOfferRepository::new(pool);
        let ja = offer("ja:1", SourceKind::JanAushadhi, "P", 1500, 10, 650);
        repo.replace_source(SourceKind::JanAushadhi, std::slice::from_ref(&ja))
            .await?;
        let wrong = offer("brand:1", SourceKind::Brand, "D", 3400, 15, 650);
        assert!(
            repo.replace_source(SourceKind::JanAushadhi, &[wrong])
                .await
                .is_err()
        );
        assert_eq!(repo.by_key(&ja.key).await?, vec![ja]);
        Ok(())
    }
}
