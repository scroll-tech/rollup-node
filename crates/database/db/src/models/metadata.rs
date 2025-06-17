use rollup_node_primitives::Metadata;
use sea_orm::{entity::prelude::*, ActiveValue};

/// A database model that represents a batch input.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "metadata")]
pub struct Model {
    /// The metadata key.
    #[sea_orm(primary_key)]
    pub key: String,
    /// The metadata value.
    pub value: String,
}

/// The relation for the batch input model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

/// The active model behavior for the batch input model.
impl ActiveModelBehavior for ActiveModel {}

impl From<Metadata> for ActiveModel {
    fn from(metadata: Metadata) -> Self {
        Self {
            key: ActiveValue::Set("l1_finalized_block".to_owned()),
            value: ActiveValue::Set(metadata.l1_finalized_block.to_string()),
        }
    }
}

impl From<Model> for Metadata {
    fn from(value: Model) -> Self {
        debug_assert!(value.key == "l1_finalized_block");
        Self { l1_finalized_block: value.value.parse().expect("invalid value") }
    }
}
