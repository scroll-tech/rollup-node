use alloy_primitives::PrimitiveSignature;
use network::{NetworkConfigBuilder, NetworkManager, NoopBlockImport};
use reth_network::{NetworkInfo, Peers};
use reth_scroll_chainspec::SCROLL_MAINNET;

#[tokio::main]
async fn main() {
    let config_1 =
        NetworkConfigBuilder::<reth_network::EthNetworkPrimitives>::with_rng_secret_key()
            .disable_discovery()
            .build_with_noop_provider((*SCROLL_MAINNET).clone());
    let network_1 = NetworkManager::new(config_1, NoopBlockImport).await;
    let network_1_handle = network_1.handle();
    let peer_1_id = *network_1_handle.peer_id();
    let peer_1_addr = network_1_handle.inner().local_addr();

    let config_2 =
        NetworkConfigBuilder::<reth_network::EthNetworkPrimitives>::with_rng_secret_key()
            .disable_discovery()
            .listener_addr(std::net::SocketAddr::V4(std::net::SocketAddrV4::new(
                std::net::Ipv4Addr::UNSPECIFIED,
                0,
            )))
            .build_with_noop_provider((*SCROLL_MAINNET).clone());
    let network_2 = NetworkManager::new(config_2, NoopBlockImport).await;
    let network_2_handle = network_2.handle();
    let peer_2_id = *network_2_handle.peer_id();
    let peer_2_addr = network_2_handle.inner().local_addr();

    tokio::spawn(network_1);
    tokio::spawn(network_2);

    network_1_handle.inner().add_peer(peer_2_id, peer_2_addr);
    network_2_handle.inner().add_peer(peer_1_id, peer_1_addr);

    let signature = PrimitiveSignature::try_from(&[0u8; 65][..]).unwrap();
    let block = reth_primitives::Block::default();

    for _ in 0..100 {
        network_1_handle.announce_block(block.clone(), signature);
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}
