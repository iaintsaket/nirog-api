use utoipa::OpenApi;

use crate::dto::*;
use crate::routes;

/// The API contract. `openapi.json` at the repo root is generated from this and
/// checked in CI, and the web client is generated from that file.
#[derive(Debug, OpenApi)]
#[openapi(
    info(title = "Nirog API", version = "1.0.0", description = "Find cheaper exact substitutes for Indian medicines."),
    paths(
        routes::health,
        routes::catalog::search,
        routes::catalog::compare,
        routes::catalog::stats,
        routes::catalog::close,
        routes::catalog::pages,
        routes::auth::start,
        routes::auth::callback,
        routes::auth::logout,
        routes::auth::me,
        routes::auth::delete_me,
        routes::recent::record,
        routes::recent::clear,
        routes::saved::list,
        routes::saved::create,
        routes::saved::remove,
    ),
    components(schemas(ErrorBody, Health, SearchResponse, ComparisonResponse, StatsResponse, CloseResponse, PagesResponse, MedicinePageDto, UserDto, RecentDto, SavedMatchDto, SaveMatchRequest)),
    tags((name = "catalog"), (name = "auth"), (name = "saved"))
)]
pub struct ApiDoc;
