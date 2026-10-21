use baukit_data_subject::{
    CustomIdentifierType, DataSubjectRequest, Identifier, IdentifierType, Limits, ValidationError,
    codes, normalize_request, validate_idempotency_key, validate_subject_reference,
};

fn request(kind: IdentifierType, value: &str) -> DataSubjectRequest {
    DataSubjectRequest {
        subject_reference: "fixture-reference-01".into(),
        identifiers: vec![Identifier {
            identifier_type: kind,
            value: value.into(),
        }],
    }
}

#[test]
fn reference_and_key_accept_inclusive_bounds_and_all_visible_ascii() {
    for length in [16, 17, 127, 128] {
        let value = "a".repeat(length);
        assert!(validate_subject_reference(&value).is_ok());
        assert!(validate_idempotency_key(Some(&value)).is_ok());
    }
    let visible: String = (0x21..=0x7e).map(char::from).collect();
    assert!(validate_subject_reference(&visible).is_ok());
    assert!(validate_idempotency_key(Some(&visible)).is_ok());
}

#[test]
fn reference_and_key_reject_invalid_lengths_and_characters_without_trimming() {
    for value in [
        String::new(),
        "a".repeat(15),
        "a".repeat(129),
        "a".repeat(1024),
        " fixture-reference-01".into(),
        "fixture-reference-01 ".into(),
        "fixture reference 01".into(),
        "fixture-reference-\t".into(),
        "fixture-reference-\n".into(),
        "fixture-reference-\r".into(),
        "fixture-reference-\0".into(),
        "fixture-reference-\u{7f}".into(),
        "fixture-reference-é".into(),
    ] {
        assert_eq!(
            validate_subject_reference(&value),
            Err(ValidationError::SubjectReference)
        );
        assert_eq!(
            validate_idempotency_key(Some(&value)),
            Err(ValidationError::IdempotencyKey)
        );
    }
    assert_eq!(
        validate_idempotency_key(None),
        Err(ValidationError::IdempotencyKey)
    );
    let mut invalid = request(IdentifierType::AccountId, "fixture");
    invalid.subject_reference = "short".into();
    assert_eq!(
        normalize_request(&invalid, &Limits::default(), &[IdentifierType::AccountId]),
        Err(ValidationError::SubjectReference)
    );
}

#[test]
fn identifier_count_enforces_discovery_bounds() {
    let mut body = request(IdentifierType::AccountId, "fixture");
    let identifier = body.identifiers[0].clone();
    for count in [1, 16] {
        body.identifiers = vec![identifier.clone(); count];
        assert!(normalize_request(&body, &Limits::default(), &[IdentifierType::AccountId]).is_ok());
    }
    for count in [0, 17] {
        body.identifiers = vec![identifier.clone(); count];
        assert_eq!(
            normalize_request(&body, &Limits::default(), &[IdentifierType::AccountId]),
            Err(ValidationError::IdentifierCount)
        );
    }
    body.identifiers = vec![identifier; 2];
    let mut limits = Limits {
        max_identifiers: 1,
        ..Limits::default()
    };
    assert_eq!(
        normalize_request(&body, &limits, &[IdentifierType::AccountId]),
        Err(ValidationError::IdentifierCount)
    );
    limits.max_identifiers = 0;
    body.identifiers.truncate(1);
    assert_eq!(
        normalize_request(&body, &limits, &[IdentifierType::AccountId]),
        Err(ValidationError::IdentifierCount)
    );
}

#[test]
fn lengths_count_raw_and_normalized_utf8_bytes() {
    for value in ["a".repeat(256), "é".repeat(128)] {
        assert!(
            normalize_request(
                &request(IdentifierType::AccountId, &value),
                &Limits::default(),
                &[IdentifierType::AccountId]
            )
            .is_ok()
        );
    }
    for value in [
        "a".repeat(257),
        "é".repeat(129),
        format!(" {} ", "a".repeat(256)),
    ] {
        assert_eq!(
            normalize_request(
                &request(IdentifierType::AccountId, &value),
                &Limits::default(),
                &[IdentifierType::AccountId]
            ),
            Err(ValidationError::IdentifierLength)
        );
    }
    let limits = Limits {
        max_identifier_length: 2,
        ..Limits::default()
    };
    assert_eq!(
        normalize_request(
            &request(IdentifierType::Email, "İ"),
            &limits,
            &[IdentifierType::Email]
        ),
        Err(ValidationError::IdentifierLength)
    );
    let limits = Limits {
        max_identifier_length: 0,
        ..Limits::default()
    };
    assert_eq!(
        normalize_request(
            &request(IdentifierType::AccountId, "a"),
            &limits,
            &[IdentifierType::AccountId]
        ),
        Err(ValidationError::IdentifierLength)
    );
}

#[test]
fn normalization_trims_and_lowercases_only_the_specified_types() {
    let custom: IdentifierType = "x-loyalty-id".parse().expect("valid custom type");
    for (kind, input, expected) in [
        (
            IdentifierType::Email,
            "  FiXtUrE@EXAMPLE.INVALID\t",
            "fixture@example.invalid",
        ),
        (IdentifierType::Email, "\u{2003}ÉXAMPLE\u{2003}", "éxample"),
        (IdentifierType::Email, "İ", "i\u{307}"),
        (IdentifierType::AccountId, "  FiXtUrE  ", "FiXtUrE"),
        (IdentifierType::CustomerId, "\tFiXtUrE\n", "FiXtUrE"),
        (custom, "\u{2003}FiXtUrE\u{2003}", "FiXtUrE"),
        (IdentifierType::Phone, "+12345678", "+12345678"),
    ] {
        let body = request(kind.clone(), input);
        let normalized = normalize_request(&body, &Limits::default(), std::slice::from_ref(&kind))
            .expect("valid request");
        assert_eq!(normalized.identifiers()[0].value(), expected);
        assert_eq!(normalized.identifiers()[0].identifier_type(), &kind);
        assert_eq!(normalized.subject_reference(), body.subject_reference);
        assert_eq!(body.identifiers[0].value, input);
    }
}

#[test]
fn empty_normalized_values_are_invalid() {
    for kind in [
        IdentifierType::Email,
        IdentifierType::AccountId,
        IdentifierType::CustomerId,
        "x-fixture".parse().expect("custom type"),
    ] {
        for value in ["", " \t\n", "\u{2003}"] {
            assert_eq!(
                normalize_request(
                    &request(kind.clone(), value),
                    &Limits::default(),
                    std::slice::from_ref(&kind)
                ),
                Err(ValidationError::IdentifierEmpty)
            );
        }
    }
}

#[test]
fn phone_requires_plus_and_eight_to_fifteen_ascii_digits() {
    for value in ["+12345678", "+123456789012345", "+00000000"] {
        assert!(
            normalize_request(
                &request(IdentifierType::Phone, value),
                &Limits::default(),
                &[IdentifierType::Phone]
            )
            .is_ok()
        );
    }
    for value in [
        "",
        "+",
        "12345678",
        "+1234567",
        "+1234567890123456",
        " +12345678",
        "+12345678 ",
        "+1234-5678",
        "+1234 5678",
        "+１２３４５６７８",
        "++12345678",
        "+1234567a",
        "+1234567\n",
    ] {
        assert_eq!(
            normalize_request(
                &request(IdentifierType::Phone, value),
                &Limits::default(),
                &[IdentifierType::Phone]
            ),
            Err(ValidationError::Phone)
        );
    }
}

#[test]
fn custom_types_validate_on_construction_and_deserialization() {
    for value in ["x-a", "x-1", "x-fixture-id", "x-a1-2b"] {
        let custom = CustomIdentifierType::new(value).expect("valid custom type");
        assert_eq!(custom.as_str(), value);
        assert_eq!(IdentifierType::Custom(custom).as_str(), value);
        let parsed: IdentifierType =
            serde_json::from_str(&format!("\"{value}\"")).expect("valid type");
        assert_eq!(parsed.as_str(), value);
    }
    for value in [
        "",
        "email ",
        "account_id",
        "Email",
        "x-",
        "X-a",
        "x-A",
        "x--a",
        "x-a-",
        "x-a--b",
        "x-a_b",
        "x-é",
        "x-a.b",
        "x-a b",
        "unknown",
    ] {
        assert_eq!(
            value.parse::<IdentifierType>(),
            Err(ValidationError::IdentifierType)
        );
        assert!(CustomIdentifierType::new(value).is_err());
        assert!(serde_json::from_str::<IdentifierType>(&format!("\"{value}\"")).is_err());
    }
}

#[test]
fn valid_types_must_be_advertised() {
    for kind in [
        IdentifierType::Email,
        IdentifierType::AccountId,
        IdentifierType::CustomerId,
        IdentifierType::Phone,
        "x-fixture".parse().expect("custom type"),
    ] {
        assert_eq!(
            normalize_request(&request(kind, "fixture"), &Limits::default(), &[]),
            Err(ValidationError::UnsupportedIdentifierType)
        );
    }
}

#[test]
fn normalized_body_keeps_identifier_order_duplicates_and_subject_reference() {
    let mut body = request(IdentifierType::Email, " FIXTURE@EXAMPLE.INVALID ");
    body.identifiers.push(body.identifiers[0].clone());
    body.identifiers.push(Identifier {
        identifier_type: IdentifierType::AccountId,
        value: " FiXtUrE ".into(),
    });
    let normalized = normalize_request(
        &body,
        &Limits::default(),
        &[IdentifierType::Email, IdentifierType::AccountId],
    )
    .expect("valid request");
    assert_eq!(
        serde_json::to_string(&normalized).expect("serialize normalized body"),
        r#"{"subjectReference":"fixture-reference-01","identifiers":[{"type":"email","value":"fixture@example.invalid"},{"type":"email","value":"fixture@example.invalid"},{"type":"accountId","value":"FiXtUrE"}]}"#
    );
    body.identifiers.swap(0, 2);
    assert_ne!(
        normalize_request(
            &body,
            &Limits::default(),
            &[IdentifierType::Email, IdentifierType::AccountId]
        )
        .expect("valid reordered request"),
        normalized
    );
}

#[test]
fn errors_map_to_contract_codes_and_never_include_inputs() {
    for error in [
        ValidationError::SubjectReference,
        ValidationError::IdentifierCount,
        ValidationError::IdentifierLength,
        ValidationError::IdentifierEmpty,
        ValidationError::Phone,
        ValidationError::IdentifierType,
    ] {
        assert_eq!(error.code(), codes::REQUEST_INVALID);
        assert_eq!(error.status_code(), 422);
    }
    assert_eq!(
        ValidationError::UnsupportedIdentifierType.code(),
        codes::IDENTIFIER_TYPE_UNSUPPORTED
    );
    assert_eq!(
        ValidationError::UnsupportedIdentifierType.status_code(),
        422
    );
    assert_eq!(
        ValidationError::IdempotencyKey.code(),
        codes::IDEMPOTENCY_KEY_INVALID
    );
    assert_eq!(ValidationError::IdempotencyKey.status_code(), 400);
    let body = request(IdentifierType::Email, "PRIVATE-FIXTURE-MARKER");
    let normalized = normalize_request(&body, &Limits::default(), &[IdentifierType::Email])
        .expect("valid request");
    for debug in [
        format!("{body:?}"),
        format!("{:?}", body.identifiers[0]),
        format!("{normalized:?}"),
        format!("{:?}", normalized.identifiers()[0]),
    ] {
        assert!(!debug.to_lowercase().contains("private-fixture-marker"));
    }
    let error = serde_json::from_str::<IdentifierType>("\"PRIVATE-FIXTURE-MARKER\"")
        .expect_err("invalid type");
    assert!(!error.to_string().contains("PRIVATE-FIXTURE-MARKER"));
}

#[test]
fn every_error_code_is_pinned() {
    assert_eq!(
        [
            codes::IDEMPOTENCY_KEY_INVALID,
            codes::REQUEST_INVALID,
            codes::IDENTIFIER_TYPE_UNSUPPORTED,
            codes::IDEMPOTENCY_CONFLICT,
            codes::OPERATION_NOT_FOUND,
            codes::EXPORT_NOT_READY,
            codes::EXPORT_EXPIRED,
            codes::CLIENT_NOT_ALLOWED,
            codes::INSUFFICIENT_SCOPE,
        ],
        [
            "data_subject_idempotency_key_invalid",
            "data_subject_request_invalid",
            "data_subject_identifier_type_unsupported",
            "data_subject_idempotency_conflict",
            "data_subject_operation_not_found",
            "data_subject_export_not_ready",
            "data_subject_export_expired",
            "data_subject_client_not_allowed",
            "insufficient_scope",
        ]
    );
}
