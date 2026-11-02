# Recorded hub plan catalog

`catalog.json` is a byte-for-byte copy of Schlüsselbund's `fixtures/plans/v1/catalog.json` at commit `fd2a2a3efc112b3894da24def6ae6de23dfa0591`, the C23 commit that last changed the file and added `organizationProducts` to the business plans. `baukit_suite::platform::feature_key_drift` and `assert_feature_keys_match` read it in product contract tests. Record a new copy and update this commit when the hub catalog changes.
