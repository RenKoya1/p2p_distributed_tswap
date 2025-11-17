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

    vec![start]
}

// 中央集権的な経路計画（衝突回避付き）
fn plan_all_paths(
    agents: &mut [AgentState],
    pos2id: &HashMap<Point, usize>,
    nodes: &[Node],
) -> Vec<MoveInstruction> {
    let mut instructions = vec![];
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // この時間ステップで予約された位置を追跡
    let mut reserved_positions: HashMap<Point, String> = HashMap::new();

    // まず、ゴールにいるエージェントはその場に留まる
    for agent in agents.iter() {
        if let Some(goal) = agent.goal_pos {
            if agent.current_pos == goal {
                reserved_positions.insert(agent.current_pos, agent.peer_id.clone());
            }
        }
    }

    // 次に、ゴールにいないエージェントの移動を計画
    for agent in agents.iter_mut() {
        if let Some(goal) = agent.goal_pos {
            if agent.current_pos == goal {
                // ゴールで待機
                instructions.push(MoveInstruction {
                    peer_id: agent.peer_id.clone(),
                    next_pos: agent.current_pos,
                    timestamp,
                });
                continue;
            }

            // まだ経路が計算されていなければ計算
            if agent.path.is_empty() {
                if let (Some(&start_id), Some(&goal_id)) =
                    (pos2id.get(&agent.current_pos), pos2id.get(&goal))
                {
                    let path_ids = get_path(start_id, goal_id, nodes);
                    agent.path = path_ids.iter().map(|&id| nodes[id].pos).collect();
                }
            }

            // 経路から次の位置を取得
            if agent.path.len() > 1 {
                let next_pos = agent.path[1];

                // 次の位置が利用可能かチェック
                if !reserved_positions.contains_key(&next_pos) {
                    reserved_positions.insert(next_pos, agent.peer_id.clone());
                    instructions.push(MoveInstruction {
                        peer_id: agent.peer_id.clone(),
                        next_pos,
                        timestamp,
                    });

                    // エージェントの現在位置と経路を更新
                    agent.current_pos = next_pos;
                    agent.path.remove(0);
                } else {
                    // 衝突回避のため待機
                    instructions.push(MoveInstruction {
                        peer_id: agent.peer_id.clone(),
                        next_pos: agent.current_pos,
                        timestamp,
                    });
                }
            } else {
                // 経路なし、待機
                instructions.push(MoveInstruction {
                    peer_id: agent.peer_id.clone(),
                    next_pos: agent.current_pos,
                    timestamp,
                });
            }
        } else {
            // ゴールなし、待機
            instructions.push(MoveInstruction {
                peer_id: agent.peer_id.clone(),
                next_pos: agent.current_pos,
                timestamp,
            });
        }
    }

    instructions
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
                .heartbeat_interval(Duration::from_millis(500))
                .heartbeat_initial_delay(Duration::from_millis(100))
                .mesh_n_low(1)
                .mesh_n(2)
                .mesh_n_high(3)
                .validation_mode(gossipsub::ValidationMode::Strict)
                .message_id_fn(message_id_fn)
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
    let planning_interval = Duration::from_millis(200);

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
            _ = tokio::time::sleep(Duration::from_millis(100)), if last_planning.elapsed() > planning_interval => {
                if !agent_states.is_empty() {
                    let mut agents: Vec<AgentState> = agent_states.values().cloned().collect();
                    let num_agents = agents.len();
                    let plan_start = std::time::Instant::now();
                    let instructions = plan_all_paths(&mut agents, &pos2id, &nodes);
                    let elapsed = plan_start.elapsed();

                    // 公平な比較のため、各エージェントごとの平均時間を記録
                    // 分散型では各エージェントが個別に計算するため、1エージェントあたりの時間で比較
                    let per_agent_duration = elapsed / num_agents as u32;
                    path_metrics.record_duration(per_agent_duration);

                    println!(
                        "⏱️ Central path planning for {} agents took {:.3} ms (avg {:.3} ms/agent)",
                        num_agents,
                        elapsed.as_secs_f64() * 1000.0,
                        per_agent_duration.as_secs_f64() * 1000.0
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

            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("🎧 Listening on {address}");
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Discovered(list))) if !ignore_mdns => {
                    for (peer_id, _multiaddr) in list {
                        if !known_peers.contains(&peer_id) {
                            println!("🔍 mDNS discovered: {}", peer_id);
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

                                if let Some(peer_id_str) = task_peer_map.get(&task_id) {
                                    if let Some(agent) = agent_states.get_mut(peer_id_str) {
                                        agent.task = None;
                                        agent.goal_pos = None;
                                        agent.path.clear();
                                        agent.task_phase = TaskPhase::Idle;
                                    }
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
                                        "🚀 Assigned {} pending tasks after completion",
                                        newly_assigned
                                    );
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
