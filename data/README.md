# Data

`janaushadhi-products.csv` is the PMBJP (Jan Aushadhi) product basket with MRPs, 2,439 rows,
as published on 14 Oct 2025. It is public government price data. Columns: `drug_code`,
`generic_name`, `unit_size`, `mrp` (rupees), `group`.

The brand price dataset is **not** committed: it is a third-party dataset and is downloaded and
imported at runtime by `apps/importer`.
