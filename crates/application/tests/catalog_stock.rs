#![allow(clippy::unwrap_used, clippy::panic)]
//! Online stock in compare and search: shown per offer, and brands every pharmacy has sold out
//! of are hidden.

use std::collections::HashMap;
use std::sync::Arc;

use StockStatus::{InStock, OutOfStock, Unknown};
use async_trait::async_trait;
use nirog_application::BrandAlias;
use nirog_application::RepoError;
use nirog_application::ports::{
    BrandAliases, OfferRepository, SourceInfo, StockSource, SystemClock,
};
use nirog_application::sellers::DefaultSellerLinks;
use nirog_application::services::CatalogService;
use nirog_application::services::catalog::ComparisonView;
use nirog_application::stock::{StockStatus, Vendor, VendorStatus};
use nirog_domain::{
    Form, MatchKey, Offer, OfferId, Pack, PackUnit, Paise, Release, Salt, SourceKind,
};
use time::OffsetDateTime;

fn key(name: &str) -> MatchKey {
    MatchKey::new(
        &[Salt::mg(name, 1000)],
        Form::Tablet,
        Release::Standard,
        PackUnit::Unit,
    )
    .unwrap()
}

fn offer(id: &str, source: SourceKind, name: &str, price: i64, key_name: &str) -> Offer {
    Offer {
        id: OfferId(id.into()),
        source,
        name: name.into(),
        manufacturer: "Maker".into(),
        price: Paise(price),
        pack: Pack {
            count: 10,
            unit: PackUnit::Unit,
        },
        key: key(key_name),
        salts_label: format!("{key_name} 2mg"),
    }
}

struct Offers(Vec<Offer>);

#[async_trait]
impl OfferRepository for Offers {
    async fn search(&self, query: &str, _limit: u32) -> Result<Vec<Offer>, RepoError> {
        let q = query.to_lowercase();
        Ok(self
            .0
            .iter()
            .filter(|o| o.name.to_lowercase().contains(&q))
            .cloned()
            .collect())
    }
    async fn closest_names(&self, _q: &str, _limit: u32) -> Result<Vec<String>, RepoError> {
        Ok(Vec::new())
    }
    async fn by_key(&self, key: &MatchKey) -> Result<Vec<Offer>, RepoError> {
        Ok(self.0.iter().filter(|o| &o.key == key).cloned().collect())
    }
    async fn key_exists(&self, key: &MatchKey) -> Result<bool, RepoError> {
        Ok(self.0.iter().any(|o| &o.key == key))
    }
    async fn record(&self, _s: &SourceInfo, _o: &[Offer]) -> Result<u64, RepoError> {
        Ok(0)
    }
}

struct NoAliases;
impl BrandAliases for NoAliases {
    fn find(&self, _query: &str) -> Option<BrandAlias> {
        None
    }
}

struct Stock(HashMap<OfferId, Vec<VendorStatus>>);

#[async_trait]
impl StockSource for Stock {
    async fn check(
        &self,
        offers: &[OfferId],
        _now: OffsetDateTime,
    ) -> Result<HashMap<OfferId, Vec<VendorStatus>>, RepoError> {
        Ok(offers
            .iter()
            .filter_map(|id| self.0.get(id).map(|s| (id.clone(), s.clone())))
            .collect())
    }
}

fn at(vendor: Vendor, status: StockStatus) -> VendorStatus {
    VendorStatus {
        vendor,
        status,
        price: Some(Paise(1_000)),
        checked_at: time::macros::datetime!(2026-11-25 0:00 UTC),
    }
}

fn atorvastatin() -> Vec<Offer> {
    vec![
        offer("b1", SourceKind::Brand, "Lipitor 10 Tablet", 20_000, "k1"),
        offer("b2", SourceKind::Brand, "Atorva 10 Tablet", 10_000, "k1"),
        offer(
            "j1",
            SourceKind::JanAushadhi,
            "Atorvastatin Tablets IP 10mg",
            908,
            "k1",
        ),
        offer("b3", SourceKind::Brand, "Dolo 650 Tablet", 3_000, "k2"),
    ]
}

/// `entries`: offer id, source B status, source C status.
fn stocked(offers: Vec<Offer>, entries: &[(&str, StockStatus, StockStatus)]) -> CatalogService {
    let stock = entries
        .iter()
        .map(|(id, pe, nm)| {
            (
                OfferId((*id).into()),
                vec![at(Vendor::SourceB, *pe), at(Vendor::SourceC, *nm)],
            )
        })
        .collect();
    CatalogService::new(
        Arc::new(Offers(offers)),
        Arc::new(DefaultSellerLinks),
        Arc::new(NoAliases),
    )
    .with_stock(Arc::new(Stock(stock)), Arc::new(SystemClock))
}

fn ids(view: &ComparisonView) -> Vec<&str> {
    view.comparison
        .offers
        .iter()
        .map(|o| o.id.0.as_str())
        .collect()
}

#[tokio::test]
async fn compare_carries_stock_and_hides_brands_sold_out_everywhere() {
    let k1 = key("k1");
    let svc = stocked(
        atorvastatin(),
        &[("b1", InStock, OutOfStock), ("b2", OutOfStock, OutOfStock)],
    );
    let view = svc.compare(k1.as_str(), None).await.unwrap();
    assert_eq!(ids(&view), ["j1", "b1"]);
    assert_eq!(view.total, 2);
    assert_eq!(view.links.len(), 2);
    assert_eq!(view.stock.len(), 2);
    assert!(view.stock[0].is_empty());
    assert_eq!(
        view.stock[1],
        [
            at(Vendor::SourceB, InStock),
            at(Vendor::SourceC, OutOfStock)
        ]
    );

    // The medicine the user searched for is always shown, with its stock.
    let searched = svc.compare(k1.as_str(), Some("b2")).await.unwrap();
    assert_eq!(ids(&searched), ["j1", "b2", "b1"]);
    assert_eq!(searched.comparison.searched, Some(1));
    assert_eq!(searched.stock[1][0].status, OutOfStock);
}

#[tokio::test]
async fn unknown_stock_never_hides_and_all_sold_out_shows_all() {
    let k1 = key("k1");
    let maybe = stocked(atorvastatin(), &[("b2", OutOfStock, Unknown)]);
    assert_eq!(
        ids(&maybe.compare(k1.as_str(), None).await.unwrap()),
        ["j1", "b2", "b1"]
    );

    let brands_only = vec![
        offer("b4", SourceKind::Brand, "Crocin 650", 2_000, "k3"),
        offer("b5", SourceKind::Brand, "Calpol 650", 3_000, "k3"),
    ];
    let gone = stocked(
        brands_only,
        &[
            ("b4", OutOfStock, OutOfStock),
            ("b5", OutOfStock, OutOfStock),
        ],
    );
    let view = gone.compare(key("k3").as_str(), None).await.unwrap();
    assert_eq!(ids(&view), ["b4", "b5"]);
    assert_eq!(view.total, 2);
    assert_eq!(view.stock.len(), 2);
}

#[tokio::test]
async fn search_hides_brands_sold_out_everywhere() {
    let svc = stocked(atorvastatin(), &[("b2", OutOfStock, OutOfStock)]);
    let groups = svc.search("tablet").await.unwrap();
    let k1: Vec<&str> = groups[0].matched.iter().map(|o| o.id.0.as_str()).collect();
    assert_eq!(k1, ["b1", "j1"]);

    // A group whose every match is sold out still shows, rather than vanishing.
    let only = stocked(atorvastatin(), &[("b3", OutOfStock, OutOfStock)]);
    let groups = only.search("dolo").await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].matched[0].id.0, "b3");
}
