//! Node specific implementations for Scroll rollup node.

use crate::{args::ScrollRollupNodeConfig, constants};
use std::time::Duration;

use super::add_ons::ScrollRollupNodeAddOns;
use reth_network::protocol::IntoRlpxSubProtocol;
use reth_node_api::NodeTypes;
use reth_node_builder::{
    components::{BasicPayloadServiceBuilder, ComponentsBuilder},
    FullNodeTypes, Node, NodeAdapter, NodeComponentsBuilder,
};
use reth_scroll_node::{
    ScrollConsensusBuilder, ScrollExecutorBuilder, ScrollNetworkBuilder, ScrollNode,
    ScrollPayloadBuilderBuilder, ScrollPoolBuilder,
};
use scroll_wire::{ScrollWireConfig, ScrollWireEvent, ScrollWireProtocolHandler};
use std::sync::Arc;
use tokio::sync::{mpsc::UnboundedReceiver, Mutex};

/// The Scroll node implementation.
#[derive(Clone, Debug)]
pub struct ScrollRollupNode {
    config: ScrollRollupNodeConfig,
    scroll_wire_events: Arc<Mutex<Option<UnboundedReceiver<ScrollWireEvent>>>>,
}

impl ScrollRollupNode {
    /// Create a new instance of [`ScrollRollupNode`].
    pub fn new(config: ScrollRollupNodeConfig) -> Self {
        config
            .validate()
            .map_err(|e| eyre::eyre!("Configuration validation failed: {}", e))
            .expect("Configuration validation failed");

        Self { config, scroll_wire_events: Arc::new(Mutex::new(None)) }
    }
}

impl<N> Node<N> for ScrollRollupNode
where
    N: FullNodeTypes<Types = Self>,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        ScrollPoolBuilder,
        BasicPayloadServiceBuilder<ScrollPayloadBuilderBuilder>,
        ScrollNetworkBuilder,
        ScrollExecutorBuilder,
        ScrollConsensusBuilder,
    >;

    type AddOns = ScrollRollupNodeAddOns<
        NodeAdapter<N, <Self::ComponentsBuilder as NodeComponentsBuilder<N>>::Components>,
    >;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        let (scroll_wire_handler, events) =
            ScrollWireProtocolHandler::new(ScrollWireConfig::new(true));

        *self.scroll_wire_events.try_lock().unwrap() = Some(events);

        ScrollNode::components()
            .payload(BasicPayloadServiceBuilder::new(ScrollPayloadBuilderBuilder {
                payload_building_time_limit: Duration::from_millis(
                    self.config.sequencer_args.payload_building_duration,
                ),
                best_transactions: (),
                block_da_size_limit: constants::DEFAULT_PAYLOAD_SIZE_LIMIT,
            }))
            .network(
                ScrollNetworkBuilder::new()
                    .with_sub_protocol(scroll_wire_handler.into_rlpx_sub_protocol()),
            )
    }

    fn add_ons(&self) -> Self::AddOns {
        ScrollRollupNodeAddOns::new(
            self.config.clone(),
            self.scroll_wire_events.try_lock().unwrap().take().unwrap(),
        )
    }
}

impl NodeTypes for ScrollRollupNode {
    type Primitives = <ScrollNode as NodeTypes>::Primitives;
    type ChainSpec = <ScrollNode as NodeTypes>::ChainSpec;
    type StateCommitment = <ScrollNode as NodeTypes>::StateCommitment;
    type Storage = <ScrollNode as NodeTypes>::Storage;
    type Payload = <ScrollNode as NodeTypes>::Payload;
}
