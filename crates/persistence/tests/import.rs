#![allow(clippy::unwrap_used)]

use nirog_application::ports::{OfferRepository, SourceInfo};
use nirog_domain::parse::{describe_salts, detect_form, detect_release};
use nirog_domain::{MatchKey, Offer, OfferId, Pack, PackUnit, Paise, Salt, SourceKind, Strength};
use nirog_persistence::PgOfferRepository;
use sqlx::PgPool;

fn dettol_generic(salts: &[(&str, &str)]) -> Offer {
    let salts: Vec<Salt> = salts
        .iter()
        .map(|(name, pct)| Salt {
            name: (*name).into(),
            strength: Some(Strength::Text((*pct).into())),
        })
        .collect();
    Offer {
        id: OfferId("ja:2516".into()),
        source: SourceKind::JanAushadhi,
        name: "Chloroxylenol 4.80% w/v, Terpineol 9% v/v, Absolute Alcohol 13.10% v/v Liquid"
            .into(),
        manufacturer: "PMBI (Jan Aushadhi)".into(),
        price: Paise(1875),
        pack: Pack {
            count: 60,
            unit: PackUnit::Ml,
        },
        key: MatchKey::new(
            &salts,
            detect_form("liquid"),
            detect_release("liquid"),
            PackUnit::Ml,
        )
        .unwrap(),
        salts_label: describe_salts(&salts),
    }
}

/// A parser fix reaches products already in the catalogue on the next import.
#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn reimport_rekeys_existing_products(pool: PgPool) {
    let repo = PgOfferRepository::new(pool);
    let source = SourceInfo {
        id: "jan-aushadhi".into(),
        kind: SourceKind::JanAushadhi,
        title: "jan-aushadhi".into(),
        url: None,
        observed_on: time::macros::date!(2026 - 11 - 20),
    };
    let old = dettol_generic(&[
        ("chloroxylenol", "4.8%"),
        ("terpineol v v", "9%"),
        ("absolute alcohol v v liquid", "13.1%"),
    ]);
    let new = dettol_generic(&[
        ("chloroxylenol", "4.8%"),
        ("terpineol", "9%"),
        ("absolute alcohol", "13.1%"),
    ]);
    repo.record(&source, std::slice::from_ref(&old))
        .await
        .unwrap();
    repo.record(&source, std::slice::from_ref(&new))
        .await
        .unwrap();

    let found = repo.by_key(&new.key).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, new.id);
    assert_eq!(found[0].salts_label, new.salts_label);
    assert!(!repo.key_exists(&old.key).await.unwrap(), "old key is gone");
}
