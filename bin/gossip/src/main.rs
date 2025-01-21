use network::{
    EthNetworkPrimitives, NetworkConfigBuilder, NetworkManager, ScrollWireConfig, SCROLL_MAINNET,
};
use reth_network::config::{rng_secret_key, NetworkMode};
use reth_node_core::args::NetworkArgs;
use temp_dir::TempDir;

mod import;
use import::GossipBlockImport;

#[tokio::main]
async fn main() {
    let default_peers_file = TempDir::new().unwrap().path().join("peers.json");
    let secret_key = rng_secret_key();
    let scroll_chain_spec = (*SCROLL_MAINNET).clone();

    let (new_block_tx, new_block_rx) = tokio::sync::mpsc::unbounded_channel();
    let network_config = NetworkArgs::default()
        .network_config(
            &Default::default(),
            scroll_chain_spec.clone(),
            secret_key,
            default_peers_file,
        )
        .block_import(Box::new(GossipBlockImport::new(new_block_tx, secret_key)))
        .network_mode(NetworkMode::Work)
        .build_with_noop_provider(scroll_chain_spec);
    let scroll_wire_config = ScrollWireConfig::new(true);
    let network = NetworkManager::new(network_config, scroll_wire_config, network::NoopBlockImport)
        .await
        .with_new_block_source(new_block_rx);

    let handle = tokio::spawn(network);

    handle.await.unwrap()
}
