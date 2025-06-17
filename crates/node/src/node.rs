//! Node specific implementations for Scroll rollup node.

use crate::args::ScrollRollupNodeConfig;
use std::time::Duration;

use super::add_ons::ScrollRollupNodeAddOns;
use reth_node_api::NodeTypes;
use reth_node_builder::{
    components::{BasicPayloadServiceBuilder, ComponentsBuilder},
    FullNodeTypes, Node, NodeAdapter, NodeComponentsBuilder,
};
use reth_scroll_node::{
    ScrollConsensusBuilder, ScrollExecutorBuilder, ScrollNetworkBuilder, ScrollNode,
    ScrollPayloadBuilderBuilder, ScrollPoolBuilder,
};

/// The Scroll node implementation.
#[derive(Clone, Debug)]
pub struct ScrollRollupNode {
    config: ScrollRollupNodeConfig,
}

impl ScrollRollupNode {
    /// Create a new instance of [`ScrollRollupNode`].
    pub fn new(config: ScrollRollupNodeConfig) -> Self {
        config
            .validate()
            .map_err(|e| eyre::eyre!("Configuration validation failed: {}", e))
            .expect("Configuration validation failed");
        Self { config }
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
        ScrollNode::components().payload(BasicPayloadServiceBuilder::new(
            ScrollPayloadBuilderBuilder {
                payload_building_time_limit: Duration::from_millis(
                    self.config.sequencer_args.payload_building_duration,
                ),
                best_transactions: (),
            },
        ))
    }

    fn add_ons(&self) -> Self::AddOns {
        ScrollRollupNodeAddOns::new(self.config.clone())
    }
}

impl NodeTypes for ScrollRollupNode {
    type Primitives = <ScrollNode as NodeTypes>::Primitives;
    type ChainSpec = <ScrollNode as NodeTypes>::ChainSpec;
    type StateCommitment = <ScrollNode as NodeTypes>::StateCommitment;
    type Storage = <ScrollNode as NodeTypes>::Storage;
    type Payload = <ScrollNode as NodeTypes>::Payload;
}
