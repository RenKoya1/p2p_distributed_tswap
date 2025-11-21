#!/bin/bash
set -euo pipefail

##############################################################################
# Centralized MAPD System Test (time-boxed with metric export)
##############################################################################

NUM_AGENTS=${1:-3}
DURATION_SECS=${2:-20}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESULTS_DIR="${SCRIPT_DIR}/results/centralized_${NUM_AGENTS}agents_$(date +%Y%m%d_%H%M%S)"
mkdir -p "${RESULTS_DIR}"

TASK_CSV="${RESULTS_DIR}/task_metrics.csv"
PATH_CSV="${RESULTS_DIR}/path_metrics.csv"
MANAGER_LOG="${RESULTS_DIR}/manager.log"
MANAGER_FIFO="${RESULTS_DIR}/manager_input"

AGENT_PIDS=()

cleanup() {
    set +e
    [[ -n "${SEND_PID:-}" ]] && kill "${SEND_PID}" 2>/dev/null || true
    for pid in "${AGENT_PIDS[@]:-}"; do
        kill "${pid}" 2>/dev/null || true
    done
    [[ -n "${MANAGER_PID:-}" ]] && kill "${MANAGER_PID}" 2>/dev/null || true
    rm -f "${MANAGER_FIFO}"
}
trap cleanup EXIT

echo ""
echo "🏢 Centralized MAPD Test"
echo "   Agents   : ${NUM_AGENTS}"
echo "   Duration : ${DURATION_SECS}s"
echo "   Results  : ${RESULTS_DIR}"
echo ""

echo "📦 Building binaries..."
cargo build --quiet --bin manager-centralized --bin agent-centralized

echo "🧵 Preparing manager input pipe..."
rm -f "${MANAGER_FIFO}"
mkfifo "${MANAGER_FIFO}"

echo "🏁 Launching manager..."
cargo run --quiet --bin manager-centralized < "${MANAGER_FIFO}" > "${MANAGER_LOG}" 2>&1 &
MANAGER_PID=$!
sleep 2

echo "🤖 Launching ${NUM_AGENTS} agents..."
for i in $(seq 1 "${NUM_AGENTS}"); do
    
    cargo run --quiet --bin agent-centralized > "${RESULTS_DIR}/agent_${i}.log" 2>&1 &
    AGENT_PIDS+=($!)
    # エージェント起動間隔を調整（200以上の場合は遅延を増やす）
    if [[ ${NUM_AGENTS} -gt 100 ]]; then
        sleep 0.2  # 大規模な場合200ms
    else
        sleep 0.1  # 小規模な場合100ms
    fi
done
echo "   ✅ Agents ready"

# Calculate warmup based on number of agents
# Formula: agent_launch_time + mesh_formation_time + buffer
AGENT_LAUNCH_TIME=$(( (NUM_AGENTS * 2) / 10 ))  # 0.2s per agent for large scale
WARMUP_SECS=$((AGENT_LAUNCH_TIME + 30))  # Add 30s for mesh formation
echo "⏳ Waiting ${WARMUP_SECS}s for agents to publish positions..."
sleep "${WARMUP_SECS}"

echo "🚚 Dispatching tasks for ${DURATION_SECS}s..."
(
    echo "tasks ${NUM_AGENTS}"
    sleep 2
    while true; do
        echo "task"
        sleep 2
    done
) > "${MANAGER_FIFO}" &
SEND_PID=$!

sleep "${DURATION_SECS}"
kill "${SEND_PID}" 2>/dev/null || true

echo "📝 Collecting metrics..."
{
    echo "metrics"
    sleep 2
    echo "save ${TASK_CSV}"
    sleep 2
    echo "save path ${PATH_CSV}"
    sleep 3
} > "${MANAGER_FIFO}"

sleep 8

SUMMARY_FILE="${RESULTS_DIR}/test_summary.txt"

echo ""
echo "📊 Test summary"
{
    echo "=========================================="
    echo "Centralized MAPD Test Summary"
    echo "=========================================="
    echo "Test Configuration:"
    echo "  - Agents       : ${NUM_AGENTS}"
    echo "  - Duration     : ${DURATION_SECS}s"
    echo "  - Timestamp    : $(date)"
    echo ""
    echo "Results:"
} > "${SUMMARY_FILE}"

if [[ -f "${TASK_CSV}" ]]; then
    TASK_COUNT=$(tail -n +2 "${TASK_CSV}" | wc -l | tr -d ' ')
    AVG_TOTAL=$(awk -F',' 'NR>1 && $7 != "" {sum+=$7; count++} END {if(count>0) printf "%.1f", sum/count/1000;}' "${TASK_CSV}")
    THROUGHPUT=$(awk -v dur="${DURATION_SECS}" -v cnt="${TASK_COUNT:-0}" 'BEGIN {if(dur>0) printf "%.2f", cnt/dur; else print "0.00";}')
    echo "   ✅ Completed tasks : ${TASK_COUNT}"
    echo "   ⚡ Throughput      : ${THROUGHPUT} tasks/sec"
    {
        echo "  - Completed tasks  : ${TASK_COUNT}"
        echo "  - Throughput       : ${THROUGHPUT} tasks/sec"
    } >> "${SUMMARY_FILE}"
    if [[ -n "${AVG_TOTAL}" ]]; then
        echo "   🕒 Avg task latency : ${AVG_TOTAL} s"
        echo "  - Avg task latency : ${AVG_TOTAL} s" >> "${SUMMARY_FILE}"
    fi
else
    echo "   ⚠️  Task metrics CSV missing (${TASK_CSV})"
    echo "  - Task metrics CSV : MISSING" >> "${SUMMARY_FILE}"
fi

if [[ -f "${PATH_CSV}" ]]; then
    PATH_COUNT=$(tail -n +2 "${PATH_CSV}" | wc -l | tr -d ' ')
    AVG_PATH=$(awk -F',' 'NR>1 {sum+=$3; count++} END {if(count>0) printf "%.3f", sum/count;}' "${PATH_CSV}")
    echo "   ⏱️  Path samples    : ${PATH_COUNT:-0}"
    {
        echo "  - Path samples     : ${PATH_COUNT:-0}"
    } >> "${SUMMARY_FILE}"
    if [[ -n "${AVG_PATH}" ]]; then
        echo "   ⏱️  Avg plan time    : ${AVG_PATH} ms"
        echo "  - Avg plan time    : ${AVG_PATH} ms" >> "${SUMMARY_FILE}"
    fi
else
    echo "   ⚠️  Path metrics CSV missing (${PATH_CSV})"
    echo "  - Path metrics CSV : MISSING" >> "${SUMMARY_FILE}"
fi

{
    echo ""
    echo "Files:"
    echo "  - Task metrics : ${TASK_CSV}"
    echo "  - Path metrics : ${PATH_CSV}"
    echo "  - Manager log  : ${MANAGER_LOG}"
    echo "=========================================="
} >> "${SUMMARY_FILE}"

echo ""
echo "Logs and CSVs stored in ${RESULTS_DIR}"
echo "Summary saved to ${SUMMARY_FILE}"
