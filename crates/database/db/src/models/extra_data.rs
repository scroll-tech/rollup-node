use alloy_primitives::Bytes;
use sea_orm::entity::prelude::*;

/// A database model that represents extra data for the block.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "extra_data")]
pub struct Model {
    #[sea_orm(primary_key)]
    block_number: i64,
    block_hash: Vec<u8>,
    data: Vec<u8>,
}

impl Model {
    /// Returns the extra data for the model.
    pub fn extra_data(&self) -> Bytes {
        self.data.clone().into()
    }
}

/// The relation for the extra data model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

/// The active model behavior for the extra data model.
impl ActiveModelBehavior for ActiveModel {}
