use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
struct Catalog {
    plans: Vec<CatalogPlan>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogPlan {
    included_products: Vec<IncludedProduct>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncludedProduct {
    product_id: String,
    features: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlanCatalogError {
    #[error("plan catalog is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("plan catalog grants no features to {0}")]
    UnknownProduct(String),
}

/// Keys a product declares against keys the hub catalog grants it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FeatureKeyDrift {
    /// Declared by the product but granted by no catalog plan.
    pub not_in_catalog: Vec<String>,
    /// Granted by a catalog plan but not declared by the product.
    pub not_declared: Vec<String>,
}
impl FeatureKeyDrift {
    pub fn is_empty(&self) -> bool {
        self.not_in_catalog.is_empty() && self.not_declared.is_empty()
    }
}

/// Every feature key any plan in `catalog_json` grants to `product_id`.
pub fn catalog_feature_keys(
    catalog_json: &str,
    product_id: &str,
) -> Result<BTreeSet<String>, PlanCatalogError> {
    let catalog: Catalog = serde_json::from_str(catalog_json)?;
    let keys: BTreeSet<String> = catalog
        .plans
        .into_iter()
        .flat_map(|plan| plan.included_products)
        .filter(|included| included.product_id == product_id)
        .flat_map(|included| included.features)
        .collect();
    if keys.is_empty() {
        return Err(PlanCatalogError::UnknownProduct(product_id.into()));
    }
    Ok(keys)
}

pub fn feature_key_drift<'a>(
    catalog_json: &str,
    product_id: &str,
    declared: impl IntoIterator<Item = &'a str>,
) -> Result<FeatureKeyDrift, PlanCatalogError> {
    let catalog = catalog_feature_keys(catalog_json, product_id)?;
    let declared: BTreeSet<String> = declared.into_iter().map(str::to_owned).collect();
    Ok(FeatureKeyDrift {
        not_in_catalog: declared.difference(&catalog).cloned().collect(),
        not_declared: catalog.difference(&declared).cloned().collect(),
    })
}

/// Contract-test assertion: panics with both differences when the product's
/// keys and the recorded hub catalog disagree.
pub fn assert_feature_keys_match<'a>(
    catalog_json: &str,
    product_id: &str,
    declared: impl IntoIterator<Item = &'a str>,
) {
    let drift = feature_key_drift(catalog_json, product_id, declared)
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        drift.is_empty(),
        "{product_id} feature keys drift from the hub catalog: not in catalog {:?}, not declared {:?}",
        drift.not_in_catalog,
        drift.not_declared
    );
}
