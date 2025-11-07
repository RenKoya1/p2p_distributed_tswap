#!/bin/bash

##############################################################################
# Task Metrics Collection Test Script (macOS/Linux Compatible)
# This script automates starting agents, sending tasks, and collecting metrics
#
# Usage:
#   ./test_metrics.sh <num_agents> <num_tasks> [timeout_seconds]
#
# Examples:
#   ./test_metrics.sh 10 100        # 10 agents, 100 tasks, 300s timeout
#   ./test_metrics.sh 50 1000 600   # 50 agents, 1000 tasks, 10 minutes
##############################################################################

NUM_AGENTS=${1:-5}
NUM_TASKS=${2:-50}
TASK_TIMEOUT=${3:-300}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESULTS_DIR="${SCRIPT_DIR}/results/metrics_test_$(date +%Y%m%d_%H%M%S)"
mkdir -p "${RESULTS_DIR}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}"
echo -e "${CYAN}📊  Task Metrics Collection Test${NC}"
echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}"
echo ""
echo "Configuration:"
echo "  Agents: ${NUM_AGENTS}"
echo "  Tasks: ${NUM_TASKS}"
echo "  Timeout: ${TASK_TIMEOUT}s"
echo "  Results: ${RESULTS_DIR}"
echo ""

START_TIME=$(date +%s)

# Cleanup
echo "🧹 Cleaning up existing processes..."
pkill -f "target/debug/manager" 2>/dev/null || true
pkill -f "target/debug/agent" 2>/dev/null || true
sleep 2

# Build
echo "🔨 Building..."
cargo build --bins 2>&1 | grep -E "(Finished|error)" | head -5

# Manager
echo "🔧 Starting manager..."
cargo run --bin manager > "${RESULTS_DIR}/manager.log" 2>&1 &
MANAGER_PID=$!
sleep 3

# Agents
echo "🤖 Starting $NUM_AGENTS agents..."
for i in $(seq 1 $NUM_AGENTS); do
    cargo run --bin agent > "${RESULTS_DIR}/agent_$i.log" 2>&1 &
    if [ $((i % 5)) -eq 0 ]; then
        echo "   ✅ Started $i agents"
    fi
    sleep 0.05
done
echo "   ✅ All agents started"

# Wait for mesh
echo "⏳ Waiting 20s for mesh to form..."
sleep 20

# Send tasks
echo "📋 Sending $NUM_TASKS tasks..."
TASK_START=$(date +%s)

# Use a temporary input file instead of echo pipeline
{
    sleep 2
    echo "tasks ${NUM_TASKS}"
    sleep "${TASK_TIMEOUT}"
} | cargo run --bin manager > "${RESULTS_DIR}/manager_tasks.log" 2>&1 &

TASK_PID=$!

# Wait for completion with timeout
echo "⏳ Waiting up to ${TASK_TIMEOUT}s for tasks..."
WAIT_TIME=0
WAIT_INTERVAL=5
while [ $WAIT_TIME -lt $TASK_TIMEOUT ]; do
    if ! kill -0 $TASK_PID 2>/dev/null; then
        echo "Task process finished naturally"
        break
    fi
    remaining=$((TASK_TIMEOUT - WAIT_TIME))
    echo -ne "\r  ⏳ ${remaining}s remaining...  "
    sleep $WAIT_INTERVAL
    WAIT_TIME=$((WAIT_TIME + WAIT_INTERVAL))
done

# Force kill if still running
if kill -0 $TASK_PID 2>/dev/null; then
    echo ""
    echo "⏱️  Timeout reached, stopping task process..."
    kill -9 $TASK_PID 2>/dev/null || true
fi

TASK_END=$(date +%s)
TASK_DURATION=$((TASK_END - TASK_START))

echo ""
sleep 3

# Collect metrics
echo "📊 Collecting metrics..."
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
CSV_FILE="${RESULTS_DIR}/metrics_${TIMESTAMP}.csv"

# New manager instance to collect metrics
{
    sleep 1
    echo "metrics"
    sleep 1
    echo "save ${CSV_FILE}"
    sleep 1
} | cargo run --bin manager > "${RESULTS_DIR}/manager_metrics.log" 2>&1 &

METRICS_PID=$!

# Wait for metrics with timeout (30 seconds)
METRICS_TIMEOUT=30
echo "⏳ Waiting for metrics (up to ${METRICS_TIMEOUT}s)..."
METRICS_WAIT=0
while [ $METRICS_WAIT -lt $METRICS_TIMEOUT ]; do
    if ! kill -0 $METRICS_PID 2>/dev/null; then
        echo "✅ Metrics collected successfully"
        break
    fi
    sleep 2
    METRICS_WAIT=$((METRICS_WAIT + 2))
done

# Force kill if still running
if kill -0 $METRICS_PID 2>/dev/null; then
    echo "⏱️  Metrics timeout, forcing shutdown..."
    kill -9 $METRICS_PID 2>/dev/null || true
fi

sleep 2

END_TIME=$(date +%s)
TOTAL_TIME=$((END_TIME - START_TIME))

echo ""
echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}✅ Test Complete!${NC}"
echo -e "${BLUE}════════════════════════════════════════════════════════════════${NC}"
echo ""
echo "📊 Results:"

if [ -f "${CSV_FILE}" ]; then
    COUNT=$(tail -n +2 "${CSV_FILE}" 2>/dev/null | wc -l)
    echo -e "${GREEN}✅ CSV saved: metrics_${TIMESTAMP}.csv${NC}"
    echo "   Records: ${COUNT}"
    echo ""
    echo "   Sample data:"
    head -4 "${CSV_FILE}" | tail -3 | sed 's/^/   /'
else
    echo -e "${RED}❌ CSV not found: ${CSV_FILE}${NC}"
fi

echo ""
echo "⏱️  Timing:"
echo "  Task Duration: ${TASK_DURATION}s"
echo "  Total Duration: ${TOTAL_TIME}s"
echo ""
echo "📁 Output files:"
echo "  - Results dir: ${RESULTS_DIR}"
echo "  - Manager log: manager.log"
echo "  - Agent logs: agent_*.log"
echo ""

# Cleanup
echo "🧹 Cleaning up..."
pkill -f "target/debug/manager" 2>/dev/null || true
pkill -f "target/debug/agent" 2>/dev/null || true

echo -e "${GREEN}Done!${NC}"
