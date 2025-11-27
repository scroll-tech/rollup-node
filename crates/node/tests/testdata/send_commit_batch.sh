#!/bin/bash

# 脚本：发送所有 commitBatch 交易到 Anvil
# 用法: ./send_commit_batch.sh

RPC_URL="http://localhost:8544"
JSON_FILE="test_transactions.json"

# 检查 JSON 文件是否存在
if [ ! -f "$JSON_FILE" ]; then
    echo "错误: 找不到文件 $JSON_FILE"
    exit 1
fi

# 检查是否安装了 jq 和 cast
if ! command -v jq &> /dev/null; then
    echo "错误: 需要安装 jq"
    exit 1
fi

if ! command -v cast &> /dev/null; then
    echo "错误: 需要安装 cast (foundry)"
    exit 1
fi

echo "开始发送 commitBatch 交易..."
echo "RPC URL: $RPC_URL"
echo ""

# 获取所有 commitBatch 的键
keys=$(jq -r '.commitBatch | keys[]' "$JSON_FILE")

# 计数器
count=0
success=0
failed=0

# 遍历所有 commitBatch 交易
for key in $keys; do
    count=$((count + 1))
    echo "[$count] 发送 commitBatch[$key]..."
    
    # 获取原始交易数据
    raw_tx=$(jq -r ".commitBatch[\"$key\"]" "$JSON_FILE")
    
    # 发送交易
    if tx_hash=$(cast rpc --rpc-url "$RPC_URL" eth_sendRawTransaction "$raw_tx" 2>&1); then
        # 提取交易哈希（去掉可能的引号）
        tx_hash=$(echo "$tx_hash" | tr -d '"')
        echo "  ✓ 成功! 交易哈希: $tx_hash"
        success=$((success + 1))
    else
        echo "  ✗ 失败! 错误: $tx_hash"
        failed=$((failed + 1))
    fi
    
    echo ""
    
    # 可选：添加延迟以避免请求过快
    # sleep 0.1
done

echo "================================"
echo "完成!"
echo "总交易数: $count"
echo "成功: $success"
echo "失败: $failed"
echo "================================"


