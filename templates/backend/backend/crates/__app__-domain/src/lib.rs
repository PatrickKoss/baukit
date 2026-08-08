use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Item {
    pub id: Uuid,
    pub name: String,
}

impl Item {
    pub fn new(id: Uuid, name: impl Into<String>) -> Result<Self, DomainError> {
        let name = name.into();
        let name = name.trim();
        if name.is_empty() {
            return Err(DomainError::EmptyName);
        }
        if name.chars().count() > 200 {
            return Err(DomainError::NameTooLong);
        }
        Ok(Self {
            id,
            name: name.to_owned(),
        })
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DomainError {
    #[error("item name must not be empty")]
    EmptyName,
    #[error("item name must contain at most 200 characters")]
    NameTooLong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_normalizes_its_name() -> Result<(), DomainError> {
        let id = Uuid::nil();
        assert_eq!(Item::new(id, "  first  ")?.name, "first");
        assert_eq!(Item::new(id, " "), Err(DomainError::EmptyName));
        Ok(())
    }
}
