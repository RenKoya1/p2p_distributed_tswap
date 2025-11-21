use futures::stream::StreamExt;
use libp2p::{
    gossipsub, mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use p2p_distributed_tswap::map::map::MAP;
use p2p_distributed_tswap::map::task_generator::{Task, TaskGeneratorAgent};
use p2p_distributed_tswap::map::task_metrics::{
    PathComputationMetrics, TaskMetric, TaskMetricsCollector,
};

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::{BinaryHeap, HashSet, hash_map::DefaultHasher};
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::{io, io::AsyncBufReadExt, select};

type Point = (usize, usize);

fn parse_map() -> Vec<Vec<char>> {
    let grid = MAP
        .replace('\r', "")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().collect::<Vec<char>>())
        .collect::<Vec<_>>();

    grid
}

#[derive(Clone)]
struct Node {
    id: usize,
    pos: Point,
    neighbors: Vec<usize>,
}

// TSWAP用のエージェント構造体
#[derive(Clone, Copy, Debug)]
struct TswapAgent {
    id: usize,
    v: usize, // current Node id
    g: usize, // goal Node id
}

impl PartialEq for TswapAgent {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for TswapAgent {}
impl PartialOrd for TswapAgent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TswapAgent {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

// マネージャーからエージェントへの移動指示
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MoveInstruction {
    peer_id: String,
    next_pos: Point,
    timestamp: u64,
}

// マネージャーが追跡するエージェントの状態
#[derive(Clone, Debug)]
struct AgentState {
    peer_id: String,
    current_pos: Point,
    goal_pos: Option<Point>,
    path: Vec<Point>,
    task: Option<Task>,
    task_phase: TaskPhase, // pickup前、delivery前、完了
}

#[derive(Clone, Debug, PartialEq)]
enum TaskPhase {
    Idle,
    MovingToPickup,
    MovingToDelivery,
}

#[derive(NetworkBehaviour)]
struct MapdBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

// TSWAP中央集権的な経路計画
fn plan_all_paths(
    agents: &mut [AgentState],
    pos2id: &HashMap<Point, usize>,
    nodes: &[Node],
    _came_from_cache: &mut HashMap<usize, usize>,
    _g_score_cache: &mut HashMap<usize, usize>,
) -> Vec<MoveInstruction> {
    let mut instructions = vec![];
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // AgentStateをTswapAgentに変換
    let mut tswap_agents: Vec<TswapAgent> = agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let v = pos2id.get(&agent.current_pos).copied().unwrap_or(0);
            let g = agent
                .goal_pos
                .and_then(|goal| pos2id.get(&goal).copied())
                .unwrap_or(v);
            TswapAgent { id: i, v, g }
        })
        .collect();

    // TSWAPステップを実行
    tswap_step(&mut tswap_agents, nodes);

    // 結果をAgentStateとMoveInstructionに反映
    for (i, tswap_agent) in tswap_agents.iter().enumerate() {
        let next_pos = nodes[tswap_agent.v].pos;
        agents[i].current_pos = next_pos;

        instructions.push(MoveInstruction {
            peer_id: agents[i].peer_id.clone(),
            next_pos,
            timestamp,
        });
    }

    instructions
}

// TSWAPアルゴリズムの1ステップ
fn tswap_step(agents: &mut [TswapAgent], nodes: &[Node]) {
    let n = agents.len();

    // TSWAP Goal Swapping Phase
    // Rule 3: If agent j is at its goal, swap goals
    // Rule 4: Deadlock detection and target rotation
    for i in 0..n {
        // Rule 1: Stay at goal
        if agents[i].v == agents[i].g {
            continue;
        }

        let path = get_path(agents[i].v, agents[i].g, nodes);
        if path.len() < 2 {
            continue;
        }
        let u = path[1]; // Desired next node

        if let Some(j) = agents.iter().position(|b| b.v == u) {
            if i == j {
                continue;
            }

            // Rule 3: Goal swap when agent j is at its goal
            if agents[j].v == agents[j].g {
                let g_i = agents[i].g;
                let g_j = agents[j].g;
                agents[i].g = g_j;
                agents[j].g = g_i;
            } else {
                // Rule 4: Deadlock detection and resolution
                let mut a_p = vec![i];
                let mut current_b_idx = j;
                let mut deadlock_found = false;

                loop {
                    let b_v = agents[current_b_idx].v;
                    let b_g = agents[current_b_idx].g;

                    if b_v == b_g {
                        break;
                    }

                    let b_path = get_path(b_v, b_g, nodes);
                    if b_path.len() < 2 {
                        break;
                    }
                    let w = b_path[1];

                    if let Some(c_idx) = agents.iter().position(|c| c.v == w) {
                        if a_p.contains(&current_b_idx) {
                            a_p.clear();
                            break;
                        }
                        a_p.push(current_b_idx);
                        current_b_idx = c_idx;

                        if current_b_idx == i {
                            deadlock_found = true;
                            break;
                        }
                    } else {
                        break;
                    }
                }

                // Rule 4: Rotate targets when deadlock is detected
                if deadlock_found && a_p.len() > 1 {
                    let first_agent_idx = a_p[0];
                    let last_goal = agents[a_p[a_p.len() - 1]].g;
                    for k in (1..a_p.len()).rev() {
                        let prev_g = agents[a_p[k - 1]].g;
                        agents[a_p[k]].g = prev_g;
                    }
                    agents[first_agent_idx].g = last_goal;
                }
            }
        }
    }

    // TSWAP Movement Phase
    // Rule 2: Move to next node if possible
    // Rule 5: Stay if next node is occupied
    for i in 0..n {
        if agents[i].v == agents[i].g {
            continue;
        }

        let path = get_path(agents[i].v, agents[i].g, nodes);
        if path.len() < 2 {
            continue;
        }
        let u = path[1];

        // Check if next position is available or can be swapped
        if let Some(j) = agents.iter().position(|b| b.v == u) {
            if i != j {
                // Check for mutual swap (both agents want each other's positions)
                let path_j = get_path(agents[j].v, agents[j].g, nodes);
                if path_j.len() >= 2 && path_j[1] == agents[i].v {
                    // Mutual swap: exchange positions simultaneously
                    let temp_v = agents[i].v;
                    agents[i].v = agents[j].v;
                    agents[j].v = temp_v;
                }
                // Rule 5: Otherwise stay (next node is occupied)
            }
        } else {
            // Rule 2: Next node is free, move to it
            agents[i].v = u;
        }
    }
}

// TSWAPで使用する経路探索（A*）
fn get_path(start: usize, goal: usize, nodes: &[Node]) -> Vec<usize> {
    if start == goal {
        return vec![start];
    }

    #[derive(Clone)]
    struct AstarNode {
        node_id: usize,
        g_cost: usize,
        f_cost: usize,
    }

    impl PartialEq for AstarNode {
        fn eq(&self, other: &Self) -> bool {
            self.f_cost == other.f_cost
        }
    }

    impl Eq for AstarNode {}

    impl PartialOrd for AstarNode {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for AstarNode {
        fn cmp(&self, other: &Self) -> Ordering {
            other
                .f_cost
                .cmp(&self.f_cost)
                .then_with(|| other.g_cost.cmp(&self.g_cost))
        }
    }

    let mut open_list = BinaryHeap::new();
    let mut came_from = HashMap::new();
    let mut g_score = HashMap::new();

    let heuristic = |node_id: usize| -> usize {
        let (x1, y1) = nodes[node_id].pos;
        let (x2, y2) = nodes[goal].pos;
        ((x1 as isize - x2 as isize).abs() + (y1 as isize - y2 as isize).abs()) as usize
    };

    g_score.insert(start, 0);
    let start_node = AstarNode {
        node_id: start,
        g_cost: 0,
        f_cost: heuristic(start),
    };
    open_list.push(start_node);

    while let Some(current) = open_list.pop() {
        let current_id = current.node_id;

        if current_id == goal {
            let mut path = vec![];
            let mut current_node = current_id;
            path.push(current_node);

            while let Some(&parent) = came_from.get(&current_node) {
                path.push(parent);
                current_node = parent;
            }
            path.reverse();
            return path;
        }

        for &neighbor_id in &nodes[current_id].neighbors {
            let tentative_g = current.g_cost + 1;

            if tentative_g < *g_score.get(&neighbor_id).unwrap_or(&usize::MAX) {
                came_from.insert(neighbor_id, current_id);
                g_score.insert(neighbor_id, tentative_g);

                let h_cost = heuristic(neighbor_id);
                let f_cost = tentative_g + h_cost;

                let neighbor_node = AstarNode {
                    node_id: neighbor_id,
                    g_cost: tentative_g,
                    f_cost,
                };

                open_list.push(neighbor_node);
            }
        }
    }

    // No path found, return best neighbor
    let mut best_neighbor = start;
    let mut min_dist = heuristic(start);

    for &neighbor_id in &nodes[start].neighbors {
        let dist = heuristic(neighbor_id);
        if dist < min_dist {
            min_dist = dist;
            best_neighbor = neighbor_id;
        }
    }

    vec![start, best_neighbor]
}

fn try_assign_pending_tasks<'a>(
    pending: &mut usize,
    agent_states: &mut HashMap<String, AgentState>,
    task_gen: &mut TaskGeneratorAgent<'a>,
    metrics_collector: &mut TaskMetricsCollector,
    task_peer_map: &mut HashMap<u64, String>,
    swarm: &mut libp2p::Swarm<MapdBehaviour>,
    topic: &gossipsub::IdentTopic,
    task_counter: &mut u64,
) -> usize {
    let mut assigned = 0;

    while *pending > 0 {
        let Some(peer_id_str) = agent_states
            .iter()
            .find(|(_, state)| state.task.is_none())
            .map(|(peer_id, _)| peer_id.clone())
        else {
            break;
        };

        let Some(mut task) = task_gen.generate_task() else {
            println!("⚠️  Task generation failed (not enough free cells)");
            break;
        };

        *task_counter += 1;
        let task_id = *task_counter;
        task.peer_id = Some(peer_id_str.clone());
        task.task_id = Some(task_id);

        let metric = TaskMetric::new(task_id, peer_id_str.clone());
        metrics_collector.add_metric(metric);

        match serde_json::to_vec(&task) {
            Ok(task_bytes) => match swarm
                .behaviour_mut()
                .gossipsub
                .publish(topic.clone(), task_bytes)
            {
                Ok(_) => {
                    if let Some(agent) = agent_states.get_mut(&peer_id_str) {
                        agent.task = Some(task.clone());
                        agent.goal_pos = Some(task.pickup);
                        agent.path.clear();
                        agent.task_phase = TaskPhase::MovingToPickup;
                    }
                    task_peer_map.insert(task_id, peer_id_str.clone());
                    *pending -= 1;
                    assigned += 1;
                    println!(
                        "✅ Task {} assigned to {}",
                        task_id,
                        &peer_id_str[..std::cmp::min(8, peer_id_str.len())]
                    );
                }
                Err(e) => {
                    println!("⚠️  Publish error: {e:?}");
                    break;
                }
            },
            Err(e) => {
                println!("⚠️  Task serialization error: {e:?}");
                break;
            }
        }
    }

    assigned
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🏢 ============================================");
    println!("🏢 [CENTRALIZED MANAGER] Starting...");
    println!("🏢 All pathfinding will be done centrally!");
    println!("🏢 ============================================");

    let args: Vec<String> = std::env::args().collect();
    let ignore_mdns = args.contains(&"--clean".to_string());

    if ignore_mdns {
        println!("🧹 Running in CLEAN mode - ignoring mDNS discoveries");
    }

    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|key| {
            let message_id_fn = |message: &gossipsub::Message| {
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                gossipsub::MessageId::from(s.finish().to_string())
            };

            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(2)) // 500ms→2秒: CPU使用量削減
                .heartbeat_initial_delay(Duration::from_millis(500)) // 初期遅延も延長
                .mesh_n_low(1) // 最小メッシュサイズ
                .mesh_n(1) // 理想メッシュサイズ: 1対多なので1で十分
                .mesh_n_high(2) // 最大メッシュサイズ削減
                .validation_mode(gossipsub::ValidationMode::Permissive)
                .message_id_fn(message_id_fn)
                .history_length(3) // メッセージ履歴を3に削減（メモリ節約）
                .history_gossip(1) // Gossip履歴を1に削減
                .max_transmit_size(262_144) // 256KB: タスク送信には十分
                .max_ihave_length(100) // IHAVE メッセージ制限
                .max_ihave_messages(10) // IHAVE メッセージ数制限
                .build()
                .map_err(io::Error::other)?;

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(key.clone()),
                gossipsub_config,
            )?;

            let mdns =
                mdns::tokio::Behaviour::new(mdns::Config::default(), key.public().to_peer_id())?;
            Ok(MapdBehaviour { gossipsub, mdns })
        })?
        .build();

    let topic = gossipsub::IdentTopic::new("mapd");
    swarm.behaviour_mut().gossipsub.subscribe(&topic)?;
    println!("🆔 Manager Peer ID: {}", swarm.local_peer_id());

    let grid = Arc::new(parse_map());
    let mut task_gen = TaskGeneratorAgent::new(&grid);
    let mut stdin = io::BufReader::new(io::stdin()).lines();

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // 経路探索用のノードグラフを構築
    let mut pos2id = HashMap::new();
    let mut id2pos = vec![];
    let mut node_id_counter = 0;
    let h = grid.len();
    let w = grid[0].len();
    for y in 0..h {
        for x in 0..w {
            if grid[y][x] != '@' {
                pos2id.insert((x, y), node_id_counter);
                id2pos.push((x, y));
                node_id_counter += 1;
            }
        }
    }
    let mut nodes = vec![];
    for (id, &(x, y)) in id2pos.iter().enumerate() {
        let mut neighbors = vec![];
        for (dx, dy) in [(0, 1), (1, 0), (0, -1), (-1, 0)] {
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if nx >= 0 && ny >= 0 {
                let npos = (nx as usize, ny as usize);
                if pos2id.contains_key(&npos) {
                    neighbors.push(pos2id[&npos]);
                }
            }
        }
        nodes.push(Node {
            id,
            pos: (x, y),
            neighbors,
        });
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("📋 Commands:");
    println!("  - 'task': Generate and assign task to an available agent");
    println!("  - 'tasks <N>': Generate and assign N tasks");
    println!("  - 'metrics': Show task and path statistics");
    println!("  - 'save <filename>': Save task metrics to CSV");
    println!("  - 'save path <filename>': Save path computation metrics to CSV");
    println!("  - 'reset': Clear all state");
    println!("⏳ Waiting for Gossipsub mesh setup...");

    tokio::time::sleep(Duration::from_secs(1)).await;
    println!("✅ [CENTRALIZED MANAGER] Ready!");

    let mut known_peers: HashSet<libp2p::PeerId> = HashSet::new();
    let mut subscribed_peers: HashSet<libp2p::PeerId> = HashSet::new();
    let mut task_counter: u64 = 0;
    let mut metrics_collector = TaskMetricsCollector::new();
    let mut path_metrics = PathComputationMetrics::new();

    // マネージャーが追跡するエージェント状態
    let mut agent_states: HashMap<String, AgentState> = HashMap::new();
    let mut task_peer_map: HashMap<u64, String> = HashMap::new();
    let mut pending_task_requests: usize = 0;

    // 定期的な経路計画
    let mut last_planning = std::time::Instant::now();
    // 平均計画時間が180msなので、余裕を持たせて300ms間隔に設定
    // これにより、計画完了後に約120msの余裕ができる
    let planning_interval = Duration::from_millis(500); // 300ms = 1秒に約3.3ステップ

    // A*アルゴリズム用の再利用可能なHashMap（メモリ削減）
    let mut astar_came_from: HashMap<usize, usize> = HashMap::with_capacity(1000);
    let mut astar_g_score: HashMap<usize, usize> = HashMap::with_capacity(1000);

    // 定期的なクリーンアップ用タイマー
    let mut last_cleanup = std::time::Instant::now();
    let cleanup_interval = Duration::from_secs(30);

    loop {
        select! {
            Ok(Some(line)) = stdin.next_line() => {
                let trimmed = line.trim();

                if trimmed == "metrics" {
                    let stats = metrics_collector.get_statistics();
                    println!("{}", stats);
                    if let Some(path_stats) = path_metrics.get_statistics() {
                        println!("{}", path_stats);
                    } else {
                        println!("⏱️ Path Computation: no samples yet");
                    }
                    continue;
                }

                if trimmed == "reset" {
                    known_peers.clear();
                    subscribed_peers.clear();
                    agent_states.clear();
                    task_peer_map.clear();
                    metrics_collector = TaskMetricsCollector::new();
                    task_counter = 0;
                    path_metrics.clear();
                    println!("✅ All state cleared!");
                    continue;
                }

                if trimmed.starts_with("save path ") {
                    let filename = trimmed["save path ".len()..].trim();
                    if filename.is_empty() {
                        println!("⚠️  Usage: save path <filename>");
                    } else {
                        match std::fs::write(filename, path_metrics.to_csv_string()) {
                            Ok(_) => println!("💾 Saved path metrics to {}", filename),
                            Err(e) => println!("⚠️  Failed to save path metrics: {e:?}"),
                        }
                    }
                    continue;
                }

                if trimmed.starts_with("save ") {
                    let filename = &trimmed[5..];
                    let csv_content = metrics_collector.to_csv_string();
                    match std::fs::write(filename, csv_content) {
                        Ok(_) => println!("✅ Metrics saved to {}", filename),
                        Err(e) => println!("⚠️  Failed to save: {e:?}"),
                    }
                    continue;
                }

                if trimmed.starts_with("tasks ") {
                    let num_str = &trimmed[6..];
                    if let Ok(num_tasks) = num_str.parse::<usize>() {
                        pending_task_requests += num_tasks;
                        let sent_count = try_assign_pending_tasks(
                            &mut pending_task_requests,
                            &mut agent_states,
                            &mut task_gen,
                            &mut metrics_collector,
                            &mut task_peer_map,
                            &mut swarm,
                            &topic,
                            &mut task_counter,
                        );
                        println!(
                            "🏢 [CENTRALIZED] Requested {}, assigned {} immediately (pending: {})",
                            num_tasks, sent_count, pending_task_requests
                        );
                        continue;
                    }
                }

                if trimmed == "task" {
                    pending_task_requests += 1;
                    let sent_count = try_assign_pending_tasks(
                        &mut pending_task_requests,
                        &mut agent_states,
                        &mut task_gen,
                        &mut metrics_collector,
                        &mut task_peer_map,
                        &mut swarm,
                        &topic,
                        &mut task_counter,
                    );
                    if sent_count == 0 {
                        println!("⚠️  No available agents right now (pending: {})", pending_task_requests);
                    }
                    continue;
                }

                // ユーザーメッセージを公開
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), trimmed.as_bytes()) {
                    println!("⚠️  Publish error: {e:?}");
                }
            }

            // 定期的な中央集権的経路計画
            _ = tokio::time::sleep(Duration::from_millis(50)), if last_planning.elapsed() >= planning_interval => {
                if !agent_states.is_empty() {
                    let mut agents: Vec<AgentState> = agent_states.values().cloned().collect();
                    let num_agents = agents.len();
                    let plan_start = std::time::Instant::now();
                    let instructions = plan_all_paths(&mut agents, &pos2id, &nodes, &mut astar_came_from, &mut astar_g_score);
                    let elapsed = plan_start.elapsed();

                    // 1ステップの計算時間 = マネージャーが全エージェントの経路を計算する総時間
                    // 集中型の特性：全エージェントを一度に処理する時間を測定
                    path_metrics.record_duration(elapsed);

                    println!(
                        "⏱️ Central path planning for {} agents took {:.3} ms (interval: {:.3}ms)",
                        num_agents,
                        elapsed.as_secs_f64() * 1000.0,
                        last_planning.elapsed().as_secs_f64() * 1000.0
                    );

                    // エージェント状態を更新
                    for agent in agents {
                        // pickup/deliveryに到達したかチェック
                        if let Some(task) = &agent.task {
                            if agent.task_phase == TaskPhase::MovingToPickup && agent.current_pos == task.pickup {
                                // pickupに到達、次はdeliveryへ
                                if let Some(state) = agent_states.get_mut(&agent.peer_id) {
                                    state.goal_pos = Some(task.delivery);
                                    state.path.clear();
                                    state.task_phase = TaskPhase::MovingToDelivery;
                                    println!("📦 Agent {} reached PICKUP, now moving to DELIVERY", &agent.peer_id[..std::cmp::min(8, agent.peer_id.len())]);
                                }
                            }
                        }
                        agent_states.insert(agent.peer_id.clone(), agent);
                    }

                    // エージェントに移動指示を送信
                    for instruction in instructions {
                        let msg = serde_json::json!({
                            "type": "move_instruction",
                            "peer_id": instruction.peer_id,
                            "next_pos": [instruction.next_pos.0, instruction.next_pos.1],
                            "timestamp": instruction.timestamp
                        }).to_string();

                        let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), msg.as_bytes());
                    }
                }
                last_planning = std::time::Instant::now();
            }

            // 定期的なメモリクリーンアップ（30秒ごと）
            _ = tokio::time::sleep(Duration::from_secs(1)), if last_cleanup.elapsed() > cleanup_interval => {
                // 完了したタスクを持つエージェントをクリーンアップ
                agent_states.retain(|_, state| {
                    state.task_phase != TaskPhase::Idle || state.task.is_some()
                });

                // エージェント数制限（最大500エージェント）
                if agent_states.len() > 500 {
                    let to_remove: Vec<String> = agent_states.keys()
                        .filter(|id| agent_states[*id].task_phase == TaskPhase::Idle)
                        .take(agent_states.len() - 500)
                        .cloned()
                        .collect();
                    for key in to_remove {
                        agent_states.remove(&key);
                    }
                }

                // 古いタスクマッピングをクリーンアップ
                let active_task_ids: std::collections::HashSet<u64> = agent_states.values()
                    .filter_map(|state| state.task.as_ref().and_then(|t| t.task_id))
                    .collect();
                task_peer_map.retain(|task_id, _| active_task_ids.contains(task_id));

                // known_peers/subscribed_peersも制限
                if known_peers.len() > 1000 {
                    let to_remove: Vec<libp2p::PeerId> = known_peers.iter()
                        .take(known_peers.len() - 1000)
                        .cloned()
                        .collect();
                    for peer in to_remove {
                        known_peers.remove(&peer);
                        subscribed_peers.remove(&peer);
                    }
                }

                println!("🧹 [CLEANUP] Active agents: {}, Active tasks: {}, Known peers: {}",
                         agent_states.len(), task_peer_map.len(), known_peers.len());
                last_cleanup = std::time::Instant::now();
            }

            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("🎧 Listening on {address}");
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Discovered(list))) if !ignore_mdns => {
                    for (peer_id, _multiaddr) in list {
                        if !known_peers.contains(&peer_id) {
                            println!("🔍 [MANAGER] mDNS discovered agent: {}", &peer_id.to_base58()[..8]);
                            // Managerのみがエージェントを明示的にGossipsubメッシュに追加
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                            known_peers.insert(peer_id);
                        }
                    }
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("⏰ mDNS expired: {}", peer_id);
                        if !ignore_mdns {
                            swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                        }
                    }
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic })) => {
                    println!("🔗 Peer {} subscribed to topic: {}", peer_id, topic);
                    subscribed_peers.insert(peer_id.clone());

                    // Subscribedイベント後、そのピアからのメッセージを受信できるように少し待つ
                    let peer_short = peer_id.to_base58();
                    println!("👂 Now listening for messages from {}", &peer_short[..std::cmp::min(8, peer_short.len())]);
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                    let source_str = message.source.as_ref().map(|s| {
                        let full = s.to_base58();
                        full[..std::cmp::min(8, full.len())].to_string()
                    });
                    println!("📨 [DEBUG] Received message from: {:?}, size: {} bytes", source_str, message.data.len());

                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&message.data) {
                        // エージェントからの位置更新
                        if val.get("type") == Some(&serde_json::Value::String("position_update".to_string())) {
                            println!("📍 [DEBUG] Received position_update message: {:?}", val);
                            if let (Some(peer_id), Some(pos_arr)) = (val.get("peer_id"), val.get("position")) {
                                if let (Some(peer_id_str), Some(pos)) = (peer_id.as_str(), pos_arr.as_array()) {
                                    if pos.len() == 2 {
                                        if let (Some(x), Some(y)) = (pos[0].as_u64(), pos[1].as_u64()) {
                                            let position = (x as usize, y as usize);
                                            println!("✅ [MANAGER] Agent {} position: {:?}", peer_id_str, position);

                                            let is_new = !agent_states.contains_key(peer_id_str);
                                            agent_states.entry(peer_id_str.to_string())
                                                .and_modify(|state| {
                                                    state.current_pos = position;
                                                })
                                                .or_insert(AgentState {
                                                    peer_id: peer_id_str.to_string(),
                                                    current_pos: position,
                                                    goal_pos: None,
                                                    path: vec![],
                                                    task: None,
                                                    task_phase: TaskPhase::Idle,
                                                });

                                            if is_new {
                                                println!("🆕 [MANAGER] New agent registered: {} at {:?}", peer_id_str, position);
                                                println!("👥 [MANAGER] Total available agents: {}", agent_states.len());
                                            }

                                            let newly_assigned = try_assign_pending_tasks(
                                                &mut pending_task_requests,
                                                &mut agent_states,
                                                &mut task_gen,
                                                &mut metrics_collector,
                                                &mut task_peer_map,
                                                &mut swarm,
                                                &topic,
                                                &mut task_counter,
                                            );

                                            if newly_assigned > 0 {
                                                println!(
                                                    "🚀 Assigned {} pending tasks after position update",
                                                    newly_assigned
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // タスクメトリクス
                        if let Some(metric_type) = val.get("type").and_then(|v| v.as_str()) {
                            if let Some(task_id) = val.get("task_id").and_then(|v| v.as_u64()) {
                                match metric_type {
                                    "task_metric_received" => metrics_collector.update_received(task_id),
                                    "task_metric_started" => metrics_collector.update_started(task_id),
                                    "task_metric_completed" => metrics_collector.update_completed(task_id),
                                    _ => {}
                                }
                            }
                        }

                        // タスク完了
                        if val.get("status") == Some(&serde_json::Value::String("done".to_string())) {
                            if let Some(task_id) = val.get("task_id").and_then(|v| v.as_u64()) {
                                println!("✅ Task {} completed!", task_id);

                                let completed_peer_id = if let Some(peer_id_str) = task_peer_map.get(&task_id) {
                                    let peer_id = peer_id_str.clone();
                                    if let Some(agent) = agent_states.get_mut(peer_id_str) {
                                        agent.task = None;
                                        agent.goal_pos = None;
                                        agent.path.clear();
                                        agent.task_phase = TaskPhase::Idle;
                                        println!("🔄 Agent {} is now available for new tasks", &peer_id[..std::cmp::min(8, peer_id.len())]);
                                    }
                                    Some(peer_id)
                                } else {
                                    None
                                };

                                // 保留中のタスクがあれば優先的に割り当て
                                if pending_task_requests > 0 {
                                    let newly_assigned = try_assign_pending_tasks(
                                        &mut pending_task_requests,
                                        &mut agent_states,
                                        &mut task_gen,
                                        &mut metrics_collector,
                                        &mut task_peer_map,
                                        &mut swarm,
                                        &topic,
                                        &mut task_counter,
                                    );

                                    if newly_assigned > 0 {
                                        println!(
                                            "🚀 Assigned {} pending tasks after completion (remaining: {})",
                                            newly_assigned, pending_task_requests
                                        );
                                    }
                                } else if let Some(peer_id) = completed_peer_id {
                                    // 保留タスクがなくても、完了したエージェントに新しいタスクを生成して割り当て
                                    if let Some(mut new_task) = task_gen.generate_task() {
                                        task_counter += 1;
                                        let new_task_id = task_counter;
                                        new_task.peer_id = Some(peer_id.clone());
                                        new_task.task_id = Some(new_task_id);

                                        let metric = TaskMetric::new(new_task_id, peer_id.clone());
                                        metrics_collector.add_metric(metric);

                                        match serde_json::to_vec(&new_task) {
                                            Ok(task_bytes) => match swarm
                                                .behaviour_mut()
                                                .gossipsub
                                                .publish(topic.clone(), task_bytes)
                                            {
                                                Ok(_) => {
                                                    if let Some(agent) = agent_states.get_mut(&peer_id) {
                                                        agent.task = Some(new_task.clone());
                                                        agent.goal_pos = Some(new_task.pickup);
                                                        agent.path.clear();
                                                        agent.task_phase = TaskPhase::MovingToPickup;
                                                    }
                                                    task_peer_map.insert(new_task_id, peer_id.clone());
                                                    println!(
                                                        "🔁 Auto-assigned new task {} to {} after completion",
                                                        new_task_id,
                                                        &peer_id[..std::cmp::min(8, peer_id.len())]
                                                    );
                                                }
                                                Err(e) => {
                                                    println!("⚠️  Failed to publish auto-assigned task: {e:?}");
                                                }
                                            },
                                            Err(e) => {
                                                println!("⚠️  Failed to serialize auto-assigned task: {e:?}");
                                            }
                                        }
                                    } else {
                                        println!("⚠️  No more tasks available to auto-assign");
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
