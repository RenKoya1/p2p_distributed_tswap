use rand::seq::SliceRandom;
use rand::thread_rng;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    hash::{Hash, Hasher},
    time::Duration,
};

use futures::stream::StreamExt;
use libp2p::{
    gossipsub, mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use p2p_distributed_tswap::map::make_node;
use p2p_distributed_tswap::map::map::MAP;
use p2p_distributed_tswap::map::map::Point;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::Arc;
use tokio::{io, io::AsyncBufReadExt, select};

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

// TSWAPに使用するエージェント情報構造体
#[derive(Clone, Debug, Serialize, Deserialize)]
struct AgentInfo {
    peer_id: String,
    current_pos: Point,
    goal_pos: Point,
    timestamp: u64,
}

// ゴール交換リクエスト
#[derive(Clone, Debug, Serialize, Deserialize)]
struct GoalSwapRequest {
    request_id: String,
    from_peer: String,
    to_peer: String,
    my_goal: Point,
}

// ゴール交換レスポンス
#[derive(Clone, Debug, Serialize, Deserialize)]
struct GoalSwapResponse {
    request_id: String,
    from_peer: String,
    to_peer: String,
    my_goal: Point,
    accepted: bool,
}

// ターゲットローテーションリクエスト
#[derive(Clone, Debug, Serialize, Deserialize)]
struct TargetRotationRequest {
    request_id: String,
    initiator: String,
    participants: Vec<String>, // デッドロックサイクルのエージェントリスト
    goals: Vec<Point>,         // 各エージェントの現在のゴール
}

// 近くのエージェントを管理
struct NearbyAgents {
    agents: HashMap<String, AgentInfo>,
    last_cleanup: std::time::Instant,
}

impl NearbyAgents {
    fn new() -> Self {
        NearbyAgents {
            agents: HashMap::new(),
            last_cleanup: std::time::Instant::now(),
        }
    }

    fn update(&mut self, info: AgentInfo) {
        self.agents.insert(info.peer_id.clone(), info);
    }

    fn get_nearby(&self, my_pos: Point, radius: usize, my_peer_id: &str) -> Vec<AgentInfo> {
        // デバッグ: 全エージェントの距離を出力
        println!(
            "[GET_NEARBY] My pos: {:?}, radius: {}, total agents: {}",
            my_pos,
            radius,
            self.agents.len()
        );

        let nearby: Vec<AgentInfo> = self
            .agents
            .values()
            .filter_map(|agent| {
                let dist = manhattan_distance(my_pos, agent.current_pos);
                let is_self = agent.peer_id == my_peer_id;
                let within_radius = dist <= radius;

                println!(
                    "  [CHECK] Agent {}: pos={:?}, dist={}, is_self={}, within_radius={}",
                    &agent.peer_id[..std::cmp::min(8, agent.peer_id.len())],
                    agent.current_pos,
                    dist,
                    is_self,
                    within_radius
                );

                if !is_self && within_radius {
                    Some(agent.clone())
                } else {
                    None
                }
            })
            .collect();

        println!("[GET_NEARBY] Found {} nearby agents", nearby.len());
        nearby
    }

    fn cleanup_old(&mut self, max_age_secs: u64) {
        if self.last_cleanup.elapsed() < Duration::from_secs(5) {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.agents
            .retain(|_, agent| now - agent.timestamp < max_age_secs);
        self.last_cleanup = std::time::Instant::now();
    }
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

fn manhattan_distance(p1: Point, p2: Point) -> usize {
    ((p1.0 as isize - p2.0 as isize).abs() + (p1.1 as isize - p2.1 as isize).abs()) as usize
}

// TSWAPベースの次の移動先を計算
// TSWAPの判定結果
#[derive(Debug, Clone)]
enum TswapAction {
    Move(Point),                              // 移動先
    WaitForGoalSwap(String),                  // ゴール交換待ち（相手のpeer_id）
    WaitForRotation(Vec<String>, Vec<Point>), // ローテーション待ち（参加者、ゴール）
    Wait,                                     // 単純待機
}

fn compute_next_move_with_tswap(
    my_pos: Point,
    my_goal: Point,
    nearby_agents: &[AgentInfo],
    _grid: &[Vec<char>],
    pos2id: &HashMap<Point, usize>,
    nodes: &[Node],
) -> TswapAction {
    // デバッグ: nearby agents の数を表示
    println!(
        "[TSWAP] My pos: {:?}, My goal: {:?}, Nearby agents: {}",
        my_pos,
        my_goal,
        nearby_agents.len()
    );
    for agent in nearby_agents {
        println!(
            "  - Agent {}: pos={:?}, goal={:?}, distance={}",
            &agent.peer_id[..8],
            agent.current_pos,
            agent.goal_pos,
            manhattan_distance(my_pos, agent.current_pos)
        );
    }

    // 自分がゴールに到達している場合は移動しない
    if my_pos == my_goal {
        return TswapAction::Move(my_pos);
    }

    // 自分の現在位置から次に進むべき位置を計算
    let path = get_path(pos2id[&my_pos], pos2id[&my_goal], nodes);
    if path.len() < 2 {
        return TswapAction::Move(my_pos);
    }

    let next_node_id = path[1];
    let next_pos = nodes[next_node_id].pos;
    println!("[TSWAP] Next position: {:?}", next_pos);

    // 次の位置に他のエージェントがいるかチェック
    if let Some(blocking_agent) = nearby_agents.iter().find(|a| a.current_pos == next_pos) {
        // TSWAPロジック: 相手がゴールにいる場合はゴールを交換
        if blocking_agent.current_pos == blocking_agent.goal_pos {
            println!(
                "[TSWAP] Agent at goal, requesting goal swap with {}",
                blocking_agent.peer_id
            );
            return TswapAction::WaitForGoalSwap(blocking_agent.peer_id.clone());
        }

        // デッドロック検出
        let mut visited = HashSet::new();
        visited.insert(my_pos);
        let mut current_agent = blocking_agent;
        let mut deadlock_chain = vec![my_pos];

        loop {
            if visited.contains(&current_agent.current_pos) {
                // デッドロックサイクル検出
                if current_agent.current_pos == my_pos {
                    println!("[TSWAP] Deadlock detected, need target rotation");
                    // ターゲットローテーションはメッセージングで調整
                }
                break;
            }

            visited.insert(current_agent.current_pos);
            deadlock_chain.push(current_agent.current_pos);

            // 次のエージェントを探す
            if current_agent.current_pos == current_agent.goal_pos {
                break;
            }

            // 次のエージェントが進みたい先を確認
            let blocking_next = nearby_agents.iter().find(|a| {
                a.current_pos != current_agent.current_pos
                    && manhattan_distance(current_agent.current_pos, a.current_pos) <= 1
            });

            if let Some(next) = blocking_next {
                current_agent = next;
            } else {
                break;
            }
        }

        // デッドロックサイクルを検出したら、ターゲットローテーションを要求
        if deadlock_chain.len() > 1 {
            // 参加者とゴールのリストを作成
            let mut participants = vec![];
            let mut goals = vec![];

            for pos in &deadlock_chain {
                if let Some(agent) = nearby_agents.iter().find(|a| &a.current_pos == pos) {
                    participants.push(agent.peer_id.clone());
                    goals.push(agent.goal_pos);
                }
            }

            if participants.len() > 1 {
                println!("[TSWAP] Deadlock cycle detected, requesting target rotation");
                println!("[TSWAP] Participants: {:?}", participants);
                return TswapAction::WaitForRotation(participants, goals);
            }
        }

        // 移動できない場合は待機
        return TswapAction::Wait;
    }

    // 移動先が空いていれば移動
    TswapAction::Move(next_pos)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
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
                .heartbeat_interval(Duration::from_millis(500)) // 500msでハートビート
                .heartbeat_initial_delay(Duration::from_millis(100)) // 初回ハートビートを100ms後に実行（即座にメッシュ構築）
                .mesh_n_low(1) // メッシュの最小ピア数を1に設定（デフォルト4）
                .mesh_n(2) // 目標メッシュピア数を2に設定（デフォルト6）
                .mesh_n_high(3) // メッシュの最大ピア数を3に設定（デフォルト12）
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
    let local_peer_id_str = swarm.local_peer_id().to_base58();
    println!("✅ Agent Peer ID: {}", local_peer_id_str);
    println!("✅ Subscribed to topic 'mapd'");

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    println!("Enter messages via STDIN and they will be sent to connected peers using MAPD topic");
    println!("PeerId: {}", local_peer_id_str);

    // === Initial Position Decision ===
    let mut my_point: Option<Point> = None;
    let grid = Arc::new(parse_map());
    let mut occupied_points: HashSet<Point> = HashSet::new();
    let free_cells = make_node::get_free_cells(&grid);
    println!("[Initial Position Decision] Waiting for other nodes to be discovered via mDNS...");
    let wait_duration = Duration::from_secs(3);
    let wait_start = std::time::Instant::now();
    let mut discovered_peers: HashSet<String> = HashSet::new();
    while wait_start.elapsed() < wait_duration {
        let timeout = wait_duration - wait_start.elapsed();
        match tokio::time::timeout(
            std::cmp::min(timeout, Duration::from_millis(300)),
            swarm.select_next_some(),
        )
        .await
        {
            Ok(event) => match event {
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    for (peer_id, _multiaddr) in &list {
                        println!("[Initial Position Decision] mDNS discovered peer: {peer_id}");
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        discovered_peers.insert(peer_id.to_base58());
                    }
                }
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Local node is listening on {address}");
                }
                _ => {}
            },
            Err(_) => {}
        }
    }

    // After peer discovery, send occupied_request and receive occupied_response

    println!("[Initial Position Decision] Sending occupied_request");
    // 1. Get peer list
    // Use discovered_peers, which is the peer list found by mDNS above
    // 2. If there are no peers except myself, proceed immediately
    if discovered_peers.is_empty()
        || (discovered_peers.len() == 1 && discovered_peers.contains(&local_peer_id_str))
    {
        println!("[Initial Position Decision] No other peers, proceeding immediately");
    } else {
        // 3. Collect occupied_response from all peers
        let mut received_peers: HashSet<String> = HashSet::new();
        let req_msg = serde_json::json!({"type": "occupied_request", "peer_id": local_peer_id_str})
            .to_string();
        let _ = swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic.clone(), req_msg.as_bytes());
        let collect_timeout = std::time::Duration::from_secs(2);
        let collect_start = std::time::Instant::now();
        while collect_start.elapsed() < collect_timeout {
            if received_peers.len() >= discovered_peers.len() {
                break;
            }
            if let Ok(event) =
                tokio::time::timeout(Duration::from_millis(300), swarm.select_next_some()).await
            {
                if let SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(
                    gossipsub::Event::Message { message, .. },
                )) = event
                {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&message.data) {
                        if val.get("type")
                            == Some(&serde_json::Value::String("occupied_response".to_string()))
                        {
                            if let Some(arr) = val.get("points").and_then(|v| v.as_array()) {
                                for p in arr {
                                    if let (Some(x), Some(y)) = (
                                        p.get(0).and_then(|v| v.as_u64()),
                                        p.get(1).and_then(|v| v.as_u64()),
                                    ) {
                                        occupied_points.insert((x as usize, y as usize));
                                    }
                                }
                            }
                            // PeerId is obtained from message.source
                            if let Some(peer_id) = &message.source {
                                received_peers.insert(peer_id.to_base58());
                            }
                        }
                    }
                }
            }
        }
        println!(
            "[Initial Position Decision] occupied_response collection complete: {}/{}",
            received_peers.len(),
            discovered_peers.len()
        );
    }
    // 3. Randomly select from free_cells excluding occupied_points
    let available_points: Vec<Point> = if occupied_points.is_empty() {
        free_cells.clone()
    } else {
        free_cells
            .iter()
            .filter(|p| !occupied_points.contains(p))
            .cloned()
            .collect()
    };
    my_point = available_points.choose(&mut thread_rng()).cloned();
    if let Some(p) = my_point {
        println!("[Initial Position Decision] My position: {:?}", p);
    } else {
        println!("[Initial Position Decision] No available position");
        return Ok(());
    }

    println!("✅ [READY] Agent is ready! Starting main loop...");
    println!("📍 Initial position: {:?}", my_point.unwrap());
    println!("⏳ Waiting 2 seconds for Gossipsub mesh to form...");

    // Gossipsubメッシュ構築のための待機
    tokio::time::sleep(Duration::from_secs(2)).await;

    println!("🚀 Starting to process tasks!");

    // === Main Loop ===
    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut peer_positions: HashMap<String, Point> = HashMap::new();
    let mut my_task: Option<p2p_distributed_tswap::map::task_generator::Task> = None;
    let mut last_position_broadcast = std::time::Instant::now();
    let mut first_broadcast_success = false;

    // TSWAPのための近隣エージェント管理
    let mut nearby_agents = NearbyAgents::new();
    let mut my_goal: Point = my_point.unwrap_or((0, 0));

    // ゴール交換とターゲットローテーションの管理
    let mut pending_goal_swap: Option<String> = None; // 交換待ちのrequest_id
    let mut pending_rotation: Option<String> = None; // ローテーション待ちのrequest_id
    let mut goal_swap_requests: HashMap<String, GoalSwapRequest> = HashMap::new();
    let mut rotation_requests: HashMap<String, TargetRotationRequest> = HashMap::new();

    // グリッドをノードグラフに変換（TSWAPで使用）
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
    let mut tswap_nodes = vec![];
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
        tswap_nodes.push(Node {
            id,
            pos: (x, y),
            neighbors,
        });
    }
    loop {
        select! {
            Ok(Some(line)) = stdin.next_line() => {
                if let Err(e) = swarm
                    .behaviour_mut().gossipsub
                    .publish(topic.clone(), line.as_bytes()) {
                    println!("Publish error: {e:?}");
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)), if last_position_broadcast.elapsed() > std::time::Duration::from_secs(1) => {
                // Periodically broadcast own position and goal (for TSWAP)
                if let Some(p) = my_point {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    let pos_json = serde_json::json!({
                        "type": "position",
                        "peer_id": local_peer_id_str,
                        "pos": [p.0, p.1],
                        "goal": [my_goal.0, my_goal.1],
                        "timestamp": timestamp
                    }).to_string();
                    match swarm.behaviour_mut().gossipsub.publish(topic.clone(), pos_json.as_bytes()) {
                        Ok(_) => {
                            if !first_broadcast_success {
                                println!("📡 [BROADCAST] Successfully broadcasting position to network!");
                                first_broadcast_success = true;
                            }
                            // デバッグ: 定期的に情報を表示
                            if nearby_agents.agents.len() > 0 {
                                println!("📡 [BROADCAST] Sent position {:?} -> goal {:?} | Nearby agents: {}",
                                         p, my_goal, nearby_agents.agents.len());
                            }
                        }
                        Err(e) => {
                            // NoPeersSubscribedToTopic は正常（他のピアがまだ接続していない）
                            let err_str = format!("{:?}", e);
                            if !err_str.contains("NoPeers") {
                                println!("⚠️  Failed to broadcast position: {e:?}");
                            } else {
                                println!("⏳ [BROADCAST] Waiting for peers to subscribe...");
                            }
                        }
                    }
                } else {
                    println!("⚠️  [BROADCAST] my_point is None, cannot broadcast position");
                }
                // 古いエージェント情報をクリーンアップ
                nearby_agents.cleanup_old(10);
                last_position_broadcast = std::time::Instant::now();
            }
            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Local node is listening on {address}");
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discovered a new peer: {peer_id}");
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discover peer has expired: {peer_id}");
                        swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic })) => {
                    println!("🔗 Peer {} subscribed to topic: {}", peer_id, topic);
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Unsubscribed { peer_id, topic })) => {
                    println!("❌ Peer {} unsubscribed from topic: {}", peer_id, topic);
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                    // 位置情報受信（TSWAPのため、ゴール情報も保存）
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&message.data) {
                        // デバッグ: 受信したメッセージのタイプを表示
                        if let Some(msg_type) = val.get("type").and_then(|v| v.as_str()) {
                            if msg_type != "position" {
                                println!("📨 [RECEIVE] Message type: {}", msg_type);
                            }
                        }

                        if val.get("type") == Some(&serde_json::Value::String("position".to_string())) {
                            if let (Some(peer_id), Some(pos_arr), Some(goal_arr)) =
                                (val.get("peer_id"), val.get("pos"), val.get("goal")) {
                                if let (Some(peer_id_str), Some(pos), Some(goal)) =
                                    (peer_id.as_str(), pos_arr.as_array(), goal_arr.as_array()) {
                                    // 自分自身のメッセージは無視
                                    if peer_id_str == local_peer_id_str {
                                        // println!("🔄 [SKIP] Ignoring own position message");
                                        continue;
                                    }

                                    if pos.len() == 2 && goal.len() == 2 {
                                        if let (Some(px), Some(py), Some(gx), Some(gy)) =
                                            (pos[0].as_u64(), pos[1].as_u64(), goal[0].as_u64(), goal[1].as_u64()) {
                                            let current_pos = (px as usize, py as usize);
                                            let goal_pos = (gx as usize, gy as usize);
                                            peer_positions.insert(peer_id_str.to_string(), current_pos);

                                            // TSWAPのため近隣エージェント情報を更新
                                            let timestamp = val.get("timestamp")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0);
                                            nearby_agents.update(AgentInfo {
                                                peer_id: peer_id_str.to_string(),
                                                current_pos,
                                                goal_pos,
                                                timestamp,
                                            });
                                            println!("📍 [POSITION UPDATE] Agent {}: pos={:?}, goal={:?}, total_tracked={}",
                                                     &peer_id_str[..std::cmp::min(8, peer_id_str.len())],
                                                     current_pos, goal_pos, nearby_agents.agents.len());
                                        }
                                    }
                                }
                            }
                        }
                        // occupied_request/responseは既存通り
                        if let Some(msg_type) = val.get("type") {
                            println!("[DEBUG] message type: {:?}", msg_type);
                        }
                        if val.get("type") == Some(&serde_json::Value::String("occupied_request".to_string())) {
                            // Check peer_id
                            let peer_id_val = val.get("peer_id");
                            if let Some(peer_id_val) = peer_id_val {
                                println!("[DEBUG] occupied_request peer_id: {:?}, my peer_id: {}", peer_id_val, local_peer_id_str);
                            }
                            if let Some(p) = my_point {
                                let points_json = serde_json::json!({
                                    "type": "occupied_response",
                                    "points": [[p.0, p.1]],
                                    "peer_id": peer_id_val.unwrap_or(&serde_json::Value::String(local_peer_id_str.clone()))
                                }).to_string();
                                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), points_json.as_bytes()) {
                                    println!("Error sending occupied_response: {e:?}");
                                } else {
                                    println!("[occupied_response] Sent my position ({:?})", p);
                                }
                            }
                        }
                        if val.get("type") == Some(&serde_json::Value::String("occupied_response".to_string())) {
                            // If occupied_response is received from another node, add to occupied_points
                            if let Some(arr) = val.get("points").and_then(|v| v.as_array()) {
                                for p in arr {
                                    if let (Some(x), Some(y)) = (
                                        p.get(0).and_then(|v| v.as_u64()),
                                        p.get(1).and_then(|v| v.as_u64()),
                                    ) {
                                        occupied_points.insert((x as usize, y as usize));
                                    }
                                }
                            }
                        }

                        // ゴール交換リクエスト受信
                        if val.get("type") == Some(&serde_json::Value::String("goal_swap_request".to_string())) {
                            if let Ok(request) = serde_json::from_value::<GoalSwapRequest>(val.clone()) {
                                if request.to_peer == local_peer_id_str {
                                    println!("[GOAL_SWAP] Received goal swap request from {}", request.from_peer);
                                    println!("[GOAL_SWAP] Their goal: {:?}, My goal: {:?}", request.my_goal, my_goal);

                                    // ゴール交換を受け入れる
                                    let response = GoalSwapResponse {
                                        request_id: request.request_id.clone(),
                                        from_peer: local_peer_id_str.clone(),
                                        to_peer: request.from_peer.clone(),
                                        my_goal,
                                        accepted: true,
                                    };

                                    let response_json = serde_json::to_string(&response).unwrap();
                                    let msg = serde_json::json!({
                                        "type": "goal_swap_response",
                                        "data": response_json
                                    }).to_string();

                                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), msg.as_bytes()) {
                                        println!("[GOAL_SWAP] Failed to send response: {e:?}");
                                    } else {
                                        println!("[GOAL_SWAP] Sent response, swapping goals");
                                        // 自分のゴールを相手のゴールに変更
                                        my_goal = request.my_goal;
                                        goal_swap_requests.insert(request.request_id.clone(), request);
                                    }
                                }
                            }
                        }

                        // ゴール交換レスポンス受信
                        if val.get("type") == Some(&serde_json::Value::String("goal_swap_response".to_string())) {
                            if let Some(data_str) = val.get("data").and_then(|v| v.as_str()) {
                                if let Ok(response) = serde_json::from_str::<GoalSwapResponse>(data_str) {
                                    if response.to_peer == local_peer_id_str && response.accepted {
                                        println!("[GOAL_SWAP] Goal swap accepted by {}", response.from_peer);
                                        println!("[GOAL_SWAP] New goal: {:?}", response.my_goal);
                                        // 自分のゴールを相手のゴールに変更
                                        my_goal = response.my_goal;
                                        pending_goal_swap = None;
                                    }
                                }
                            }
                        }

                        // ターゲットローテーションリクエスト受信
                        if val.get("type") == Some(&serde_json::Value::String("target_rotation_request".to_string())) {
                            if let Ok(request) = serde_json::from_value::<TargetRotationRequest>(val.clone()) {
                                // 自分がparticipantsに含まれているかチェック
                                if let Some(my_index) = request.participants.iter().position(|p| p == &local_peer_id_str) {
                                    println!("[ROTATION] Received rotation request from {}", request.initiator);
                                    println!("[ROTATION] Participants: {:?}", request.participants);

                                    // 次のエージェントのゴールを自分のゴールにする（ローテーション）
                                    let next_index = (my_index + 1) % request.participants.len();
                                    if next_index < request.goals.len() {
                                        let new_goal = request.goals[next_index];
                                        println!("[ROTATION] Rotating goal: {:?} -> {:?}", my_goal, new_goal);
                                        my_goal = new_goal;
                                        rotation_requests.insert(request.request_id.clone(), request);
                                    }
                                }
                            }
                        }

                        // タスクスワップリクエスト受信
                        if val.get("type") == Some(&serde_json::Value::String("swap_request".to_string())) {
                            // swap_request: {type: "swap_request", from_peer: ..., to_peer: ..., task: ...}
                            if let (Some(from_peer), Some(task_val)) = (val.get("from_peer"), val.get("task")) {
                                if let Some(from_peer_str) = from_peer.as_str() {
                                    println!("[SWAP] swap request from {}", from_peer_str);
                                    // Receiver swaps its own task
                                    if let Some(my_task_val) = my_task.clone() {
                                        let swap_response = serde_json::json!({
                                            "type": "swap_response",
                                            "from_peer": local_peer_id_str,
                                            "to_peer": from_peer_str,
                                            "task": my_task_val
                                        }).to_string();
                                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), swap_response.as_bytes()) {
                                            println!("Failed to send swap_response: {e:?}");
                                        } else {
                                            println!("Sent swap_response to {}", from_peer_str);
                                        }
                                        // 受信したタスクに切り替え
                                        if let Ok(new_task) = serde_json::from_value::<p2p_distributed_tswap::map::task_generator::Task>(task_val.clone()) {
                                            my_task = Some(new_task);
                                        }
                                    }
                                }
                            }
                        }
                        // タスクスワップレスポンス受信
                        if val.get("type") == Some(&serde_json::Value::String("swap_response".to_string())) {
                    if let Some(task_val) = val.get("task") {
                        if let Ok(new_task) = serde_json::from_value::<p2p_distributed_tswap::map::task_generator::Task>(task_val.clone()) {
                            println!("[SWAP] Received swapped task");
                            my_task = Some(new_task.clone());
                            // 新しいタスクのpickup/deliveryでTSWAPベースの移動を行う
                            let pickup = Some(new_task.pickup);
                            let delivery = Some(new_task.delivery);
                            if let (Some(pickup), Some(delivery), Some(mut current_pos)) = (pickup, delivery, my_point) {
                                // 1. Move from current position to pickup with TSWAP
                                my_goal = pickup;
                                println!("Worker: Moving to pickup at {:?} using TSWAP (swapped task)", pickup);
                                while current_pos != pickup {
                                    let nearby = nearby_agents.get_nearby(current_pos, 15, &local_peer_id_str);
                                    let action = compute_next_move_with_tswap(
                                        current_pos, my_goal, &nearby, &grid, &pos2id, &tswap_nodes,
                                    );
                                    match action {
                                        TswapAction::Move(next_pos) => {
                                            if next_pos != current_pos {
                                                current_pos = next_pos;
                                                my_point = Some(current_pos);
                                            }
                                        }
                                        _ => {} // 交換リクエストは省略（簡略版）
                                    }
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                }

                                // 2. Move from pickup to delivery with TSWAP
                                my_goal = delivery;
                                println!("Worker: Moving to delivery at {:?} using TSWAP (swapped task)", delivery);
                                while current_pos != delivery {
                                    let nearby = nearby_agents.get_nearby(current_pos, 15, &local_peer_id_str);
                                    let action = compute_next_move_with_tswap(
                                        current_pos, my_goal, &nearby, &grid, &pos2id, &tswap_nodes,
                                    );
                                    match action {
                                        TswapAction::Move(next_pos) => {
                                            if next_pos != current_pos {
                                                current_pos = next_pos;
                                                my_point = Some(current_pos);
                                            }
                                        }
                                        _ => {} // 交換リクエストは省略（簡略版）
                                    }
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                }

                                my_point = Some(current_pos);
                                // 完了通知
                                let done_json = if let Some(task_id) = new_task.task_id {
                                    serde_json::json!({"status": "done", "task_id": task_id}).to_string()
                                } else {
                                    serde_json::json!({"status": "done"}).to_string()
                                };
                                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), done_json.as_bytes()) {
                                    println!("Failed to send completion notification: {e:?}");
                                } else {
                                    println!("Completion notification ({}) sent", done_json);
                                }
                            } else {
                                println!("Worker: invalid pickup or delivery location for swapped task id={:?}", new_task.task_id);
                            }
                        }
                        }
                    }
                        }
                    // タスク受信
                    if let Ok(task) = serde_json::from_slice::<p2p_distributed_tswap::map::task_generator::Task>(&message.data) {
                        if let Some(ref peer_id) = task.peer_id {
                            if peer_id != &local_peer_id_str {
                                continue;
                            }
                        } else {
                            continue;
                        }
                        println!("=========================");
                        println!("📦 [TASK RECEIVED] Task ID: {:?}", task.task_id);
                        println!("   Pickup: {:?} -> Delivery: {:?}", task.pickup, task.delivery);
                        println!("=========================");
                        my_task = Some(task.clone());
                        let pickup = Some(task.pickup);
                        let delivery = Some(task.delivery);
                        // Check if another agent is at the destination
                        let mut swap_sent = false;
                        for (peer, pos) in &peer_positions {
                            if Some(*pos) == pickup || Some(*pos) == delivery {
                                // Send swap request
                                let swap_req = serde_json::json!({
                                    "type": "swap_request",
                                    "from_peer": local_peer_id_str,
                                    "to_peer": peer,
                                    "task": task
                                }).to_string();
                                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), swap_req.as_bytes()) {
                                    println!("Failed to send swap_request: {e:?}");
                                } else {
                                    println!("Sent swap_request to {}", peer);
                                }
                                swap_sent = true;
                                break;
                            }
                        }
                        if swap_sent {
                            println!("[SWAP] Waiting for swap response...");
                            continue;
                        }
                        // Agent must go from current position to pickup, then from pickup to delivery
                        // TSWAPベースの移動ロジックを使用
                        if let (Some(pickup), Some(delivery), Some(mut current_pos)) = (pickup, delivery, my_point) {
                            // 1. Move from current position to pickup with TSWAP
                            my_goal = pickup;
                            println!("🚶 [PHASE 1] Moving to PICKUP at {:?} (current: {:?})", pickup, current_pos);
                            while current_pos != pickup {
                                let nearby = nearby_agents.get_nearby(current_pos, 15, &local_peer_id_str);
                                println!("  📍 Current: {:?} -> {:?} (Nearby: {})", current_pos, my_goal, nearby.len());

                                let action = compute_next_move_with_tswap(
                                    current_pos,
                                    my_goal,
                                    &nearby,
                                    &grid,
                                    &pos2id,
                                    &tswap_nodes,
                                );

                                match action {
                                    TswapAction::Move(next_pos) => {
                                        if next_pos != current_pos {
                                            println!("[TSWAP] Moving {} -> {}",
                                                format!("{:?}", current_pos),
                                                format!("{:?}", next_pos));
                                            current_pos = next_pos;
                                            my_point = Some(current_pos);
                                        }
                                    }
                                    TswapAction::WaitForGoalSwap(peer_id) => {
                                        println!("[TSWAP] Sending goal swap request to {}", peer_id);
                                        let request_id = format!("{}_{}", local_peer_id_str, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
                                        let request = GoalSwapRequest {
                                            request_id: request_id.clone(),
                                            from_peer: local_peer_id_str.clone(),
                                            to_peer: peer_id,
                                            my_goal,
                                        };
                                        let msg = serde_json::to_value(&request).unwrap();
                                        let msg_with_type = serde_json::json!({
                                            "type": "goal_swap_request",
                                            "request_id": request.request_id,
                                            "from_peer": request.from_peer,
                                            "to_peer": request.to_peer,
                                            "my_goal": [request.my_goal.0, request.my_goal.1]
                                        }).to_string();
                                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), msg_with_type.as_bytes()) {
                                            println!("[TSWAP] Failed to send goal swap request: {e:?}");
                                        }
                                        pending_goal_swap = Some(request_id);
                                    }
                                    TswapAction::WaitForRotation(participants, goals) => {
                                        println!("[TSWAP] Sending target rotation request");
                                        println!("[TSWAP] Participants: {:?}", participants);
                                        let request_id = format!("{}_{}", local_peer_id_str, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
                                        let request = TargetRotationRequest {
                                            request_id: request_id.clone(),
                                            initiator: local_peer_id_str.clone(),
                                            participants,
                                            goals,
                                        };
                                        let msg = serde_json::to_value(&request).unwrap();
                                        let msg_with_type = serde_json::json!({
                                            "type": "target_rotation_request",
                                            "request_id": request.request_id,
                                            "initiator": request.initiator,
                                            "participants": request.participants,
                                            "goals": request.goals.iter().map(|g| [g.0, g.1]).collect::<Vec<_>>()
                                        }).to_string();
                                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), msg_with_type.as_bytes()) {
                                            println!("[TSWAP] Failed to send rotation request: {e:?}");
                                        }
                                        pending_rotation = Some(request_id);
                                    }
                                    TswapAction::Wait => {
                                        println!("[TSWAP] Waiting due to collision avoidance...");
                                    }
                                }

                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            }
                            println!("✅ [PHASE 1 COMPLETE] Reached PICKUP at {:?}", pickup);

                            // 2. Move from pickup to delivery with TSWAP
                            my_goal = delivery;
                            println!("🚚 [PHASE 2] Moving to DELIVERY at {:?} (current: {:?})", delivery, current_pos);
                            while current_pos != delivery {
                                let nearby = nearby_agents.get_nearby(current_pos, 15, &local_peer_id_str);
                                println!("  📍 Current: {:?} -> {:?} (Nearby: {})", current_pos, my_goal, nearby.len());

                                let action = compute_next_move_with_tswap(
                                    current_pos,
                                    my_goal,
                                    &nearby,
                                    &grid,
                                    &pos2id,
                                    &tswap_nodes,
                                );

                                match action {
                                    TswapAction::Move(next_pos) => {
                                        if next_pos != current_pos {
                                            println!("[TSWAP] Moving {} -> {}",
                                                format!("{:?}", current_pos),
                                                format!("{:?}", next_pos));
                                            current_pos = next_pos;
                                            my_point = Some(current_pos);
                                        }
                                    }
                                    TswapAction::WaitForGoalSwap(peer_id) => {
                                        println!("[TSWAP] Sending goal swap request to {}", peer_id);
                                        let request_id = format!("{}_{}", local_peer_id_str, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
                                        let request = GoalSwapRequest {
                                            request_id: request_id.clone(),
                                            from_peer: local_peer_id_str.clone(),
                                            to_peer: peer_id,
                                            my_goal,
                                        };
                                        let msg_with_type = serde_json::json!({
                                            "type": "goal_swap_request",
                                            "request_id": request.request_id,
                                            "from_peer": request.from_peer,
                                            "to_peer": request.to_peer,
                                            "my_goal": [request.my_goal.0, request.my_goal.1]
                                        }).to_string();
                                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), msg_with_type.as_bytes()) {
                                            println!("[TSWAP] Failed to send goal swap request: {e:?}");
                                        }
                                        pending_goal_swap = Some(request_id);
                                    }
                                    TswapAction::WaitForRotation(participants, goals) => {
                                        println!("[TSWAP] Sending target rotation request");
                                        println!("[TSWAP] Participants: {:?}", participants);
                                        let request_id = format!("{}_{}", local_peer_id_str, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
                                        let request = TargetRotationRequest {
                                            request_id: request_id.clone(),
                                            initiator: local_peer_id_str.clone(),
                                            participants,
                                            goals,
                                        };
                                        let msg_with_type = serde_json::json!({
                                            "type": "target_rotation_request",
                                            "request_id": request.request_id,
                                            "initiator": request.initiator,
                                            "participants": request.participants,
                                            "goals": request.goals.iter().map(|g| [g.0, g.1]).collect::<Vec<_>>()
                                        }).to_string();
                                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), msg_with_type.as_bytes()) {
                                            println!("[TSWAP] Failed to send rotation request: {e:?}");
                                        }
                                        pending_rotation = Some(request_id);
                                    }
                                    TswapAction::Wait => {
                                        println!("[TSWAP] Waiting due to collision avoidance...");
                                    }
                                }

                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            }
                            println!("✅ [PHASE 2 COMPLETE] Reached DELIVERY at {:?}", delivery);
                            my_point = Some(current_pos);
                        } else {
                            println!("❌ [ERROR] Invalid pickup or delivery location for task id={:?}", task.task_id);
                        }
                        let reached_goal = true; // Goal reached check (should be determined by logic)
                        if reached_goal {
                            // Publish completion notification including task_id
                            let done_json = if let Some(task_id) = task.task_id {
                                serde_json::json!({"status": "done", "task_id": task_id}).to_string()
                            } else {
                                serde_json::json!({"status": "done"}).to_string()
                            };
                            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), done_json.as_bytes()) {
                                println!("❌ [ERROR] Failed to send completion notification: {e:?}");
                            } else {
                                println!("🎉 [TASK COMPLETE] Task ID {:?} finished! Notification sent to manager", task.task_id);
                            }
                        }
                        println!("=========================");
                    }
                },
                _ => {}
            }
        }
    }
    #[allow(unreachable_code)]
    Ok(())
}
