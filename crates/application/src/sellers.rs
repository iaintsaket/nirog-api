use nirog_domain::{Offer, SourceKind};
use url::form_urlencoded::byte_serialize;

use crate::model::{BuyLink, LinkKind};
use crate::ports::SellerLinks;

const JAN_AUSHADHI_STORES: &str = "https://janaushadhi.gov.in/";
const SUGAM_APP: &str = "https://apps.apple.com/in/app/jan-aushadhi-sugam-pmbi/id1476574620";

/// Jan Aushadhi → official store finders only (online sale is not permitted).
/// Brands → search pages of licensed online pharmacies.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultSellerLinks;

fn path_segment(s: &str) -> String {
    byte_serialize(s.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

impl SellerLinks for DefaultSellerLinks {
    fn links(&self, offer: &Offer) -> Vec<BuyLink> {
        match offer.source {
            SourceKind::JanAushadhi => vec![
                BuyLink {
                    label: "Find a Jan Aushadhi Kendra",
                    url: JAN_AUSHADHI_STORES.into(),
                    kind: LinkKind::Store,
                },
                BuyLink {
                    label: "Jan Aushadhi Sugam app",
                    url: SUGAM_APP.into(),
                    kind: LinkKind::Store,
                },
            ],
            SourceKind::Brand => {
                let q = path_segment(&offer.name);
                vec![
                    BuyLink {
                        label: "Tata 1mg",
                        url: format!("https://www.1mg.com/search/all?name={q}"),
                        kind: LinkKind::Online,
                    },
                    BuyLink {
                        label: "PharmEasy",
                        url: format!("https://pharmeasy.in/search/all?name={q}"),
                        kind: LinkKind::Online,
                    },
                    BuyLink {
                        label: "Netmeds",
                        url: format!("https://www.netmeds.com/catalogsearch/result/{q}/all"),
                        kind: LinkKind::Online,
                    },
                    BuyLink {
                        label: "Apollo Pharmacy",
                        url: format!("https://www.apollopharmacy.in/search-medicines/{q}"),
                        kind: LinkKind::Online,
                    },
                ]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::offer;

    #[test]
    fn jan_aushadhi_gets_store_links_only() {
        let links = DefaultSellerLinks.links(&offer(
            "ja-1",
            SourceKind::JanAushadhi,
            "Atorvastatin Tablets IP 10mg",
            908,
            "k",
        ));
        assert!(links.iter().all(|l| l.kind == LinkKind::Store));
        assert_eq!(links.len(), 2);
    }

    #[test]
    fn brands_get_encoded_pharmacy_searches() {
        let links = DefaultSellerLinks.links(&offer(
            "b",
            SourceKind::Brand,
            "Augmentin 625 Duo/Tablet&x",
            1,
            "k",
        ));
        assert_eq!(
            links[0].url,
            "https://www.1mg.com/search/all?name=Augmentin%20625%20Duo%2FTablet%26x"
        );
        assert_eq!(
            links[2].url,
            "https://www.netmeds.com/catalogsearch/result/Augmentin%20625%20Duo%2FTablet%26x/all"
        );
        assert!(
            links
                .iter()
                .all(|l| l.kind == LinkKind::Online && l.url.starts_with("https://"))
        );
    }
}
