#!/bin/bash

##############################################################################
# System Comparison Script
# Runs both centralized and decentralized systems and compares performance
##############################################################################

NUM_AGENTS=${1:-3}
DURATION_SECS=${2:-30}

echo ""
echo "🔬 =================================================="
echo "🔬 MAPD System Performance Comparison"
echo "🔬 =================================================="
echo ""
echo "Parameters:"
echo "  - Agents: $NUM_AGENTS"
echo "  - Duration: ${DURATION_SECS}s each"
echo ""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPARISON_DIR="${SCRIPT_DIR}/results/comparison_$(date +%Y%m%d_%H%M%S)"
mkdir -p "${COMPARISON_DIR}"

# Run centralized system
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 Phase 1/2: Testing CENTRALIZED system..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

CENTRALIZED_LOG="${COMPARISON_DIR}/centralized.log"
./test_centralized.sh $NUM_AGENTS $DURATION_SECS > "$CENTRALIZED_LOG" 2>&1
CENTRALIZED_EXIT=$?

# Extract centralized results
if [ -f "$CENTRALIZED_LOG" ]; then
    CENTRAL_TASKS=$(grep "Completed tasks:" "$CENTRALIZED_LOG" | awk '{print $4}')
    CENTRAL_THROUGHPUT=$(grep "Throughput:" "$CENTRALIZED_LOG" | awk '{print $3}')
    CENTRAL_AVG_TIME=$(grep "Average computation time:" "$CENTRALIZED_LOG" | awk '{print $5}')
    CENTRAL_CSV=$(grep "Results saved:" "$CENTRALIZED_LOG" | awk '{print $4}')
fi

echo ""
sleep 3

# Run decentralized system
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 Phase 2/2: Testing DECENTRALIZED system..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

DECENTRALIZED_LOG="${COMPARISON_DIR}/decentralized.log"
./test_decentralized.sh $NUM_AGENTS $DURATION_SECS > "$DECENTRALIZED_LOG" 2>&1
DECENTRALIZED_EXIT=$?

# Extract decentralized results
if [ -f "$DECENTRALIZED_LOG" ]; then
    DECENTRAL_TASKS=$(grep "Completed tasks:" "$DECENTRALIZED_LOG" | awk '{print $4}')
    DECENTRAL_THROUGHPUT=$(grep "Throughput:" "$DECENTRALIZED_LOG" | awk '{print $3}')
    DECENTRAL_AVG_TIME=$(grep "Average computation time:" "$DECENTRALIZED_LOG" | awk '{print $5}')
    DECENTRAL_CSV=$(grep "Results saved:" "$DECENTRALIZED_LOG" | awk '{print $4}')
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 COMPARISON RESULTS"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "┌────────────────────────────────────────────────────┐"
echo "│ Configuration                                      │"
echo "├────────────────────────────────────────────────────┤"
echo "│ Agents:            $NUM_AGENTS"
echo "│ Duration:          ${DURATION_SECS}s"
echo "└────────────────────────────────────────────────────┘"
echo ""
echo "┌────────────────────────────────────────────────────┐"
echo "│                CENTRALIZED SYSTEM                  │"
echo "├────────────────────────────────────────────────────┤"
echo "│ Completed Tasks:   ${CENTRAL_TASKS:-N/A}"
echo "│ Throughput:        ${CENTRAL_THROUGHPUT:-N/A} tasks/sec"
echo "│ Avg Comp Time:     ${CENTRAL_AVG_TIME:-N/A}s"
echo "└────────────────────────────────────────────────────┘"
echo ""
echo "┌────────────────────────────────────────────────────┐"
echo "│               DECENTRALIZED SYSTEM                 │"
echo "├────────────────────────────────────────────────────┤"
echo "│ Completed Tasks:   ${DECENTRAL_TASKS:-N/A}"
echo "│ Throughput:        ${DECENTRAL_THROUGHPUT:-N/A} tasks/sec"
echo "│ Avg Comp Time:     ${DECENTRAL_AVG_TIME:-N/A}s"
echo "└────────────────────────────────────────────────────┘"
echo ""

# Calculate improvements if both have valid data
if [ -n "$CENTRAL_TASKS" ] && [ -n "$DECENTRAL_TASKS" ] && [ "$CENTRAL_TASKS" != "0" ]; then
    TASK_DIFF=$((DECENTRAL_TASKS - CENTRAL_TASKS))
    TASK_PERCENT=$(echo "scale=1; ($DECENTRAL_TASKS - $CENTRAL_TASKS) * 100 / $CENTRAL_TASKS" | bc)
    
    echo "┌────────────────────────────────────────────────────┐"
    echo "│                   ANALYSIS                         │"
    echo "├────────────────────────────────────────────────────┤"
    
    if [ "$DECENTRAL_TASKS" -gt "$CENTRAL_TASKS" ]; then
        echo "│ 🎯 Decentralized completed $TASK_DIFF more tasks"
        echo "│    (${TASK_PERCENT}% improvement)"
    elif [ "$DECENTRAL_TASKS" -lt "$CENTRAL_TASKS" ]; then
        echo "│ 🎯 Centralized completed ${TASK_DIFF#-} more tasks"
        echo "│    (${TASK_PERCENT#-}% better)"
    else
        echo "│ 🎯 Both systems completed same number of tasks"
    fi
    
    # Compare average computation time
    if [ -n "$CENTRAL_AVG_TIME" ] && [ -n "$DECENTRAL_AVG_TIME" ]; then
        TIME_DIFF=$(echo "scale=3; $DECENTRAL_AVG_TIME - $CENTRAL_AVG_TIME" | bc)
        TIME_PERCENT=$(echo "scale=1; ($DECENTRAL_AVG_TIME - $CENTRAL_AVG_TIME) * 100 / $CENTRAL_AVG_TIME" | bc)
        
        echo "│"
        if (( $(echo "$DECENTRAL_AVG_TIME < $CENTRAL_AVG_TIME" | bc -l) )); then
            echo "│ ⚡ Decentralized is ${TIME_DIFF#-}s faster per task"
            echo "│    (${TIME_PERCENT#-}% faster computation)"
        else
            echo "│ ⚡ Centralized is ${TIME_DIFF}s faster per task"
            echo "│    (${TIME_PERCENT}% slower in decentralized)"
        fi
    fi
    
    echo "└────────────────────────────────────────────────────┘"
fi

echo ""
echo "📁 Detailed logs saved to:"
echo "   - Centralized:  $CENTRALIZED_LOG"
echo "   - Decentralized: $DECENTRALIZED_LOG"
echo ""

if [ -n "$CENTRAL_CSV" ] && [ -f "$CENTRAL_CSV" ]; then
    echo "📊 CSV data:"
    echo "   - Centralized:  $CENTRAL_CSV"
fi

if [ -n "$DECENTRAL_CSV" ] && [ -f "$DECENTRAL_CSV" ]; then
    echo "   - Decentralized: $DECENTRAL_CSV"
fi

echo ""
echo "✅ Comparison complete!"
echo ""
