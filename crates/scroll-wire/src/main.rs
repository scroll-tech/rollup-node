use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use reth_eth_wire::EthNetworkPrimitives;
use reth_network::{protocol::IntoRlpxSubProtocol, NetworkConfig, NetworkManager};
use reth_scroll_chainspec::SCROLL_MAINNET;
use scroll_wire::{NewBlockMessage, ScrollNetwork, ScrollWireProtocolHandler};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Instantiate the network manager for the first peer.
    let (handler, events, broadcast) = ScrollWireProtocolHandler::new();
    let config = NetworkConfig::<_, EthNetworkPrimitives>::builder_with_rng_secret_key()
        .add_rlpx_sub_protocol(handler.into_rlpx_sub_protocol())
        .disable_discovery()
        .build_with_noop_provider((*SCROLL_MAINNET).clone());
    let network = NetworkManager::new(config).await?;

    // Fetch the peer ID, address, and peers handle for the first  peer.
    let peer_1_id = *network.peer_id();
    let peer_1_addr = network.local_addr();
    let peer_1_peers_handle = network.peers_handle();

    // Instantiate the ScrollNetwork for the first peer.
    let scroll_wire = ScrollNetwork::new(events, broadcast);

    // Launch the network manager and ScrollNetwork for the first peer.
    tokio::task::spawn(network);
    tokio::task::spawn(scroll_wire);

    // Instantiate the network manager for the second peer.
    let (handler, events, broadcast) = ScrollWireProtocolHandler::new();
    let config = NetworkConfig::<_, EthNetworkPrimitives>::builder_with_rng_secret_key()
        .add_rlpx_sub_protocol(handler.into_rlpx_sub_protocol())
        .disable_discovery()
        .listener_addr(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)))
        .build_with_noop_provider((*SCROLL_MAINNET).clone());
    let network = NetworkManager::new(config).await?;

    // Fetch the peer ID, address, and peers handle for the second peer.
    let peer_2_id = *network.peer_id();
    let peer_2_addr = network.local_addr();
    let peer_2_peers_handle = network.peers_handle();

    // Instantiate the ScrollNetwork for the second peer.
    let scroll_wire = ScrollNetwork::new(events, broadcast);
    let scroll_wire_handle = scroll_wire.handle().clone();

    // Launch the network manager and ScrollNetwork for the second peer.
    tokio::task::spawn(network);
    tokio::task::spawn(scroll_wire);

    // Add the peers to each other's peer sets.
    peer_1_peers_handle.add_peer(peer_2_id, peer_2_addr);
    peer_2_peers_handle.add_peer(peer_1_id, peer_1_addr);

    // Announce a new block every 5 seconds.
    // Observe logs in terminal to see the new block announcements.
    while true {
        let new_block = NewBlockMessage {
            signature: [0u8; 65],
            block: Default::default(),
        };
        scroll_wire_handle.announce_block(new_block);

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    Ok(())
}
