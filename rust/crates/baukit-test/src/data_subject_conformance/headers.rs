use axum::http::{HeaderMap, header};

pub(super) fn valid_discovery_cache(headers: &HeaderMap) -> bool {
    let mut max_age = None;
    for header in headers.get_all(header::CACHE_CONTROL) {
        let Ok(value) = header.to_str() else {
            return false;
        };
        for directive in value.split(',') {
            let (name, value) = directive
                .trim()
                .split_once('=')
                .map_or((directive.trim(), None), |(name, value)| {
                    (name.trim(), Some(value.trim().trim_matches('"')))
                });
            if ["no-store", "no-cache", "private"]
                .iter()
                .any(|v| name.eq_ignore_ascii_case(v))
            {
                return false;
            }
            if name.eq_ignore_ascii_case("max-age") {
                if max_age.is_some() {
                    return false;
                }
                let Some(parsed) = value.and_then(|value| value.parse::<u64>().ok()) else {
                    return false;
                };
                max_age = Some(parsed);
            }
            if name.eq_ignore_ascii_case("s-maxage") && value != Some("300") {
                return false;
            }
        }
    }
    max_age == Some(300)
}

pub(super) fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim() == "application/json")
        })
}
