use std::{
    error::Error,
    net::{IpAddr, Ipv4Addr, SocketAddr},
};

use axum::http::StatusCode;

use crate::support::{self, BODY_LIMIT, Quota};

const PEER_PORT: u16 = 40_000;

fn peer(index: usize, salt: u8) -> Result<SocketAddr, Box<dyn Error>> {
    let index = u16::try_from(index)?;
    let [high, low] = index.to_be_bytes();
    Ok(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(10, salt, high, low)),
        PEER_PORT,
    ))
}

/// G-03: every served route is limited, body-capped and carries the security headers.
#[tokio::test]
async fn every_route_is_limited_capped_and_hardened() -> Result<(), Box<dyn Error>> {
    let composed = support::compose(&Quota::ONE{% if context.auth_enabled %}, None).await{% else %}){% endif %}?;
    support::assert_late_routes_listed(&composed.api);
    let router = &composed.api.router;
    let routes = support::inventory(&composed.api)?;
    assert!(!routes.is_empty());
    for (index, (method, path)) in routes.iter().enumerate() {
        let label = format!("{method} {path}");
        let client = peer(index, 1)?;
        let first = support::send(
            router,
            support::request(method.clone(), path, client, None, Vec::new())?,
        )
        .await?;
        support::assert_security_headers(&label, &first.headers);
        let second = support::send(
            router,
            support::request(method.clone(), path, client, None, Vec::new())?,
        )
        .await?;
        assert_eq!(
            second.status,
            StatusCode::TOO_MANY_REQUESTS,
            "{label}: second request at quota 1"
        );
        support::assert_security_headers(&label, &second.headers);

        let oversized = vec![b'x'; BODY_LIMIT + 1];
        let capped = support::send(
            router,
            support::request(method.clone(), path, peer(index, 2)?, None, oversized)?,
        )
        .await?;
        assert_eq!(
            capped.status,
            StatusCode::PAYLOAD_TOO_LARGE,
            "{label}: oversized body"
        );
        support::assert_security_headers(&label, &capped.headers);
    }
    Ok(())
}
