#![allow(clippy::unwrap_used, clippy::panic)]

use nirog_application::stock::{STALE_AFTER, StockStatus, Vendor, VendorListing, VendorStatus};
use nirog_domain::{OfferId, Paise};
use time::{Duration, macros::datetime};

fn listing(status: StockStatus) -> VendorListing {
    VendorListing {
        offer_id: OfferId("b25:abcd".into()),
        vendor: Vendor::SourceB,
        url: "https://source-b.example/x-1".into(),
        status,
        price: Paise(2518),
        checked_at: datetime!(2026-11-20 10:00 UTC),
    }
}

#[test]
fn vendor_and_status_round_trip_through_their_db_text() {
    for v in Vendor::ALL {
        assert_eq!(Vendor::parse(v.as_str()).unwrap(), v);
    }
    // Stored in `vendor_listings` and sent to the web as is.
    assert_eq!(
        Vendor::ALL.map(Vendor::as_str),
        ["source_a", "source_b", "source_c", "source_d"]
    );
    for s in [
        StockStatus::InStock,
        StockStatus::OutOfStock,
        StockStatus::Unknown,
    ] {
        assert_eq!(StockStatus::parse(s.as_str()).unwrap(), s);
    }
    assert!(Vendor::parse("source_z").is_err());
    assert!(StockStatus::parse("maybe").is_err());
}

#[test]
fn a_fresh_listing_keeps_its_status_and_price() {
    let l = listing(StockStatus::OutOfStock);
    let s = VendorStatus::at(&l, l.checked_at + STALE_AFTER);
    assert_eq!(s.status, StockStatus::OutOfStock);
    assert_eq!(s.price, Some(Paise(2518)));
}

#[test]
fn a_listing_older_than_seven_days_reads_unknown_without_a_price() {
    let l = listing(StockStatus::InStock);
    let s = VendorStatus::at(&l, l.checked_at + STALE_AFTER + Duration::seconds(1));
    assert_eq!(s.status, StockStatus::Unknown);
    assert_eq!(s.price, None);
    assert_eq!(s.checked_at, l.checked_at);
}

#[test]
fn a_one_area_source_alone_never_hides_a_brand() {
    // Source A answers for one delivery area, so its "out of stock" needs a second opinion.
    use nirog_application::stock::sold_out_everywhere;
    let st = |vendor, status| VendorStatus {
        vendor,
        status,
        price: None,
        checked_at: datetime!(2026-11-25 0:00 UTC),
    };
    let out = StockStatus::OutOfStock;
    assert!(!sold_out_everywhere(&[st(Vendor::SourceA, out)]));
    assert!(sold_out_everywhere(&[
        st(Vendor::SourceA, out),
        st(Vendor::SourceC, out)
    ]));
    assert!(sold_out_everywhere(&[st(Vendor::SourceB, out)]));
    assert!(!sold_out_everywhere(&[
        st(Vendor::SourceB, out),
        st(Vendor::SourceD, StockStatus::Unknown)
    ]));
}
