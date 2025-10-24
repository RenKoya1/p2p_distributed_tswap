#!/bin/bash

# P2P Distributed TSWAP - Multi-Agent Test Script
# This script runs 1 manager and N agents, then executes 3*N tasks

set -e

# Default values
NUM_AGENTS=3
WAIT_TIME=60
MAP_SIZE=100

# Parse arguments
if [ $# -ge 1 ]; then
    NUM_AGENTS=$1
fi

if [ $# -ge 2 ]; then
    WAIT_TIME=$2
fi

NUM_TASKS=$((NUM_AGENTS * 3))

echo "╔═══════════════════════════════════════════╗"
echo "║  P2P Distributed TSWAP - Multi-Agent Test ║"
echo "╚═══════════════════════════════════════════╝"
echo ""
echo "📊 Configuration:"
echo "   • Agents: $NUM_AGENTS"
echo "   • Tasks: $NUM_TASKS (3 × $NUM_AGENTS)"
echo "   • Map size: ${MAP_SIZE}×${MAP_SIZE}"
echo "   • Wait time: ${WAIT_TIME}s"
echo ""

# Build the project
echo "🔨 Building project..."
cargo build --release --bin manager --bin agent
echo "✅ Build complete"
echo ""

# Function to cleanup on exit
cleanup() {
    echo ""
    echo "🧹 Cleaning up processes..."
    pkill -f "target/release/manager" 2>/dev/null || true
    pkill -f "target/release/agent" 2>/dev/null || true
    echo "✅ Cleanup complete"
    exit 0
}

trap cleanup SIGINT SIGTERM EXIT

# Start manager
echo "🚀 Starting Manager..."
./target/release/manager > logs/manager.log 2>&1 &
MANAGER_PID=$!
echo "✅ Manager started (PID: $MANAGER_PID)"
sleep 3

# Start agents
echo "🚀 Starting $NUM_AGENTS agents..."
mkdir -p logs

for i in $(seq 1 $NUM_AGENTS); do
    ./target/release/agent > logs/agent_$i.log 2>&1 &
    AGENT_PID=$!
    echo "✅ Agent $i started (PID: $AGENT_PID)"
    sleep 0.5
done

# Wait for agents to connect
echo "⏳ Waiting for agents to connect..."
sleep 5

# Generate and display tasks
echo ""
echo "═══════════════════════════════════════════"
echo "📋 GENERATED TASKS ($NUM_TASKS tasks):"
echo "═══════════════════════════════════════════"
echo ""
echo "To execute tasks, copy and paste these commands into the manager terminal:"
echo ""

for i in $(seq 0 $((NUM_TASKS - 1))); do
    PICKUP_X=$(( (i * 3 + 1) % MAP_SIZE ))
    PICKUP_Y=$(( (i * 2 + 1) % MAP_SIZE ))
    DELIVERY_X=$(( (i * 4 + 10) % MAP_SIZE ))
    DELIVERY_Y=$(( (i * 3 + 10) % MAP_SIZE ))
    
    echo "task $PICKUP_X $PICKUP_Y $DELIVERY_X $DELIVERY_Y"
done

echo ""
echo "═══════════════════════════════════════════"
echo ""
echo "📊 Monitoring logs:"
echo "   • Manager: tail -f logs/manager.log"
echo "   • Agent 1: tail -f logs/agent_1.log"
echo "   • All agents: tail -f logs/agent_*.log"
echo ""
echo "⏳ Test will run for $WAIT_TIME seconds..."
echo "   Press Ctrl+C to stop early"
echo ""

# Wait for completion
sleep $WAIT_TIME

echo ""
echo "✅ Test completed!"
echo ""
echo "📈 To analyze results, check:"
echo "   • Goal swaps: grep -r 'GOAL_SWAP' logs/"
echo "   • Task completion: grep -r 'Task completed' logs/"
echo "   • Movements: grep -r 'TSWAP.*Moving' logs/"

# Cleanup will be called by trap
