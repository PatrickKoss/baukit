//! Extra trusted roots configured on [`EgressOptions`].

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

use baukit_egress::{
    AddressPolicy, EgressError, EgressOptions, EgressOptionsError, EgressRequest, GuardedClient,
    StaticResolver,
};
use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;
use url::Url;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const HOST: &str = "bank.test";
const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

struct PrivateCa {
    root_pem: String,
    leaf: CertificateDer<'static>,
    leaf_key: PrivateKeyDer<'static>,
}

fn private_ca() -> TestResult<PrivateCa> {
    let mut root_params = CertificateParams::new(Vec::<String>::new())?;
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let root = CertifiedIssuer::self_signed(root_params, KeyPair::generate()?)?;
    let leaf_key = KeyPair::generate()?;
    let leaf = CertificateParams::new(vec![HOST.to_owned()])?.signed_by(&leaf_key, &root)?;
    Ok(PrivateCa {
        root_pem: root.pem(),
        leaf: leaf.der().clone(),
        leaf_key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    })
}

async fn serve_tls(ca: &PrivateCa, connections: usize) -> TestResult<Url> {
    let config = ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(vec![ca.leaf.clone()], ca.leaf_key.clone_key())?;
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        for _ in 0..connections {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(mut tls) = acceptor.accept(stream).await else {
                continue;
            };
            let mut buffer = [0_u8; 4096];
            if tls.read(&mut buffer).await.is_err() {
                continue;
            }
            let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok";
            if tls.write_all(reply).await.is_ok() {
                let _ = tls.shutdown().await;
            }
        }
    });
    Ok(Url::parse(&format!("https://{HOST}:{port}/"))?)
}

fn client(options: EgressOptions) -> TestResult<GuardedClient> {
    let resolver = Arc::new(StaticResolver::new().with_host(HOST, [LOOPBACK]));
    Ok(GuardedClient::with_resolver(resolver, options)?)
}

#[tokio::test]
async fn a_private_ca_is_refused_without_its_root_and_accepted_with_it() -> TestResult {
    let ca = private_ca()?;
    let url = serve_tls(&ca, 2).await?;
    let loopback = EgressOptions::default().with_policy(AddressPolicy::AllowLoopback);

    let refused = client(loopback.clone())?
        .execute(EgressRequest::get(url.clone()))
        .await
        .expect_err("an unknown CA is refused");
    assert!(matches!(refused, EgressError::Transport(_)), "{refused:?}");

    let trusted = loopback.with_extra_root_certificates_pem(ca.root_pem.as_bytes())?;
    let response = client(trusted)?
        .execute(EgressRequest::get(url))
        .await
        .expect("the extra root is trusted");
    assert_eq!(response.body(), b"ok");
    Ok(())
}

#[tokio::test]
async fn an_extra_root_does_not_lift_the_address_policy() -> TestResult {
    let ca = private_ca()?;
    let url = serve_tls(&ca, 1).await?;
    let public_only =
        EgressOptions::default().with_extra_root_certificates_pem(ca.root_pem.as_bytes())?;
    let error = client(public_only)?
        .execute(EgressRequest::get(url))
        .await
        .expect_err("loopback stays blocked");
    assert!(matches!(error, EgressError::BlockedAddress), "{error:?}");
    Ok(())
}

#[test]
fn malformed_or_empty_pem_is_refused() {
    let truncated = "-----BEGIN CERTIFICATE-----\nnot base64\n-----END CERTIFICATE-----\n";
    for pem in [&b""[..], b"no certificate here", truncated.as_bytes()] {
        assert_eq!(
            EgressOptions::default().with_extra_root_certificates_pem(pem),
            Err(EgressOptionsError::InvalidRootCertificate)
        );
    }
}
