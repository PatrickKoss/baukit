use std::error::Error;

use uuid::Uuid;

use {{ context.app_crate }}_domain::Item;
use {{ context.app_crate }}_ports::ItemRepository;

/// A second owner can neither read, list, update nor delete the first owner's item.
pub async fn assert_items_are_owner_scoped(
    repository: &dyn ItemRepository,
) -> Result<(), Box<dyn Error>> {
    let item = repository
        .create(Item::new(
            Uuid::now_v7(),
            "first-owner",
            "first owner's item",
        )?)
        .await?;
    assert_eq!(repository.get("second-owner", item.id).await?, None);
    assert!(repository.list("second-owner").await?.is_empty());
    let overwrite = Item::new(item.id, "second-owner", "overwrite")?;
    assert_eq!(repository.update(overwrite).await?, None);
    assert!(!repository.delete("second-owner", item.id).await?);
    assert_eq!(
        repository.get("first-owner", item.id).await?,
        Some(item.clone())
    );
    assert_eq!(repository.list("first-owner").await?, vec![item]);
    Ok(())
}
