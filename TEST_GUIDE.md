# Multi-Agent Task Test

This test runs 1 manager and N agents, executing 3*N tasks to verify the P2P distributed TSWAP system.

## Test Structure

### Components
- **1 Manager**: Distributes tasks to agents
- **N Agents**: Execute tasks using TSWAP algorithm
- **3*N Tasks**: Each agent gets approximately 3 tasks

### What's Being Tested
1. **Peer Discovery**: Agents discover each other via mDNS
2. **Task Distribution**: Manager assigns tasks to available agents
3. **TSWAP Algorithm**: 
   - Goal swapping when agents block each other
   - Deadlock detection and rotation
   - Collision avoidance
4. **P2P Communication**: 
   - Position broadcasting
   - Goal swap requests/responses
   - Target rotation coordination

## Running the Test

### Method 1: Using the Shell Script (Recommended)

```bash
# Run with default config (3 agents, 9 tasks, 60s wait)
./run_test.sh

# Run with custom config (5 agents, 15 tasks, 90s wait)
./run_test.sh 5 90

# Run with 10 agents, 30 tasks, 120s wait
./run_test.sh 10 120
```

The script will:
1. Build the project
2. Start the manager
3. Start N agents
4. Display generated tasks to copy/paste
5. Wait for completion
6. Cleanup processes

### Method 2: Using Cargo (Manual)

```bash
# Build the test binary
cargo build --bin task_test

# Run with default config (3 agents)
cargo run --bin task_test

# Run with custom config (5 agents, 90s wait)
cargo run --bin task_test 5 90
```

### Method 3: Using Cargo Test

```bash
# Run specific test (3 agents, 9 tasks)
cargo test --test task test_3_agents_9_tasks -- --ignored --nocapture

# Run 5 agents, 15 tasks
cargo test --test task test_5_agents_15_tasks -- --ignored --nocapture

# Run 10 agents, 30 tasks
cargo test --test task test_10_agents_30_tasks -- --ignored --nocapture
```

## Monitoring the Test

### Real-time Logs

```bash
# Watch manager logs
tail -f logs/manager.log

# Watch specific agent
tail -f logs/agent_1.log

# Watch all agents
tail -f logs/agent_*.log
```

### Key Events to Look For

#### Goal Swaps
```bash
grep -r "GOAL_SWAP" logs/
```
Expected output:
```
[GOAL_SWAP] Received goal swap request from <peer_id>
[GOAL_SWAP] Their goal: (x, y), My goal: (x, y)
[GOAL_SWAP] Sent response, swapping goals
[GOAL_SWAP] Goal swap accepted by <peer_id>
```

#### Task Completion
```bash
grep -r "Task completed" logs/
```

#### TSWAP Movements
```bash
grep -r "TSWAP.*Moving" logs/
```

#### Deadlock Detection
```bash
grep -r "Deadlock detected" logs/
```

## Test Results Analysis

### Success Criteria

✅ **All tasks completed**: Check for "Task completed" messages
✅ **No crashes**: All processes should run without panicking
✅ **Goal swaps occurred**: Look for GOAL_SWAP messages
✅ **Agents cooperated**: Multiple agents should have interacted
✅ **No deadlocks**: Or deadlocks were resolved via rotation

### Performance Metrics

Count events:
```bash
# Count goal swaps
grep -r "GOAL_SWAP" logs/ | wc -l

# Count completed tasks
grep -r "Task completed" logs/ | wc -l

# Count movements
grep -r "TSWAP.*Moving" logs/ | wc -l

# Count deadlock detections
grep -r "Deadlock detected" logs/ | wc -l
```

## Example Output

```
╔═══════════════════════════════════════════╗
║  P2P Distributed TSWAP - Multi-Agent Test ║
╚═══════════════════════════════════════════╝

📊 Configuration:
   • Agents: 3
   • Tasks: 9 (3 × 3)
   • Map size: 32×32
   • Wait time: 60s

🔨 Building project...
✅ Build complete

🚀 Starting Manager...
✅ Manager started (PID: 12345)

🚀 Starting 3 agents...
✅ Agent 1 started (PID: 12346)
✅ Agent 2 started (PID: 12347)
✅ Agent 3 started (PID: 12348)

⏳ Waiting for agents to connect...

═══════════════════════════════════════════
📋 GENERATED TASKS (9 tasks):
═══════════════════════════════════════════

task 1 1 10 10
task 4 3 14 13
task 7 5 18 16
task 10 7 22 19
task 13 9 26 22
task 16 11 30 25
task 19 13 2 28
task 22 15 6 31
task 25 17 10 2

═══════════════════════════════════════════
```

## Troubleshooting

### Agents not connecting
- Check if mDNS is working: `dns-sd -B _p2p._udp`
- Increase wait time after agent start
- Check firewall settings

### Tasks not completing
- Increase wait time
- Check logs for errors
- Verify map size is sufficient (32x32 recommended)

### Goal swaps not happening
- Verify agents are in close proximity
- Check radius setting (should be 15)
- Look for agents at their goals blocking others

### Processes not cleaning up
```bash
# Manually kill processes
pkill -f "target/release/manager"
pkill -f "target/release/agent"
```

## Configuration

### Adjusting Parameters

Edit `src/test/run/task.rs`:

```rust
pub struct TestConfig {
    pub num_agents: usize,    // Number of agents
    pub map_size: usize,      // Map grid size
    pub wait_time_secs: u64,  // Test duration
}
```

### Custom Task Generation

Modify the task generation logic in `task.rs` or `run_test.sh`:

```rust
let pickup_x = (i * 3 + 1) % self.config.map_size;
let pickup_y = (i * 2 + 1) % self.config.map_size;
let delivery_x = (i * 4 + 10) % self.config.map_size;
let delivery_y = (i * 3 + 10) % self.config.map_size;
```

## Expected Behavior

1. **Manager starts** and waits for agents
2. **Agents connect** via mDNS discovery
3. **Agents subscribe** to Gossipsub topics
4. **Tasks are sent** to the manager
5. **Manager assigns tasks** to available agents
6. **Agents execute tasks** using TSWAP:
   - Navigate to pickup location
   - Navigate to delivery location
   - Swap goals when necessary
   - Avoid collisions
   - Resolve deadlocks
7. **Agents report completion**
8. **Test completes** after wait time

## Next Steps

After running the test:

1. Review logs for any errors
2. Analyze goal swap frequency
3. Check task completion rate
4. Tune parameters if needed:
   - Increase radius for more cooperation
   - Adjust map size for different densities
   - Modify task distribution
