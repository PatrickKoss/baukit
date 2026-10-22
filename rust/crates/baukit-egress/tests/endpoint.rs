use std::{
    collections::VecDeque,
    error::Error,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use baukit_egress::{
    AddressPolicy, AllowedEndpoint, DestinationRejection, EgressError, EgressOptions,
    EgressRequest, GuardedClient, PrivateNetworks, ResolveFuture, Resolver, resolve_destination,
    validate_destination,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;

fn policy(host: &str, port: u16, http: bool) -> AddressPolicy {
    AddressPolicy::OnlyEndpoint(AllowedEndpoint::new(host, port, http).expect("endpoint"))
}

#[test]
fn endpoint_allowances_reject_wildcards_urls_and_reserved_literals() {
    for host in [
        "*",
        "*.internal",
        "https://proxy.internal",
        "proxy.internal:4000",
        "user@proxy.internal",
        "proxy.internal/path",
        "proxy.internal?secret",
        "proxy.internal#fragment",
        "proxy.internal.",
        "",
        "169.254.169.254",
        "[fd00:ec2::254]",
        "[fd20:ce::254]",
        "100.100.100.200",
        "[fe80::1]",
        "[::ffff:169.254.169.254]",
        "[64:ff9b::a9fe:a9fe]",
        "0.0.0.0",
        "[::]",
        "224.0.0.1",
        "192.0.2.1",
    ] {
        assert!(AllowedEndpoint::new(host, 4000, true).is_err(), "{host}");
    }
    assert!(AllowedEndpoint::new("proxy.internal", 0, false).is_err());
}

#[test]
fn endpoint_policy_requires_exact_host_port_and_separate_http_opt_in() -> Result<(), Box<dyn Error>>
{
    let allowed = policy("PROXY.internal", 4000, false);
    assert!(
        validate_destination(
            &Url::parse("https://proxy.internal:4000/v1/chat/completions")?,
            &allowed
        )
        .is_ok()
    );
    for url in [
        "http://proxy.internal:4000/v1/chat/completions",
        "https://proxy.internal:4001/v1/chat/completions",
        "https://other.internal:4000",
        "https://proxy.internal.evil:4000",
        "https://8.8.8.8:4000",
        "https://proxy.internal:4000/#fragment",
        "https://secret@proxy.internal:4000",
    ] {
        assert!(
            validate_destination(&Url::parse(url)?, &allowed).is_err(),
            "{url}"
        );
    }
    for (host, url) in [
        ("127.0.0.1", "http://127.0.0.1:4000/v1"),
        ("[::1]", "http://[::1]:4000/v1"),
        ("10.0.0.3", "http://10.0.0.3:4000/v1"),
        ("[fd12::1]", "http://[fd12::1]:4000/v1"),
    ] {
        assert!(validate_destination(&Url::parse(url)?, &policy(host, 4000, true)).is_ok());
        assert!(validate_destination(&Url::parse(url)?, &AddressPolicy::PublicOnly).is_err());
    }
    assert!(
        validate_destination(&Url::parse("https://8.8.8.8")?, &AddressPolicy::PublicOnly).is_ok()
    );
    Ok(())
}

struct ChangingResolver(Mutex<VecDeque<Vec<IpAddr>>>);
impl ChangingResolver {
    fn new(answers: &[&[&str]]) -> Self {
        Self(Mutex::new(
            answers
                .iter()
                .map(|set| set.iter().map(|ip| ip.parse().expect("address")).collect())
                .collect(),
        ))
    }
}
impl Resolver for ChangingResolver {
    fn resolve<'a>(&'a self, _: &'a str) -> ResolveFuture<'a> {
        Box::pin(async {
            Ok(self
                .0
                .lock()
                .expect("answers")
                .pop_front()
                .expect("expected lookup"))
        })
    }
}

#[tokio::test]
async fn endpoint_resolver_rejects_unlisted_hosts_before_dns() {
    let resolver = ChangingResolver::new(&[]);
    for host in ["unlisted.internal", "proxy.internal.evil", "8.8.8.8"] {
        assert!(matches!(
            resolve_destination(
                &resolver,
                host,
                &policy("proxy.internal", 4000, true),
                Duration::from_secs(1)
            )
            .await,
            Err(EgressError::BlockedAddress)
        ));
    }
}

#[tokio::test]
async fn every_dns_answer_is_checked_and_public_hosts_cannot_rebind_private()
-> Result<(), Box<dyn Error>> {
    let timeout = Duration::from_secs(1);
    for address in [
        "169.254.169.254",
        "fd00:ec2::254",
        "fd20:ce::254",
        "100.100.100.200",
        "::ffff:169.254.169.254",
        "fe80::1",
        "64:ff9b::a9fe:a9fe",
        "224.0.0.1",
    ] {
        let resolver = ChangingResolver::new(&[&["10.0.0.3", address]]);
        assert!(matches!(
            resolve_destination(
                &resolver,
                "proxy.internal",
                &policy("proxy.internal", 4000, true),
                timeout
            )
            .await,
            Err(EgressError::BlockedAddress)
        ));
    }
    for address in ["10.0.0.3", "127.0.0.1", "::1", "fd12::1", "::ffff:10.0.0.3"] {
        let resolver = Arc::new(ChangingResolver::new(&[&["8.8.8.8"], &[address]]));
        assert_eq!(
            resolve_destination(
                resolver.as_ref(),
                "unlisted.internal",
                &AddressPolicy::PublicOnly,
                timeout
            )
            .await?,
            vec!["8.8.8.8".parse::<IpAddr>()?]
        );
        let client = GuardedClient::with_resolver(resolver, EgressOptions::default())?;
        assert!(matches!(
            client
                .execute(EgressRequest::get(Url::parse("https://unlisted.internal")?))
                .await,
            Err(EgressError::BlockedAddress)
        ));
    }
    Ok(())
}

async fn server(
    response: &'static str,
) -> Result<(u16, tokio::task::JoinHandle<()>), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("connection");
        let mut buffer = [0; 4096];
        assert!(socket.read(&mut buffer).await.expect("request") > 0);
        socket
            .write_all(response.as_bytes())
            .await
            .expect("response");
    });
    Ok((port, task))
}

#[tokio::test]
async fn allowed_endpoint_blocks_redirects_and_rechecks_new_dns_connections()
-> Result<(), Box<dyn Error>> {
    let (port, task) =
        server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await?;
    let resolver = Arc::new(ChangingResolver::new(&[
        &["127.0.0.1"],
        &["169.254.169.254"],
    ]));
    let client = GuardedClient::with_resolver(
        resolver,
        EgressOptions::default().with_policy(policy("proxy.internal", port, true)),
    )?;
    let request = EgressRequest::get(Url::parse(&format!("http://proxy.internal:{port}"))?);
    assert_eq!(client.execute(request.clone()).await?.body(), b"ok");
    task.await?;
    assert!(matches!(
        client.execute(request).await,
        Err(EgressError::BlockedAddress)
    ));

    for destination in [
        "http://169.254.169.254/latest/meta-data",
        "http://10.0.0.1:4000",
        "https://public.example.com",
        "http://127.0.0.1:1",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("connection");
            let mut buffer = [0; 4096];
            assert!(socket.read(&mut buffer).await.expect("request") > 0);
            socket.write_all(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {destination}\r\nContent-Length: 0\r\n\r\n").as_bytes()).await.expect("response");
        });
        let client = GuardedClient::new(EgressOptions::default().with_policy(policy(
            "127.0.0.1",
            port,
            true,
        )))?;
        assert!(
            matches!(client.execute(EgressRequest::get(Url::parse(&format!("http://127.0.0.1:{port}"))?)).await, Err(EgressError::Status {status, ..}) if status.as_u16() == 307)
        );
        task.await?;
    }
    Ok(())
}

#[test]
fn endpoint_metadata_addresses_are_blocked_under_every_policy() -> Result<(), Box<dyn Error>> {
    let networks = PrivateNetworks::new(["100.64.0.0/10".parse()?, "fc00::/7".parse()?])?;
    for address in [
        "169.254.169.254",
        "100.100.100.200",
        "fd00:ec2::254",
        "fd20:ce::254",
        "::ffff:169.254.169.254",
        "::ffff:100.100.100.200",
        "64:ff9b::a9fe:a9fe",
        "64:ff9b::6464:64c8",
    ] {
        let address: IpAddr = address.parse()?;
        for allowed in [
            AddressPolicy::PublicOnly,
            AddressPolicy::PublicHttp,
            AddressPolicy::AllowLoopback,
            AddressPolicy::AllowPrivate(networks.clone()),
            policy("proxy.internal", 4000, true),
        ] {
            assert!(!allowed.permits(address), "{allowed:?}: {address}");
            assert!(
                !allowed.permits_plain_http(address),
                "{allowed:?}: {address}"
            );
        }
    }
    Ok(())
}

#[test]
fn endpoint_matching_normalizes_hosts_and_uses_effective_ports() -> Result<(), Box<dyn Error>> {
    let endpoint = AllowedEndpoint::new("PROXY.internal", 443, false)?;
    assert_eq!(endpoint.host(), "proxy.internal");
    assert_eq!(endpoint.port(), 443);
    assert!(endpoint.matches(&Url::parse("https://proxy.internal/path")?));
    assert!(endpoint.matches(&Url::parse("https://PROXY.internal:443/other")?));
    let policy = AddressPolicy::OnlyEndpoint(endpoint);
    let error = validate_destination(&Url::parse("https://other.internal")?, &policy)
        .expect_err("host mismatch");
    assert!(matches!(
        error,
        EgressError::Destination(DestinationRejection::Endpoint)
    ));
    assert_eq!(DestinationRejection::Endpoint.code(), "endpoint");
    assert!(!error.is_retryable());
    let endpoint = AllowedEndpoint::new("[::1]", 80, true)?;
    assert!(endpoint.matches(&Url::parse("http://[::1]")?));
    assert_eq!(endpoint.host(), "[::1]");
    Ok(())
}

#[tokio::test]
async fn endpoint_dns_accepts_public_private_shared_and_loopback_answers()
-> Result<(), Box<dyn Error>> {
    let answers = [
        "8.8.8.8",
        "10.0.0.3",
        "100.64.0.1",
        "127.0.0.1",
        "::1",
        "fd12::1",
        "::ffff:10.0.0.3",
    ];
    let resolver = ChangingResolver::new(&[&answers]);
    let resolved = resolve_destination(
        &resolver,
        "PROXY.internal",
        &policy("proxy.internal", 4000, false),
        Duration::from_secs(1),
    )
    .await?;
    let expected = answers
        .iter()
        .map(|address| address.parse::<IpAddr>().expect("address").to_canonical())
        .collect::<Vec<_>>();
    assert_eq!(resolved, expected);
    Ok(())
}
