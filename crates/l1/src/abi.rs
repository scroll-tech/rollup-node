use alloy_sol_types::sol;

// L1 Message Queue Contract
sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    L1MessageQueue,
    "abi/L1MessageQueue.json",
);

// Scroll Chain Contract
sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    ScrollChain,
    "abi/ScrollChain.json",
);
