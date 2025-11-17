#!/bin/bash
set -euo pipefail

##############################################################################
# Decentralized MAPD Test (time-boxed with metric export)
##############################################################################

NUM_AGENTS=${1:-3}
DURATION_SECS=${2:-20}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESULTS_DIR="${SCRIPT_DIR}/results/decentralized_$(date +%Y%m%d_%H%M%S)"
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
echo "🌐 Decentralized MAPD Test"
echo "   Agents   : ${NUM_AGENTS}"
echo "   Duration : ${DURATION_SECS}s"
echo "   Results  : ${RESULTS_DIR}"
echo ""

echo "📦 Building binaries..."
cargo build --quiet --bin manager-decentralized --bin agent-decentralized

echo "🧵 Preparing manager input pipe..."
rm -f "${MANAGER_FIFO}"
mkfifo "${MANAGER_FIFO}"

echo "🏁 Launching manager (clean mode)..."
cargo run --quiet --bin manager-decentralized -- --clean < "${MANAGER_FIFO}" > "${MANAGER_LOG}" 2>&1 &
MANAGER_PID=$!
sleep 3

echo "🤖 Launching ${NUM_AGENTS} agents..."
for i in $(seq 1 "${NUM_AGENTS}"); do
    cargo run --quiet --bin agent-decentralized > "${RESULTS_DIR}/agent_${i}.log" 2>&1 &
    AGENT_PIDS+=($!)
    sleep 0.1
done
echo "   ✅ Agents ready"

echo "⏳ Allowing mesh formation..."
sleep 10

echo "🚚 Dispatching tasks for ${DURATION_SECS}s..."
(
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
    sleep 1
    echo "save ${TASK_CSV}"
    sleep 0.5
    echo "save path ${PATH_CSV}"
    sleep 1
} > "${MANAGER_FIFO}"

sleep 3

echo ""
echo "📊 Test summary"
if [[ -f "${TASK_CSV}" ]]; then
    TASK_COUNT=$(tail -n +2 "${TASK_CSV}" | wc -l | tr -d ' ')
    AVG_TOTAL=$(awk -F',' 'NR>1 && $7 != "" {sum+=$7; count++} END {if(count>0) printf "%.1f", sum/count/1000;}' "${TASK_CSV}")
    THROUGHPUT=$(awk -v dur="${DURATION_SECS}" -v cnt="${TASK_COUNT:-0}" 'BEGIN {if(dur>0) printf "%.2f", cnt/dur; else print "0.00";}')
    echo "   ✅ Completed tasks : ${TASK_COUNT}"
    echo "   ⚡ Throughput      : ${THROUGHPUT} tasks/sec"
    if [[ -n "${AVG_TOTAL}" ]]; then
        echo "   🕒 Avg task latency : ${AVG_TOTAL} s"
    fi
else
    echo "   ⚠️  Task metrics CSV missing (${TASK_CSV})"
fi

if [[ -f "${PATH_CSV}" ]]; then
    PATH_COUNT=$(tail -n +2 "${PATH_CSV}" | wc -l | tr -d ' ')
    AVG_PATH=$(awk -F',' 'NR>1 {sum+=$3; count++} END {if(count>0) printf "%.3f", sum/count;}' "${PATH_CSV}")
    echo "   ⏱️  Path samples    : ${PATH_COUNT:-0}"
    if [[ -n "${AVG_PATH}" ]]; then
        echo "   ⏱️  Avg plan time    : ${AVG_PATH} ms"
    fi
else
    echo "   ⚠️  Path metrics CSV missing (${PATH_CSV})"
fi

echo ""
echo "Logs and CSVs stored in ${RESULTS_DIR}"
