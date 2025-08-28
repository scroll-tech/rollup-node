use alloy_primitives::B256;
use rollup_node_signer::Signature;
use sea_orm::{entity::prelude::*, ActiveValue};

/// A database model that represents a block signature.
///
/// TODO: remove this once we deprecated l2geth.
/// The purpose of this model is to store the block signature in the database, and it will be used
/// to provide the block signature when reth receives eth66 block request from scroll's l2geth.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "block_signature")]
pub struct Model {
    /// The block hash as a primary key.
    #[sea_orm(primary_key, auto_increment = false)]
    pub block_hash: Vec<u8>,
    /// The block signature.
    pub signature: Vec<u8>,
}

/// The relation for the block signature model.
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

/// The active model behavior for the block signature model.
impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// Get the block hash as B256
    pub fn get_block_hash(&self) -> Result<B256, String> {
        if self.block_hash.len() != 32 {
            return Err(format!("Invalid block hash length: {}", self.block_hash.len()));
        }
        Ok(B256::from_slice(&self.block_hash))
    }

    /// Get the signature
    pub fn get_signature(&self) -> Result<Signature, String> {
        if self.signature.len() != 65 {
            return Err(format!("Invalid signature length: {}", self.signature.len()));
        }
        Signature::from_raw(&self.signature).map_err(|e| format!("Invalid signature: {}", e))
    }
}

impl From<(B256, Signature)> for ActiveModel {
    fn from((block_hash, signature): (B256, Signature)) -> Self {
        Self {
            block_hash: ActiveValue::Set(block_hash.to_vec()),
            signature: ActiveValue::Set(signature.as_bytes().to_vec()),
        }
    }
}
