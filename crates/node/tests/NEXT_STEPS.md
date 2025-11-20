# Next Steps for Issue #420 Implementation

## Summary

你已经完成了 Issue #420 的基础设施搭建：

✅ **已完成的工作**:
1. Anvil 集成到 TestFixture
2. 准备了测试数据（anvil_state.json, test_transactions.json）
3. 创建了完整的测试框架（l1_multi_mode.rs）
4. 文档化了所有测试场景

## 立即需要做的事情

### 1. 修复编译错误

```bash
# 清理并重新编译
cd /Users/yiweichi/Scroll/rollup-node
cargo clean
cargo build --tests

# 运行测试
cargo test --test l1_multi_mode
```

**预期问题**: Proc macro ABI 不匹配（Rust 工具链版本问题）

**解决方案**: 使用项目指定的 Rust 工具链版本重新编译。

### 2. 完善 Anvil 状态加载

当前 `spawn_anvil` 函数不支持从文件加载状态。需要实现：

**文件**: `crates/node/src/test_utils/fixture.rs`

```rust
async fn spawn_anvil(
    state_path: Option<&std::path::Path>,
    chain_id: Option<u64>,
    block_time: Option<u64>,
) -> eyre::Result<anvil::NodeHandle> {
    let mut config = anvil::NodeConfig::default();
    
    if let Some(id) = chain_id {
        config.chain_id = Some(id);
    }
    
    if let Some(time) = block_time {
        config.block_time = Some(std::time::Duration::from_secs(time));
    }
    
    // TODO: 实现状态加载
    // 研究 anvil crate 的正确 API
    // 可能需要使用 alloy_node_bindings 或其他方式
    if let Some(path) = state_path {
        // config.load_state = Some(path.to_path_buf());
        tracing::warn!("State loading not yet implemented");
    }
    
    let (_api, handle) = anvil::spawn(config).await;
    Ok(handle)
}
```

### 3. 添加缺失的事件断言方法

**文件**: `crates/node/src/test_utils/event_utils.rs`

需要添加：

```rust
impl EventAssertions {
    /// Wait for a batch reverted event.
    pub async fn batch_reverted(mut self) -> eyre::Result<()> {
        loop {
            let event = self.rx.recv().await.ok_or_else(|| eyre::eyre!("Channel closed"))?;
            match event {
                ChainOrchestratorEvent::BatchReverted(_) => return Ok(()),
                _ => continue,
            }
        }
    }
    
    /// Wait for an L1 reorg event.
    pub async fn l1_reorg(mut self) -> eyre::Result<()> {
        loop {
            let event = self.rx.recv().await.ok_or_else(|| eyre::eyre!("Channel closed"))?;
            match event {
                ChainOrchestratorEvent::L1Reorg(_) => return Ok(()),
                _ => continue,
            }
        }
    }
}
```

### 4. 使用真实的 L1 合约交互

利用 `test_transactions.json` 中的数据，实现与 Anvil 上部署的合约的真实交互：

```rust
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::TransactionRequest;

#[tokio::test]
async fn test_real_batch_commit_from_l1_contract() -> eyre::Result<()> {
    // 1. 启动带 Anvil 的 fixture
    let mut fixture = TestFixture::builder()
        .sequencer()
        .with_anvil_default_state()
        .with_anvil_chain_id(1337)
        .build()
        .await?;
    
    // 2. 获取 Anvil provider
    let anvil = fixture.anvil.as_ref().unwrap();
    // TODO: 获取 anvil endpoint
    
    // 3. 从 test_transactions.json 加载交易
    let tx_json = std::fs::read_to_string("crates/node/tests/testdata/test_transactions.json")?;
    let transactions: serde_json::Value = serde_json::from_str(&tx_json)?;
    
    // 4. 发送 BatchCommit 交易到 ScrollChain 合约
    // let batch_commit_tx = transactions["batch_commit_0"].as_str().unwrap();
    
    // 5. 等待 rollup node 处理事件
    fixture.expect_event().batch_consolidated().await?;
    
    Ok(())
}
```

## 中期目标

### 1. 实现节点重启测试

需要添加：
- 持久化数据库支持
- 节点停止/重启功能
- 重启后的状态验证

### 2. 完整的重组测试覆盖

添加测试：
- BatchRevert 的重组处理
- BatchRangeReverted 的重组处理
- 多个连续重组的处理

### 3. 性能和压力测试

- 大量批次的处理
- 频繁的重组场景
- 长时间运行的稳定性

## 测试运行清单

使用这个清单来跟踪测试进度：

```bash
# 1. 基础功能测试
cargo test --test l1_multi_mode test_batch_commit_while_syncing
cargo test --test l1_multi_mode test_batch_commit_while_synced
cargo test --test l1_multi_mode test_batch_finalized_while_syncing
cargo test --test l1_multi_mode test_batch_finalized_while_synced
cargo test --test l1_multi_mode test_batch_revert_while_syncing
cargo test --test l1_multi_mode test_batch_revert_while_synced

# 2. 重组测试
cargo test --test l1_multi_mode test_l1_reorg_batch_commit
cargo test --test l1_multi_mode test_l1_reorg_batch_finalized_has_no_effect

# 3. Anvil 集成测试（需要实现）
cargo test --test l1_multi_mode test_with_anvil_l1_events -- --ignored

# 4. 节点重启测试（需要实现）
cargo test --test l1_multi_mode test_node_restart_after_l1_reorg
```

## 潜在的问题和解决方案

### 问题 1: Anvil 状态文件格式

**问题**: `anvil_state.json` 文件非常大（593k tokens），可能格式不兼容。

**解决方案**: 
1. 检查 Anvil 支持的状态文件格式
2. 可能需要转换或简化状态文件
3. 考虑使用 Anvil 的 `--dump-state` 命令生成兼容的格式

### 问题 2: 测试交易数据格式

**问题**: `test_transactions.json` 包含原始交易数据，需要正确解析和发送。

**解决方案**:
1. 使用 `alloy` 的 transaction 类型解析
2. 确保交易签名正确
3. 使用正确的 nonce 和 gas 设置

### 问题 3: L1 Watcher 与 Anvil 的集成

**问题**: L1 Watcher 需要连接到 Anvil 实例。

**解决方案**:
1. 在 TestFixture 中跟踪 Anvil 的 endpoint
2. 配置 rollup node 的 `--l1.url` 指向 Anvil
3. 确保 L1 合约地址与 `anvil.env` 中的地址匹配

## 相关代码文件

**测试文件**:
- `crates/node/tests/l1_multi_mode.rs` - 主测试文件
- `crates/node/tests/L1_MULTI_MODE_TESTS.md` - 详细文档

**测试数据**:
- `tests/anvil_state.json` - Anvil 初始状态
- `tests/anvil.env` - 合约地址配置
- `crates/node/tests/testdata/test_transactions.json` - 测试交易
- `crates/node/tests/testdata/batch_0_calldata.bin` - 批次数据
- `crates/node/tests/testdata/batch_1_calldata.bin` - 批次数据

**基础设施**:
- `crates/node/src/test_utils/fixture.rs` - TestFixture 实现
- `crates/node/src/test_utils/l1_helpers.rs` - L1 事件辅助函数
- `crates/node/src/test_utils/event_utils.rs` - 事件断言

## 与团队协作

### 需要讨论的问题

1. **Anvil 状态加载**: 最佳方式是什么？是否需要自定义实现？
2. **节点重启**: 是否需要完整的持久化支持？还是可以用其他方式测试？
3. **测试数据**: `test_transactions.json` 的具体用法和格式要求？

### 可以并行进行的工作

- ✅ 测试框架已完成，可以开始实现缺失的辅助方法
- ⚠️ Anvil 集成需要先解决状态加载问题
- ⚠️ 真实合约交互需要 Anvil 完全工作

## 总结

你已经完成了 Issue #420 的大部分基础工作。下一步的关键任务是：

1. **修复编译问题** (最高优先级)
2. **完善 Anvil 状态加载** (阻塞项)
3. **添加缺失的事件断言** (简单任务)
4. **实现真实 L1 交互** (核心功能)

完成这些步骤后，你将拥有一个全面的 L1 多模式测试套件，完全满足 Issue #420 的要求。

祝顺利！🚀


