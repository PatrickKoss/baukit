#[cfg(test)]
mod tests {
    use baukit_suite::{
        config::SuiteConfig,
        domain::{PayloadCatalog, PeerRegistry, PeerRegistrySettings, ValidatedPayload},
        services::AuthorizationRequest,
    };
    use serde::Deserialize;
    use serde_json::{Map, Value, json};

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Activity {
        minutes: u32,
    }

    #[test]
    fn domain_consumer_reads_peers_and_validates_product_payloads() {
        let peers = r#"{"schemaVersion":1,"peers":[{"id":"alpha","displayName":"Alpha","emits":["alpha.activity.completed"],"accepts":[],"rewardModes":["off"],"scheme":"alpha"}]}"#;
        let registry = PeerRegistry::new("alpha", peers, PeerRegistrySettings::default())
            .expect("embedded registry");
        assert!(registry.standalone());
        assert_eq!(
            registry.peer_metadata("alpha").expect("own app").scheme,
            "alpha"
        );
        assert!(registry.peer_metadata("missing").is_none());
        assert!(SuiteConfig::default().registry("alpha", peers).is_ok());

        let catalog = PayloadCatalog::from_json(r#"{"schemaVersion":1,"types":[{"type":"alpha.activity.completed","naturalKey":"minutes","fields":[{"name":"minutes","required":true,"type":"integer","minimum":0,"maximum":1440}]}]}"#)
            .expect("embedded catalog");
        let payload: ValidatedPayload = catalog
            .validate(
                "alpha.activity.completed",
                Map::from_iter([("minutes".into(), json!(30))]),
            )
            .expect("valid activity");
        assert_eq!(
            payload.deserialize::<Activity>().expect("product payload"),
            Activity { minutes: 30 }
        );
        for invalid in [Value::Null, json!(-1), json!(1441), json!(1.5)] {
            assert!(
                catalog
                    .validate(
                        "alpha.activity.completed",
                        Map::from_iter([("minutes".into(), invalid)])
                    )
                    .is_err()
            );
        }
        let request: AuthorizationRequest = serde_json::from_value(json!({
            "client": "alpha", "state": "state", "codeChallenge": "challenge", "hint": null,
        }))
        .expect("authorization contract");
        assert_eq!(request.client, "alpha");
    }
}
