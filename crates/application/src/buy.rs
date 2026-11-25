//! Buy links. Every link to an online pharmacy goes through our own redirect,
//! `/v1/go/{offer}/{source}`, to a destination kept only in the database, and the
//! sources' display names live only in the database too.

use async_trait::async_trait;
use nirog_domain::{Offer, OfferId, SourceKind};

use crate::error::RepoError;
use crate::model::{BuyLink, LinkKind};
use crate::ports::SourceProfiles;
use crate::stock::{Vendor, VendorStatus};

/// A price source's public face, as stored in `price_sources`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceProfile {
    pub vendor: Vendor,
    pub display_name: String,
    /// An `https://` search page with `{query}` where the medicine name goes.
    pub search_url_template: Option<String>,
}

/// What the redirect knows about one offer at one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyTarget {
    pub offer_name: String,
    /// The product page the last price check saw, if any.
    pub listing_url: Option<String>,
    pub search_url_template: Option<String>,
}

/// How a reader sees one source's listing of an offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLink {
    pub display_name: String,
    /// Our redirect to the source, when it has somewhere to send the buyer.
    pub buy_path: Option<String>,
}

/// No stored sources: names fall back to "Pharmacy A".."Pharmacy D", and no search links.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSources;

#[async_trait]
impl SourceProfiles for NoSources {
    async fn profiles(&self) -> Result<Vec<SourceProfile>, RepoError> {
        Ok(Vec::new())
    }
}

/// The stored display name, else "Pharmacy A".."Pharmacy D" from the source id.
pub fn display_name(vendor: Vendor, profiles: &[SourceProfile]) -> String {
    match profiles.iter().find(|p| p.vendor == vendor) {
        Some(p) => p.display_name.clone(),
        None => {
            let letter = match vendor {
                Vendor::SourceA => 'A',
                Vendor::SourceB => 'B',
                Vendor::SourceC => 'C',
                Vendor::SourceD => 'D',
            };
            format!("Pharmacy {letter}")
        }
    }
}

/// Our redirect for `offer` at `vendor`.
pub fn buy_path(offer: &OfferId, vendor: Vendor) -> String {
    format!("/v1/go/{}/{}", encode(&offer.0), vendor.as_str())
}

/// Where to send a buyer: the listing's page if it is `https`, else the source's search for the
/// offer's name, else nowhere. Only stored values are used, so this is never an open redirect.
pub fn destination(target: &BuyTarget) -> Option<String> {
    target
        .listing_url
        .as_deref()
        .and_then(https)
        .or_else(|| search_url(target.search_url_template.as_deref()?, &target.offer_name))
}

/// The reader-facing name and redirect of each of `statuses`, plus search links for a brand
/// at every source that has a search page but no listing of it.
pub fn for_offer(
    offer: &Offer,
    statuses: &[VendorStatus],
    profiles: &[SourceProfile],
) -> (Vec<SourceLink>, Vec<BuyLink>) {
    let searchable = |vendor: Vendor| {
        profiles.iter().any(|p| {
            p.vendor == vendor
                && p.search_url_template
                    .as_deref()
                    .and_then(|t| search_url(t, &offer.name))
                    .is_some()
        })
    };
    let entries = statuses
        .iter()
        .map(|s| SourceLink {
            display_name: display_name(s.vendor, profiles),
            buy_path: (s.has_page || searchable(s.vendor)).then(|| buy_path(&offer.id, s.vendor)),
        })
        .collect();
    let links = if offer.source == SourceKind::Brand {
        profiles
            .iter()
            .filter(|p| !statuses.iter().any(|s| s.vendor == p.vendor) && searchable(p.vendor))
            .map(|p| BuyLink {
                label: p.display_name.clone(),
                url: buy_path(&offer.id, p.vendor),
                kind: LinkKind::Online,
            })
            .collect()
    } else {
        Vec::new()
    };
    (entries, links)
}

/// `template` with `{query}` replaced by the percent-encoded `name`, if the result is `https`.
fn search_url(template: &str, name: &str) -> Option<String> {
    https(&template.replace("{query}", &encode(name)))
}

/// Percent-encodes everything but RFC 3986 unreserved characters.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The URL if it is a well-formed `https` URL with a host, normalised so it is safe in a header.
pub(crate) fn https(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    (url.scheme() == "https" && url.host_str().is_some_and(|h| !h.is_empty())).then(|| url.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::offer;
    use crate::stock::StockStatus;
    use time::macros::datetime;

    fn target(listing: Option<&str>, template: Option<&str>) -> BuyTarget {
        BuyTarget {
            offer_name: "Dolo 650 Tablet".into(),
            listing_url: listing.map(Into::into),
            search_url_template: template.map(Into::into),
        }
    }

    const SEARCH: &str = "https://shop.example/search?q={query}";

    #[test]
    fn destination_is_an_https_listing_else_the_stored_search() {
        let page = "https://shop.example/p/dolo-650";
        assert_eq!(
            destination(&target(Some(page), Some(SEARCH))).as_deref(),
            Some(page)
        );
        // A stored non-https page is refused; the search page stands in.
        assert_eq!(
            destination(&target(Some("http://shop.example/p/1"), Some(SEARCH))).as_deref(),
            Some("https://shop.example/search?q=Dolo%20650%20Tablet")
        );
        for bad in ["http://shop.example/p/1", "javascript:alert(1)", "https://"] {
            assert_eq!(destination(&target(Some(bad), None)), None, "{bad}");
        }
        assert_eq!(destination(&target(None, None)), None);
    }

    #[test]
    fn listings_get_names_and_redirects_and_brands_get_search_links() {
        let status = |vendor, has_page| VendorStatus {
            vendor,
            status: StockStatus::InStock,
            price: None,
            checked_at: datetime!(2026-11-25 0:00 UTC),
            has_page,
        };
        let profiles = [
            SourceProfile {
                vendor: Vendor::SourceA,
                display_name: "Pharmacy One".into(),
                search_url_template: Some(SEARCH.into()),
            },
            SourceProfile {
                vendor: Vendor::SourceB,
                display_name: "Pharmacy Two".into(),
                search_url_template: None,
            },
        ];
        let brand = offer("brand:1", SourceKind::Brand, "Dolo 650 Tablet", 3091, "k");
        let (entries, links) = for_offer(
            &brand,
            &[
                status(Vendor::SourceB, true),
                status(Vendor::SourceD, false),
            ],
            &profiles,
        );
        assert_eq!(
            entries,
            [
                SourceLink {
                    display_name: "Pharmacy Two".into(),
                    buy_path: Some("/v1/go/brand%3A1/source_b".into()),
                },
                // No stored name and no usable page: a placeholder name and no link.
                SourceLink {
                    display_name: "Pharmacy D".into(),
                    buy_path: None,
                },
            ]
        );
        assert_eq!(
            links,
            [BuyLink {
                label: "Pharmacy One".into(),
                url: "/v1/go/brand%3A1/source_a".into(),
                kind: LinkKind::Online,
            }]
        );
        // A listed source is not linked twice; Jan Aushadhi is never sold online.
        let (_, listed) = for_offer(&brand, &[status(Vendor::SourceA, false)], &profiles);
        assert!(listed.is_empty());
        let ja = offer("ja:1", SourceKind::JanAushadhi, "Paracetamol", 1500, "k");
        assert!(for_offer(&ja, &[], &profiles).1.is_empty());
    }
}
