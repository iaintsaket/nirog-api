use nirog_domain::{Offer, SourceKind};

use crate::model::{BuyLink, LinkKind};
use crate::ports::SellerLinks;

const JAN_AUSHADHI_STORES: &str = "https://janaushadhi.gov.in/";
const SUGAM_APP: &str = "https://apps.apple.com/in/app/jan-aushadhi-sugam-pmbi/id1476574620";

/// Jan Aushadhi → official store finders only (online sale is not permitted).
/// Brands → no links.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultSellerLinks;

impl SellerLinks for DefaultSellerLinks {
    fn links(&self, offer: &Offer) -> Vec<BuyLink> {
        match offer.source {
            SourceKind::JanAushadhi => vec![
                BuyLink {
                    label: "Find a Jan Aushadhi Kendra".into(),
                    url: JAN_AUSHADHI_STORES.into(),
                    kind: LinkKind::Store,
                },
                BuyLink {
                    label: "Jan Aushadhi Sugam app".into(),
                    url: SUGAM_APP.into(),
                    kind: LinkKind::Store,
                },
            ],
            SourceKind::Brand => Vec::new(),
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
    fn brands_get_no_links() {
        let links =
            DefaultSellerLinks.links(&offer("b", SourceKind::Brand, "Augmentin 625", 1, "k"));
        assert!(links.is_empty());
    }
}
