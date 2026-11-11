use super::*;

pub(super) async fn check_validation(
    http: &mut Http<'_>,
    discovery: &DiscoveryDocument,
    kind: OperationKind,
    token: &str,
    request: &DataSubjectRequest,
) -> Result<(), Failure> {
    let path = route(kind);
    let body = serde_json::to_value(request).map_err(|_| Failure::Adapter)?;
    check_idempotency_key_validation(http, path, token, &body).await?;
    http.send_bytes(
        "POST",
        path,
        Some(token),
        &["conformance-key-01"],
        b"{broken".to_vec(),
    )
    .await?
    .error(422, codes::REQUEST_INVALID, Failure::InvalidBody)?;
    for body in invalid_bodies(discovery, request, &body) {
        http.send(
            "POST",
            path,
            Some(token),
            &["conformance-key-01"],
            Some(body),
        )
        .await?
        .error(422, codes::REQUEST_INVALID, Failure::InvalidBody)?;
    }
    check_unsupported_identifier_type(http, discovery, path, token, &body).await?;
    check_identifier_limits(http, discovery, path, token, body).await
}

async fn check_idempotency_key_validation(
    http: &mut Http<'_>,
    path: &str,
    token: &str,
    body: &Value,
) -> Result<(), Failure> {
    let long = "k".repeat(129);
    for keys in [
        vec![],
        vec!["k"; 1],
        vec![long.as_str()],
        vec!["conformance-key-01", "conformance-key-01"],
        vec!["invalid key with spaces"],
    ] {
        http.send("POST", path, Some(token), &keys, Some(body.clone()))
            .await?
            .error(400, codes::IDEMPOTENCY_KEY_INVALID, Failure::IdempotencyKey)?;
    }
    Ok(())
}

fn invalid_bodies(
    discovery: &DiscoveryDocument,
    request: &DataSubjectRequest,
    body: &Value,
) -> Vec<Value> {
    let mut invalid = vec![
        Value::Null,
        json!({}),
        json!({"subjectReference": request.subject_reference, "identifiers": []}),
    ];
    for reference in [
        "r".repeat(15),
        "r".repeat(129),
        "reference with spaces".into(),
        "référence-non-ascii".into(),
    ] {
        let mut changed = body.clone();
        changed["subjectReference"] = json!(reference);
        invalid.push(changed);
    }
    for (kind, value) in [
        ("x-Bad", "invalid-type-value"),
        (request.identifiers[0].identifier_type.as_str(), "   "),
    ] {
        let mut changed = body.clone();
        changed["identifiers"] = json!([{"type":kind,"value":value}]);
        invalid.push(changed);
    }
    if discovery.identifier_types.contains(&IdentifierType::Phone) {
        let mut changed = body.clone();
        changed["identifiers"] = json!([{"type":"phone","value":"+1234invalid5678"}]);
        invalid.push(changed);
    }
    invalid
}

async fn check_unsupported_identifier_type(
    http: &mut Http<'_>,
    discovery: &DiscoveryDocument,
    path: &str,
    token: &str,
    body: &Value,
) -> Result<(), Failure> {
    let mut unsupported = "x-conformance-unsupported".to_owned();
    while discovery
        .identifier_types
        .iter()
        .any(|kind| kind.as_str() == unsupported)
    {
        unsupported.push_str("-other");
    }
    let mut changed = body.clone();
    changed["identifiers"][0]["type"] = json!(unsupported);
    http.send(
        "POST",
        path,
        Some(token),
        &["conformance-key-01"],
        Some(changed),
    )
    .await?
    .error(
        422,
        codes::IDENTIFIER_TYPE_UNSUPPORTED,
        Failure::UnsupportedIdentifierType,
    )
}

async fn check_identifier_limits(
    http: &mut Http<'_>,
    discovery: &DiscoveryDocument,
    path: &str,
    token: &str,
    body: Value,
) -> Result<(), Failure> {
    let mut changed = body.clone();
    changed["identifiers"] = json!(vec![
        body["identifiers"][0].clone();
        discovery
            .limits
            .max_identifiers
            .checked_add(1)
            .ok_or(Failure::DiscoveryShape)?
    ]);
    http.send(
        "POST",
        path,
        Some(token),
        &["conformance-key-01"],
        Some(changed),
    )
    .await?
    .error(422, codes::REQUEST_INVALID, Failure::Limits)?;
    let mut changed = body;
    changed["identifiers"][0]["value"] = json!(
        "z".repeat(
            discovery
                .limits
                .max_identifier_length
                .checked_add(1)
                .ok_or(Failure::DiscoveryShape)?
        )
    );
    http.send(
        "POST",
        path,
        Some(token),
        &["conformance-key-01"],
        Some(changed),
    )
    .await?
    .error(422, codes::REQUEST_INVALID, Failure::Limits)
}
