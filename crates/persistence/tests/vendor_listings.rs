#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::HashSet;

use nirog_application::ports::{StockSource, VendorListings};
use nirog_application::stock::{StockStatus, Vendor, VendorListing, VendorStatus};
use nirog_domain::{OfferId, Paise};
use nirog_persistence::PgVendorListings;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime, macros::datetime};

const T0: OffsetDateTime = datetime!(2026-11-20 10:00 UTC);

async fn offer(pool: &PgPool, id: &str, source: &str, key: &str) {
    sqlx::query(
        "INSERT INTO offers (id, source, name, manufacturer, price_paise, pack_count, pack_unit,
                             unit_price_millipaise, match_key, salts_label)
         VALUES ($1, $2, 'Dolo 650', 'Micro Labs', 3000, 15, 'unit', 200000, $3, 'Paracetamol')",
    )
    .bind(id)
    .bind(source)
    .bind(key)
    .execute(pool)
    .await
    .unwrap();
}

fn listing(vendor: Vendor, price: i64, at: OffsetDateTime) -> VendorListing {
    VendorListing {
        offer_id: OfferId("b1".into()),
        vendor,
        url: format!("https://{}.example/dolo-650", vendor.as_str()),
        status: StockStatus::InStock,
        price: Paise(price),
        checked_at: at,
    }
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn targets_are_brand_offers_that_have_an_exact_substitute(pool: PgPool) {
    let pcm = "tablet|standard|paracetamol:650mg|unit";
    offer(&pool, "b1", "brand", pcm).await;
    offer(&pool, "j1", "jan_aushadhi", pcm).await;
    offer(&pool, "b2", "brand", "tablet|standard|cetirizine:10mg|unit").await;
    offer(&pool, "b3", "brand", "tablet|standard|aspirin:75mg|unit").await;
    offer(&pool, "b4", "brand", "tablet|standard|aspirin:75mg|unit").await;
    offer(
        &pool,
        "j2",
        "jan_aushadhi",
        "tablet|standard|ibuprofen:400mg|unit",
    )
    .await;

    let ids: Vec<String> = PgVendorListings::new(pool)
        .price_check_targets()
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.id.0)
        .collect();
    assert_eq!(ids, ["b1", "b3", "b4"]);
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn upsert_writes_only_changes_and_reads_back_as_of_now(pool: PgPool) {
    let repo = PgVendorListings::new(pool);
    let hour = Duration::hours(1);
    let day = Duration::days(1);
    let both = |pe: i64, at| {
        [
            listing(Vendor::SourceB, pe, at),
            listing(Vendor::SourceC, 3100, at),
        ]
    };

    assert_eq!(repo.upsert(&both(3000, T0)).await.unwrap(), 2);
    // Same content an hour later: nothing to write.
    assert_eq!(repo.upsert(&both(3000, T0 + hour)).await.unwrap(), 0);
    // A day later the check time is refreshed even though nothing else changed.
    assert_eq!(repo.upsert(&both(3000, T0 + day)).await.unwrap(), 2);
    // A price change is written at once.
    assert_eq!(repo.upsert(&both(2900, T0 + day + hour)).await.unwrap(), 1);
    // An older observation never overwrites a newer one.
    assert_eq!(repo.upsert(&both(2500, T0)).await.unwrap(), 0);

    let now = T0 + day + 2 * hour;
    let b1 = OfferId("b1".into());
    let b2 = OfferId("b2".into());
    let gone = VendorListing {
        offer_id: b2.clone(),
        status: StockStatus::OutOfStock,
        ..listing(Vendor::SourceC, 4000, T0)
    };
    assert_eq!(repo.upsert(std::slice::from_ref(&gone)).await.unwrap(), 1);

    // One read answers many offers; an offer no vendor lists is absent.
    let seen = repo
        .check(&[b1.clone(), b2.clone(), OfferId("nope".into())], now)
        .await
        .unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[&b1],
        [
            VendorStatus::at(&listing(Vendor::SourceB, 2900, T0 + day + hour), now),
            VendorStatus::at(&listing(Vendor::SourceC, 3100, T0 + day), now),
        ]
    );
    assert_eq!(seen[&b1][0].price, Some(Paise(2900)));
    assert_eq!(seen[&b2], [VendorStatus::at(&gone, now)]);
    assert!(repo.check(&[], now).await.unwrap().is_empty());

    // Past the stale window the stock is unknown and no price is shown.
    let later = repo
        .check(std::slice::from_ref(&b1), now + Duration::days(8))
        .await
        .unwrap();
    assert!(
        later[&b1]
            .iter()
            .all(|s| s.status == StockStatus::Unknown && s.price.is_none())
    );

    let fresh = repo
        .fresh_urls(Vendor::SourceB, T0 + day + hour)
        .await
        .unwrap();
    assert_eq!(
        fresh,
        HashSet::from(["https://source_b.example/dolo-650".to_owned()])
    );
    assert!(
        repo.fresh_urls(Vendor::SourceC, T0 + day + hour)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn every_vendor_can_be_stored(pool: PgPool) {
    let repo = PgVendorListings::new(pool);
    let rows: Vec<_> = Vendor::ALL.iter().map(|v| listing(*v, 3000, T0)).collect();
    repo.upsert(&rows).await.unwrap();
    let got = repo.check(&[OfferId("b1".into())], T0).await.unwrap();
    assert_eq!(got[&OfferId("b1".into())].len(), Vendor::ALL.len());
}
