use alloy::{
    providers::{Provider, ProviderBuilder, RootProvider},
    transports::http::{Client, Http},
};
use std::time::Duration;

pub struct TestClients {
    pub sequencer_provider: RootProvider<Http<Client>>,
    pub follower_provider: RootProvider<Http<Client>>,
}

impl TestClients {
    pub async fn new(
        sequencer_url: &str,
        follower_url: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let sequencer_provider = ProviderBuilder::new().on_http(sequencer_url.parse()?);

        let follower_provider = ProviderBuilder::new().on_http(follower_url.parse()?);

        Ok(Self { sequencer_provider, follower_provider })
    }

    pub fn l2_provider(&self) -> &dyn Provider<Http<Client>> {
        &self.sequencer_provider
    }

    pub async fn verify_connectivity(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Sequencer
        let seq_chain_id = self.sequencer_provider.get_chain_id().await?;
        let seq_block = self.sequencer_provider.get_block_number().await?;
        println!("✅ Sequencer connected - Chain ID: {}, Block: {}", seq_chain_id, seq_block);

        // Follower
        let fol_chain_id = self.follower_provider.get_chain_id().await?;
        let fol_block = self.follower_provider.get_block_number().await?;
        println!("✅ Follower connected - Chain ID: {}, Block: {}", fol_chain_id, fol_block);

        Ok(())
    }

    pub async fn wait_for_sequencer_blocks(
        &self,
        target_blocks: u64,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let start_block = self.sequencer_provider.get_block_number().await?;
        let target_block = start_block + target_blocks;

        println!("⏳ Waiting for sequencer to produce {} blocks...", target_blocks);

        for i in 0..60 {
            let current_block = self.sequencer_provider.get_block_number().await?;
            if current_block >= target_block {
                println!("✅ Sequencer reached block {}", current_block);
                return Ok(current_block);
            }

            if i % 5 == 0 {
                println!("⏳ Sequencer at block {}/{}", current_block, target_block);
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        Err("Timeout waiting for sequencer blocks".into())
    }

    pub async fn wait_for_follower_sync(
        &self,
        target_block: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!("⏳ Waiting for follower to sync to block {}...", target_block);

        for i in 0..60 {
            let follower_block = self.follower_provider.get_block_number().await?;
            if follower_block >= target_block {
                println!("✅ Follower synced to block {}", follower_block);
                return Ok(());
            }

            if i % 5 == 0 {
                println!("⏳ Follower at block {}/{}", follower_block, target_block);
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        Err("Timeout waiting for follower sync".into())
    }

    pub async fn verify_blocks_match(
        &self,
        block_number: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let seq_block = self
            .sequencer_provider
            .get_block_by_number(block_number.into(), false)
            .await?
            .ok_or("Sequencer block not found")?;

        let fol_block = self
            .follower_provider
            .get_block_by_number(block_number.into(), false)
            .await?
            .ok_or("Follower block not found")?;

        let seq_hash = seq_block.header.hash.unwrap();
        let fol_hash = fol_block.header.hash.unwrap();

        if seq_hash != fol_hash {
            return Err(format!(
                "Block {} hashes differ: sequencer={:?}, follower={:?}",
                block_number, seq_hash, fol_hash
            )
            .into());
        }

        println!("✅ Block {} matches: hash={:?}", block_number, seq_hash);
        Ok(())
    }

    pub async fn wait_for_l2_blocks(
        &self,
        blocks: u64,
        timeout_secs: u64,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let start_time = std::time::Instant::now();
        let start_block = self.sequencer_provider.get_block_number().await?;
        let target_block = start_block + blocks;

        loop {
            if start_time.elapsed().as_secs() > timeout_secs {
                return Err("Timeout waiting for blocks".into());
            }

            let current_block = self.sequencer_provider.get_block_number().await?;
            if current_block >= target_block {
                return Ok(current_block);
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
