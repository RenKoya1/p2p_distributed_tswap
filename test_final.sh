#!/bin/bash

##############################################################################
# Task Metrics Test Script - Time-based execution
##############################################################################

NUM_AGENTS=${1:-3}
DURATION_SECS=${2:-30}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESULTS_DIR="${SCRIPT_DIR}/results/metrics_test_$(date +%Y%m%d_%H%M%S)"
mkdir -p "${RESULTS_DIR}"

echo ""
echo "🚀 Starting Task Metrics Test (Time-based)"
echo "   Agents: $NUM_AGENTS, Duration: ${DURATION_SECS}s"
echo ""

# Cleanup - Terminate all existing processes
echo "🧹 [Step 1/3] Terminating all existing agent and manager processes..."
pkill -f "target/debug/agent" 2>/dev/null || true
pkill -f "target/debug/manager" 2>/dev/null || true
pkill -9 -f "cargo run" 2>/dev/null || true
sleep 2

# Verify cleanup
REMAINING=$(ps aux | grep -E "target/debug/(agent|manager)" | grep -v grep | wc -l)
if [ $REMAINING -eq 0 ]; then
    echo "✅ All processes terminated successfully"
else
    echo "⚠️  Warning: $REMAINING processes still running, forcing kill..."
    pkill -9 -f "target/debug/agent" 2>/dev/null || true
    pkill -9 -f "target/debug/manager" 2>/dev/null || true
    sleep 1
fi

echo "🧹 [Step 2/3] Verifying clean state..."
sleep 2

# Build
echo "📦 [Step 3/3] Building..."
cargo build --bins 2>&1 | grep -E "Finished|error"

# Start manager with --clean flag to ignore old mDNS peers
echo "🔧 Starting manager in CLEAN mode (ignoring old peers)..."
cargo run --bin manager -- --clean > "${RESULTS_DIR}/manager.log" 2>&1 &
sleep 3

# Start agents
echo "🤖 Starting $NUM_AGENTS agents..."
for i in $(seq 1 $NUM_AGENTS); do
    cargo run --bin agent > "${RESULTS_DIR}/agent_$i.log" 2>&1 &
    sleep 0.1
done
echo "   ✅ Started $NUM_AGENTS agents"

# Wait for mesh
echo "⏳ Waiting 15s for mesh..."
sleep 15

# Send tasks
echo "📋 Starting continuous task sending for ${DURATION_SECS} seconds..."
CSV_TIME=$(date +%Y%m%d_%H%M%S)
CSV_PATH="${RESULTS_DIR}/metrics_${CSV_TIME}.csv"

# Use a named pipe for manager input
MANAGER_FIFO="${RESULTS_DIR}/manager_input"
mkfifo "$MANAGER_FIFO" 2>/dev/null || true

# Restart manager with stdin from FIFO
pkill -9 -f "cargo run --bin manager" 2>/dev/null || true
sleep 1
cargo run --bin manager -- --clean < "$MANAGER_FIFO" >> "${RESULTS_DIR}/manager.log" 2>&1 &
MANAGER_PID=$!
sleep 2

# Send commands - start sending tasks after a delay
(
    sleep 3
    # 無限にタスクを送信し続ける
    for ((i=0; i<$((DURATION_SECS/2)); i++)); do
        echo "task"
        sleep 2
    done
) > "$MANAGER_FIFO" &

SEND_PID=$!

# Wait for specified duration
echo "📋 Running for ${DURATION_SECS} seconds, sending tasks continuously..."
sleep "${DURATION_SECS}"

# Send metrics and save command before killing
{
    sleep 1
    echo "metrics"
    sleep 1
    echo "save $CSV_PATH"
    sleep 2
} > "$MANAGER_FIFO" &

METRICS_PID=$!
sleep 5

# Kill processes
kill -9 $MANAGER_PID $SEND_PID $METRICS_PID 2>/dev/null || true
sleep 2
rm -f "$MANAGER_FIFO"

# Cleanup
echo ""
echo "🧹 Cleaning up..."
pkill -9 -f "cargo run" 2>/dev/null || true
sleep 2

# Results
echo ""
echo "✅ Done!"
echo ""

# Wait a bit for CSV to be written
sleep 2

if [ -f "$CSV_PATH" ]; then
    COUNT=$(tail -n +2 "$CSV_PATH" 2>/dev/null | wc -l)
    THROUGHPUT=$(echo "scale=2; $COUNT / $DURATION_SECS" | bc)
    echo "📊 Results saved: $CSV_PATH"
    echo "   ✅ Completed tasks: $COUNT"
    echo "   ⚡ Throughput: $THROUGHPUT tasks/sec"
    echo "   ⏱️  Duration: ${DURATION_SECS}s"
    echo "   🤖 Agents: $NUM_AGENTS"
    if [ $COUNT -gt 0 ]; then
        echo ""
        echo "   Sample data (first 2 rows):"
        head -3 "$CSV_PATH" | tail -2 | sed 's/^/      /'
    fi
else
    echo "⚠️  No CSV found: $CSV_PATH"
    echo ""
    echo "Manager log (last 30 lines):"
    tail -30 "${RESULTS_DIR}/manager.log" | sed 's/^/  /'
fi

echo ""
