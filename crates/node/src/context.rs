use crate::constants::SCROLL_GAS_LIMIT;
use reth_node_api::{AddOnsContext, FullNodeComponents, FullNodeTypes};
use reth_node_core::cli::config::PayloadBuilderConfig;
use reth_node_types::NodeTypes;
use reth_tasks::TaskExecutor;
use std::{path::PathBuf, sync::Arc};

/// The context passed to `ScrollRollupNodeConfig::build` method.
#[derive(Debug)]
pub struct RollupNodeContext<N, CS> {
    /// The network component of the rollup node.
    pub network: N,
    /// The chain specification of the rollup node.
    pub chain_spec: Arc<CS>,
    /// The datadir of the rollup node.
    pub datadir: PathBuf,
    /// The block gas limit of the rollup node.
    pub block_gas_limit: u64,
    /// The task executor.
    pub task_executor: TaskExecutor,
}

impl<N, CS> RollupNodeContext<N, CS> {
    /// Returns a new instance of the [`RollupNodeContext`].
    pub const fn new(
        network: N,
        chain_spec: Arc<CS>,
        datadir: PathBuf,
        block_gas_limit: u64,
        task_executor: TaskExecutor,
    ) -> Self {
        Self { network, chain_spec, datadir, block_gas_limit, task_executor }
    }
}

impl<N> From<&AddOnsContext<'_, N>>
    for RollupNodeContext<N::Network, <<N as FullNodeTypes>::Types as NodeTypes>::ChainSpec>
where
    N: FullNodeComponents,
{
    fn from(value: &AddOnsContext<'_, N>) -> Self {
        Self {
            network: value.node.network().clone(),
            chain_spec: value.config.chain.clone(),
            datadir: value.config.datadir().db(),
            block_gas_limit: value.config.builder.gas_limit().unwrap_or(SCROLL_GAS_LIMIT),
            task_executor: value.node.task_executor().clone(),
        }
    }
}
