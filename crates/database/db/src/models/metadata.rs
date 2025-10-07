use rollup_node_primitives::Metadata;
use sea_orm::{entity::prelude::*, ActiveValue};

/// A database model that represents the metadata for the rollup node.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "metadata")]
pub struct Model {
    /// The metadata key.
    #[sea_orm(primary_key)]
    pub key: String,
    /// The metadata value.
    pub value: String,
}

/// The relation for the metadata model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

/// The active model behavior for the metadata model.
impl ActiveModelBehavior for ActiveModel {}

impl From<Metadata> for ActiveModel {
    fn from(metadata: Metadata) -> Self {
        Self { key: ActiveValue::Set(metadata.key), value: ActiveValue::Set(metadata.value) }
    }
}
