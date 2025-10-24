#!/bin/bash

# Script to automatically start 10 agents
# Usage: ./start_agents.sh

echo "🤖 Starting 10 agents automatically..."
echo "⚠️  Make sure the manager is already running!"
echo ""

# Function to start an agent in a new terminal tab/window
start_agent() {
    local agent_num=$1
    echo "🚀 Starting Agent $agent_num..."
    
    # For macOS Terminal
    if [[ "$OSTYPE" == "darwin"* ]]; then
        osascript -e "
        tell application \"Terminal\"
            do script \"cd '$PWD' && echo 'Agent $agent_num starting...' && cargo run --bin agent\"
        end tell
        "
    # For Linux with gnome-terminal
    elif command -v gnome-terminal &> /dev/null; then
        gnome-terminal --tab --title="Agent $agent_num" -- bash -c "cd '$PWD' && echo 'Agent $agent_num starting...' && cargo run --bin agent; exec bash"
    # For other terminals, just run in background
    else
        echo "Starting Agent $agent_num in background..."
        cargo run --bin agent &
    fi
    
    # Small delay between starts to avoid conflicts
    sleep 2
}

# Start 10 agents
for i in {1..100}; do
    start_agent $i
done

echo ""
echo "✅ All 10 agents have been started!"
echo "📋 Wait for all agents to show: '🚀 Starting to process tasks!'"
echo "🎯 Then go to the manager terminal and type 'task' to assign tasks"