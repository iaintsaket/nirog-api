use async_trait::async_trait;
use nirog_application::RepoError;
use nirog_application::model::NearbyGroup;
use nirog_application::ports::{NearbyOffers, OfferRepository, SourceInfo};
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

    async fn record(&self, source: &SourceInfo, offers: &[Offer]) -> Result<u64, RepoError> {
        if let Some(o) = offers.iter().find(|o| o.source != source.kind) {
            return Err(RepoError(format!(
                "offer {} is not from {}",
                o.id.0, source.id
            )));
        }
        let mut tx = self.pool.begin().await.map_err(db)?;
        sqlx::query(
            "INSERT INTO sources (id, kind, title, url, observed_on) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (id) DO UPDATE SET kind = EXCLUDED.kind, title = EXCLUDED.title,
                 url = EXCLUDED.url, observed_on = EXCLUDED.observed_on, updated_at = now()",
        )
        .bind(&source.id)
        .bind(source.kind.as_str())
        .bind(&source.title)
        .bind(&source.url)
        .bind(source.observed_on)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query(
            "CREATE TEMP TABLE incoming (
                 id text, identity text, kind text, name text, manufacturer text,
                 pack_count int, pack_unit text, match_key text, salts_label text, price_paise bigint
             ) ON COMMIT DROP",
        )
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        // Chunked UNNEST inserts: one round trip per 1,000 rows.
        for chunk in offers.chunks(1000) {
            let mut cols: [Vec<String>; 8] = Default::default();
            let (mut count, mut price) = (Vec::new(), Vec::new());
            for o in chunk {
                cols[0].push(o.id.0.clone());
                cols[1].push(o.identity());
                cols[2].push(o.source.as_str().to_owned());
                cols[3].push(o.name.clone());
                cols[4].push(o.manufacturer.clone());
                cols[5].push(o.pack.unit.as_str().to_owned());
                cols[6].push(o.key.as_str().to_owned());
                cols[7].push(o.salts_label.clone());
                count.push(
                    i32::try_from(o.pack.count)
                        .map_err(|_| RepoError("pack count too large".into()))?,
                );
                price.push(o.price.0);
            }
            let [ids, identities, kinds, names, makers, units, keys, labels] = cols;
            sqlx::query(
                "INSERT INTO incoming (id, identity, kind, name, manufacturer, pack_unit, match_key,
                                       salts_label, pack_count, price_paise)
                 SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[], $5::text[],
                                      $6::text[], $7::text[], $8::text[], $9::int[], $10::bigint[])",
            )
            .bind(ids)
            .bind(identities)
            .bind(kinds)
            .bind(names)
            .bind(makers)
            .bind(units)
            .bind(keys)
            .bind(labels)
            .bind(count)
            .bind(price)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        }
        // A listing joins an existing product with the same identity, or failing
        // that the same id (a source renamed it); otherwise it is a new product.
        let observed = |sql| sqlx::query(sql).bind(source.observed_on);
        observed("INSERT INTO products (id, identity, kind, name, manufacturer, pack_count, pack_unit,
                                   match_key, salts_label, first_seen, last_seen)
             SELECT DISTINCT ON (i.id) i.id, i.identity, i.kind, i.name, i.manufacturer, i.pack_count,
                    i.pack_unit, i.match_key, i.salts_label, $1, $1
             FROM incoming i
             WHERE NOT EXISTS (SELECT 1 FROM products p WHERE p.identity = i.identity OR p.id = i.id)
             ORDER BY i.id").execute(&mut *tx).await.map_err(db)?;
        sqlx::query(
            "CREATE TEMP TABLE matched ON COMMIT DROP AS
             SELECT DISTINCT ON (product_id) product_id, price_paise FROM (
                 SELECT COALESCE(
                            (SELECT p.id FROM products p WHERE p.identity = i.identity),
                            (SELECT p.id FROM products p WHERE p.id = i.id)) AS product_id,
                        i.price_paise, i.id
                 FROM incoming i) m
             WHERE product_id IS NOT NULL
             ORDER BY product_id, id",
        )
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        observed(
            "UPDATE products p SET first_seen = LEAST(p.first_seen, $1),
                                   last_seen = GREATEST(p.last_seen, $1)
             FROM matched m WHERE m.product_id = p.id",
        )
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        let recorded = sqlx::query(
            "INSERT INTO price_observations (product_id, source_id, observed_on, price_paise)
             SELECT product_id, $1, $2, price_paise FROM matched
             ON CONFLICT (product_id, source_id, observed_on)
             DO UPDATE SET price_paise = EXCLUDED.price_paise, imported_at = now()",
        )
        .bind(&source.id)
        .bind(source.observed_on)
        .execute(&mut *tx)
        .await
        .map_err(db)?
        .rows_affected();
        sqlx::query("INSERT INTO import_runs (source_id, observed_on, offers) VALUES ($1, $2, $3)")
            .bind(&source.id)
            .bind(source.observed_on)
            .bind(i64::try_from(recorded).map_err(|_| RepoError("too many offers".into()))?)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        // Serve the newest price of every product. Only offers that changed are
        // written, so an import leaves no dead copy of the whole table behind
        // (the database stays small). Searches see the old rows until commit.
        for step in [
            "CREATE TEMP TABLE fresh ON COMMIT DROP AS
             SELECT DISTINCT ON (p.id) p.id, p.kind AS source, p.name, p.manufacturer,
                    o.price_paise, p.pack_count, p.pack_unit,
                    o.price_paise * 1000 / p.pack_count AS unit_price_millipaise,
                    p.match_key, p.salts_label
             FROM products p JOIN price_observations o ON o.product_id = p.id
             ORDER BY p.id, o.observed_on DESC, o.imported_at DESC",
            "DELETE FROM offers o WHERE NOT EXISTS (SELECT 1 FROM fresh f WHERE f.id = o.id)",
            "INSERT INTO offers (id, source, name, manufacturer, price_paise, pack_count, pack_unit,
                                 unit_price_millipaise, match_key, salts_label)
             SELECT * FROM fresh
             ON CONFLICT (id) DO UPDATE SET
                 source = EXCLUDED.source, name = EXCLUDED.name,
                 manufacturer = EXCLUDED.manufacturer, price_paise = EXCLUDED.price_paise,
                 pack_count = EXCLUDED.pack_count, pack_unit = EXCLUDED.pack_unit,
                 unit_price_millipaise = EXCLUDED.unit_price_millipaise,
                 match_key = EXCLUDED.match_key, salts_label = EXCLUDED.salts_label,
                 imported_at = now()
             WHERE (offers.source, offers.name, offers.manufacturer, offers.price_paise,
                    offers.pack_count, offers.pack_unit, offers.match_key, offers.salts_label)
                   IS DISTINCT FROM
                   (EXCLUDED.source, EXCLUDED.name, EXCLUDED.manufacturer, EXCLUDED.price_paise,
                    EXCLUDED.pack_count, EXCLUDED.pack_unit, EXCLUDED.match_key,
                    EXCLUDED.salts_label)",
        ] {
            sqlx::query(step).execute(&mut *tx).await.map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(recorded)
    }
}

/// Cheapest offer per formula for formulas sharing a salt with `key` (same form, release, unit).
const NEARBY: &str = r#"
SELECT DISTINCT ON (match_key)
       id, source, name, manufacturer, price_paise, pack_count, pack_unit, match_key, salts_label,
       count(*) OVER (PARTITION BY match_key) AS n
FROM offers
WHERE match_key LIKE $1 ESCAPE '\' AND match_key LIKE $2 ESCAPE '\' AND match_key <> $3
  AND match_key LIKE ANY($4)
ORDER BY match_key, unit_price_millipaise, id
LIMIT $5
"#;

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[async_trait]
impl NearbyOffers for PgOfferRepository {
    async fn nearby(&self, key: &MatchKey, limit: u32) -> Result<Vec<NearbyGroup>, RepoError> {
        let parts: Vec<&str> = key.as_str().split('|').collect();
        let [form, release, salts, unit] = parts[..] else {
            return Ok(vec![]);
        };
        let names: Vec<String> = salts
            .split('+')
            .filter_map(|t| t.split_once(':'))
            .flat_map(|(n, _)| {
                let n = like_escape(n);
                [format!("%|{n}:%"), format!("%+{n}:%")]
            })
            .collect();
        let rows = sqlx::query(NEARBY)
            .bind(format!("{}|{}|%", like_escape(form), like_escape(release)))
            .bind(format!("%|{}", like_escape(unit)))
            .bind(key.as_str())
            .bind(&names)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.iter()
            .map(|r| {
                let cheapest = to_offer(r)?;
                let n: i64 = r.try_get("n").map_err(db)?;
                Ok(NearbyGroup {
                    key: cheapest.key.clone(),
                    cheapest,
                    offers: u64::try_from(n).unwrap_or(0),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nirog_domain::{Form, Release, Salt};
    use time::{Date, macros::date};

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

    fn src(id: &str, kind: SourceKind, observed_on: Date) -> SourceInfo {
        SourceInfo {
            id: id.into(),
            kind,
            title: id.into(),
            url: None,
            observed_on,
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn record_search_and_key_lookup(pool: PgPool) -> Result<(), RepoError> {
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
        let ja_src = src("ja", SourceKind::JanAushadhi, date!(2026 - 11 - 20));
        let old = src("brands-2022", SourceKind::Brand, date!(2022 - 11 - 17));
        assert_eq!(repo.record(&ja_src, std::slice::from_ref(&ja)).await?, 1);
        assert_eq!(repo.record(&old, &[dolo.clone(), other.clone()]).await?, 2);

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
        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn newest_price_wins_and_products_are_never_dropped(
        pool: PgPool,
    ) -> Result<(), RepoError> {
        let repo = PgOfferRepository::new(pool);
        let old = src("brands-2022", SourceKind::Brand, date!(2022 - 11 - 17));
        let new = src("onemg-2025", SourceKind::Brand, date!(2025 - 10 - 22));
        let dolo = offer("brand:1", SourceKind::Brand, "Dolo 650", 3400, 15, 650);
        let calpol = offer("brand:2", SourceKind::Brand, "Calpol 500", 2000, 15, 500);
        repo.record(&old, &[dolo.clone(), calpol.clone()]).await?;

        // The 2025 list spells Dolo differently, has a new price and lacks Calpol.
        let mut dolo_2025 = offer("onemg:9", SourceKind::Brand, "DOLO-650", 3213, 15, 650);
        dolo_2025.manufacturer = "maker.".into();
        repo.record(&new, &[dolo_2025]).await?;
        let mut expected = dolo.clone();
        expected.price = Paise(3213);
        assert_eq!(repo.by_key(&dolo.key).await?, vec![expected.clone()]);
        assert_eq!(repo.by_key(&calpol.key).await?, vec![calpol.clone()]);

        // Re-importing the older list does not bring back its older price.
        repo.record(&old, std::slice::from_ref(&dolo)).await?;
        assert_eq!(repo.by_key(&dolo.key).await?, vec![expected]);
        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn reimport_rewrites_only_changed_offers(pool: PgPool) -> Result<(), RepoError> {
        // Rewriting every offer on each import leaves a dead copy of the whole table behind.
        let versions = |pool: PgPool| async move {
            sqlx::query_as::<_, (String, String)>("SELECT id, xmin::text FROM offers ORDER BY id")
                .fetch_all(&pool)
                .await
                .map_err(db)
        };
        let repo = PgOfferRepository::new(pool.clone());
        let ja_src = src("ja", SourceKind::JanAushadhi, date!(2026 - 11 - 20));
        let a = offer("ja:1", SourceKind::JanAushadhi, "P 650", 1500, 10, 650);
        let b = offer("ja:2", SourceKind::JanAushadhi, "P 500", 1200, 10, 500);
        repo.record(&ja_src, &[a.clone(), b.clone()]).await?;
        let before = versions(pool.clone()).await?;

        let mut b_cheaper = b.clone();
        b_cheaper.price = Paise(900);
        repo.record(&ja_src, &[a.clone(), b_cheaper.clone()])
            .await?;
        let after = versions(pool.clone()).await?;
        assert_eq!(after[0], before[0], "unchanged offer was rewritten");
        assert_ne!(after[1], before[1], "changed offer was not rewritten");
        assert_eq!(repo.by_key(&b.key).await?, vec![b_cheaper]);
        assert_eq!(repo.by_key(&a.key).await?, vec![a]);
        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn record_rejects_mixed_sources_and_rolls_back(pool: PgPool) -> Result<(), RepoError> {
        let repo = PgOfferRepository::new(pool);
        let ja_src = src("ja", SourceKind::JanAushadhi, date!(2026 - 11 - 20));
        let ja = offer("ja:1", SourceKind::JanAushadhi, "P", 1500, 10, 650);
        repo.record(&ja_src, std::slice::from_ref(&ja)).await?;
        let mut cheaper = ja.clone();
        cheaper.price = Paise(900);
        let wrong = offer("brand:1", SourceKind::Brand, "D", 3400, 15, 650);
        assert!(repo.record(&ja_src, &[cheaper, wrong]).await.is_err());
        assert_eq!(repo.by_key(&ja.key).await?, vec![ja]);
        Ok(())
    }
}
