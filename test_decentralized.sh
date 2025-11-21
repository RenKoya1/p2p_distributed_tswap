#!/bin/bash
set -euo pipefail

##############################################################################
# Decentralized MAPD System Test (TSWAP-based with metric export)
##############################################################################

NUM_AGENTS=${1:-3}
DURATION_SECS=${2:-20}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RESULTS_DIR="${SCRIPT_DIR}/results/decentralized_${NUM_AGENTS}agents_$(date +%Y%m%d_%H%M%S)"
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
    # Manager will be shut down gracefully by quit command in main script
    # Only kill it here if script fails prematurely
    if [[ "${GRACEFUL_SHUTDOWN:-}" != "true" ]]; then
        [[ -n "${MANAGER_PID:-}" ]] && kill "${MANAGER_PID}" 2>/dev/null || true
    fi
    # Close file descriptor 3 if still open
    exec 3>&- 2>/dev/null || true
    rm -f "${MANAGER_FIFO}"
}
trap cleanup EXIT

echo ""
echo "🌐 Decentralized MAPD Test (TSWAP)"
echo "   Agents   : ${NUM_AGENTS}"
echo "   Duration : ${DURATION_SECS}s"
echo "   TSWAP Radius: 15 (Manhattan distance)"
echo "   Results  : ${RESULTS_DIR}"
echo ""

echo "📦 Building binaries..."
cargo build --quiet --bin manager-decentralized --bin agent-decentralized

echo "🧵 Preparing manager input pipe..."
rm -f "${MANAGER_FIFO}"
mkfifo "${MANAGER_FIFO}"

# Keep FIFO open with a background holder process
exec 3<>"${MANAGER_FIFO}"  # File descriptor 3 keeps the FIFO open

echo "🏁 Launching manager..."
TASK_CSV_PATH="${TASK_CSV}" PATH_CSV_PATH="${PATH_CSV}" \
    cargo run --quiet --bin manager-decentralized <&3 > "${MANAGER_LOG}" 2>&1 &
MANAGER_PID=$!
sleep 2

echo "🤖 Launching ${NUM_AGENTS} agents..."
for ((i=1; i<=NUM_AGENTS; i++)); do
    cargo run --quiet --bin agent-decentralized > "${RESULTS_DIR}/agent_${i}.log" 2>&1 &
    AGENT_PIDS+=($!)
    
    # エージェント起動の間隔を調整（ネットワーク負荷軽減）
    if (( NUM_AGENTS >= 30 )); then
        sleep 0.2  # 大規模な場合200ms
    else
        sleep 0.1  # 小規模な場合100ms
    fi
done

echo "✅ Launched ${NUM_AGENTS} agents (PIDs: ${AGENT_PIDS[*]})"

# エージェント起動にかかった時間に基づいてウォームアップ時間を計算
AGENT_LAUNCH_TIME=$(( (NUM_AGENTS * 2) / 10 ))  # 0.2s per agent for large scale
WARMUP_SECS=$(( 10 + AGENT_LAUNCH_TIME ))  # エージェント初期化7秒 + 余裕3秒
echo "⏳ Waiting ${WARMUP_SECS}s for agents to connect and mesh to form..."
sleep "${WARMUP_SECS}"

# マネージャーにタスクを送信（継続的に）
echo "📤 Sending continuous tasks for ${DURATION_SECS}s..."
{
    # 最初にまとめて送る
    echo "tasks ${NUM_AGENTS}"
    sleep 5  # 初期タスクが配布されるのを待つ
    
    # その後、定期的に追加タスクを送信
    ELAPSED=5
    while (( ELAPSED < DURATION_SECS )); do
        sleep 3
        echo "tasks ${NUM_AGENTS}"
        ELAPSED=$((ELAPSED + 3))
    done
} >&3 &
SEND_PID=$!

echo "⏱️  Running for ${DURATION_SECS}s..."
sleep "${DURATION_SECS}"

echo "🛑 Stopping task sender..."
kill "${SEND_PID}" 2>/dev/null || true

echo "⏳ Waiting 3s for final tasks to complete..."
sleep 3

# Stop agents first
echo "🛑 Stopping agents..."
for pid in "${AGENT_PIDS[@]:-}"; do
    kill "${pid}" 2>/dev/null || true
done
sleep 1

# メトリクスを収集して終了
echo "📊 Collecting metrics..."
{
    echo "metrics"
    sleep 1
    echo "save ${TASK_CSV}"
    sleep 1
    echo "save path ${PATH_CSV}"
    sleep 1
    echo "quit"
} >&3

# Close the FIFO file descriptor
exec 3>&-

METRICS_PID=""

# Wait for manager to finish gracefully (with timeout)
timeout=10
elapsed=0
while kill -0 "${MANAGER_PID}" 2>/dev/null && (( elapsed < timeout )); do
    sleep 1
    ((elapsed++))
done

# If still running, force kill
if kill -0 "${MANAGER_PID}" 2>/dev/null; then
    echo "⚠️  Manager didn't exit gracefully, forcing termination..."
    kill "${MANAGER_PID}" 2>/dev/null || true
fi

GRACEFUL_SHUTDOWN=true

echo "✅ Metrics saved"

# サマリーを表示
SUMMARY_FILE="${RESULTS_DIR}/test_summary.txt"
{
    echo "========================================="
    echo "Decentralized MAPD Test Summary (TSWAP)"
    echo "========================================="
    echo "Agents      : ${NUM_AGENTS}"
    echo "Duration    : ${DURATION_SECS}s"
    echo "TSWAP Radius: 15"
    echo ""
    echo "Results:"
    echo "---------"
    
    if [[ -f "${TASK_CSV}" ]]; then
        COMPLETED=$(awk -F',' 'NR>1 && $5!="" {count++} END {print count+0}' "${TASK_CSV}")
        echo "   ✅ Completed tasks : ${COMPLETED}"
        
        if (( COMPLETED > 0 )); then
            THROUGHPUT=$(awk -v c="${COMPLETED}" -v d="${DURATION_SECS}" 'BEGIN {printf "%.2f", c/d}')
            echo "   ⚡ Throughput      : ${THROUGHPUT} tasks/sec"
        fi
    fi
    
    if [[ -f "${PATH_CSV}" ]]; then
        SAMPLES=$(awk -F',' 'NR>1 {count++} END {print count+0}' "${PATH_CSV}")
        if (( SAMPLES > 0 )); then
            AVG_TIME=$(awk -F',' 'NR>1 {sum+=$2; count++} END {if(count>0) printf "%.3f", sum/count}' "${PATH_CSV}")
            echo "   ⏱️  Path samples    : ${SAMPLES}"
            echo "   ⏱️  Avg plan time    : ${AVG_TIME} ms (per-agent TSWAP computation)"
        fi
    fi
    
    echo ""
    echo "📁 Detailed results: ${RESULTS_DIR}"
    echo "========================================="
} | tee "${SUMMARY_FILE}"

echo ""
echo "🎉 Test complete!"
echo ""
