#!/bin/bash

# このスクリプトは、既存のピアをクリアしてからManagerを起動します

echo "🧹 [1/3] Terminating all existing agent and manager processes..."
pkill -f "target/debug/agent"
pkill -f "target/debug/manager"

# プロセスが完全に終了するまで待機
sleep 2

echo "🧹 [2/3] Verifying all processes are terminated..."
REMAINING=$(ps aux | grep -E "target/debug/(agent|manager)" | grep -v grep | wc -l)
if [ $REMAINING -eq 0 ]; then
    echo "✅ All processes terminated successfully"
else
    echo "⚠️  Warning: $REMAINING processes still running, forcing kill..."
    pkill -9 -f "target/debug/agent"
    pkill -9 -f "target/debug/manager"
    sleep 1
fi　

echo "🧹 [3/3] Starting Manager in CLEAN mode (ignoring old mDNS peers)..."
echo ""
echo "========================================="
echo "Manager starting with --clean flag"
echo "This will ignore any previously discovered peers"
echo "========================================="
echo ""

# Managerを--cleanフラグ付きで起動
cd /Users/renkoya/Library/Mobile\ Documents/com~apple~CloudDocs/CS/Lab/p2p_distributed_tswap
cargo run --bin manager -- --clean
