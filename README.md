# p2p_distributed_tswap


cargo clean

## Test

### TSWAP with Manager and Agents

**Important: Start in this order!**

```bash
# 1. Start Manager first (Terminal 1)
cargo run --bin manager

# Wait for: "✅ Manager ready! Listening for agents..."

# 2. Start Agent(s) (Terminal 2, 3, 4...)
# Option A: Start agents manually
cargo run --bin agent

# Option B: Start 10 agents automatically (macOS/Linux)
./start_agents.sh

# Wait for each agent to show: "🚀 Starting to process tasks!"

# 3. In Manager terminal, type "task" to assign tasks
task
```

Kill Task
```
pkill -f "target/debug/agent"
```

**Note:** 
- Wait 3-5 seconds after all agents are ready before typing `task`
- Look for "🔗 Peer XXX subscribed to topic: mapd" messages
- Manager will automatically detect subscribed agents

```


### chat 

```
cargo run --bin chat     
```


### stream

get listen_address

```
cargo run --bin stream

```

Other terminal

```
cargo run --bin stream -- "{listen_address}"

```
