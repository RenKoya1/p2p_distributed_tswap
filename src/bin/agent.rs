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

    fn get_nearby(&self, my_pos: Point, radius: usize) -> Vec<AgentInfo> {
        self.agents
            .values()
            .filter(|agent| manhattan_distance(my_pos, agent.current_pos) <= radius)
            .cloned()
            .collect()
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
fn compute_next_move_with_tswap(
    my_pos: Point,
    my_goal: Point,
    nearby_agents: &[AgentInfo],
    _grid: &[Vec<char>],
    pos2id: &HashMap<Point, usize>,
    nodes: &[Node],
) -> Point {
    // 自分がゴールに到達している場合は移動しない
    if my_pos == my_goal {
        return my_pos;
    }

    // 自分の現在位置から次に進むべき位置を計算
    let path = get_path(pos2id[&my_pos], pos2id[&my_goal], nodes);
    if path.len() < 2 {
        return my_pos;
    }

    let next_node_id = path[1];
    let next_pos = nodes[next_node_id].pos;

    // 次の位置に他のエージェントがいるかチェック
    if let Some(blocking_agent) = nearby_agents.iter().find(|a| a.current_pos == next_pos) {
        // TSWAPロジック: 相手がゴールにいる場合はゴールを交換
        if blocking_agent.current_pos == blocking_agent.goal_pos {
            println!("[TSWAP] Agent at goal, requesting goal swap");
            // ゴール交換はメッセージングで行う（後で実装）
            return my_pos; // 今回は移動しない
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

        // 移動できない場合は待機
        return my_pos;
    }

    // 移動先が空いていれば移動
    next_pos
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
                .heartbeat_interval(Duration::from_secs(10))
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

    // === Main Loop ===
    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut peer_positions: HashMap<String, Point> = HashMap::new();
    let mut my_task: Option<p2p_distributed_tswap::map::task_generator::Task> = None;
    let mut last_position_broadcast = std::time::Instant::now();

    // TSWAPのための近隣エージェント管理
    let mut nearby_agents = NearbyAgents::new();
    let mut my_goal: Point = my_point.unwrap_or((0, 0));

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
                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), pos_json.as_bytes()) {
                        println!("Failed to broadcast position: {e:?}");
                    }
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
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                    println!("Received message: {:?}", message);
                    // 位置情報受信（TSWAPのため、ゴール情報も保存）
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&message.data) {
                        if val.get("type") == Some(&serde_json::Value::String("position".to_string())) {
                            if let (Some(peer_id), Some(pos_arr), Some(goal_arr)) =
                                (val.get("peer_id"), val.get("pos"), val.get("goal")) {
                                if let (Some(peer_id_str), Some(pos), Some(goal)) =
                                    (peer_id.as_str(), pos_arr.as_array(), goal_arr.as_array()) {
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
                                    let nearby = nearby_agents.get_nearby(current_pos, 5);
                                    let next_pos = compute_next_move_with_tswap(
                                        current_pos, my_goal, &nearby, &grid, &pos2id, &tswap_nodes,
                                    );
                                    if next_pos != current_pos {
                                        current_pos = next_pos;
                                        my_point = Some(current_pos);
                                    }
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                }

                                // 2. Move from pickup to delivery with TSWAP
                                my_goal = delivery;
                                println!("Worker: Moving to delivery at {:?} using TSWAP (swapped task)", delivery);
                                while current_pos != delivery {
                                    let nearby = nearby_agents.get_nearby(current_pos, 5);
                                    let next_pos = compute_next_move_with_tswap(
                                        current_pos, my_goal, &nearby, &grid, &pos2id, &tswap_nodes,
                                    );
                                    if next_pos != current_pos {
                                        current_pos = next_pos;
                                        my_point = Some(current_pos);
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
                        println!("-------------------------", );
                        println!("Received task: {:?}", task);
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
                            println!("Worker: Moving to pickup at {:?} using TSWAP", pickup);
                            while current_pos != pickup {
                                let nearby = nearby_agents.get_nearby(current_pos, 5);
                                println!("[TSWAP] Current: {:?}, Goal: {:?}, Nearby agents: {}", current_pos, my_goal, nearby.len());

                                let next_pos = compute_next_move_with_tswap(
                                    current_pos,
                                    my_goal,
                                    &nearby,
                                    &grid,
                                    &pos2id,
                                    &tswap_nodes,
                                );

                                if next_pos == current_pos {
                                    println!("[TSWAP] Waiting due to collision avoidance...");
                                } else {
                                    println!("[TSWAP] Moving {} -> {}",
                                        format!("{:?}", current_pos),
                                        format!("{:?}", next_pos));
                                    current_pos = next_pos;
                                    my_point = Some(current_pos);
                                }

                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            }
                            println!("Worker: Reached pickup at {:?}", pickup);

                            // 2. Move from pickup to delivery with TSWAP
                            my_goal = delivery;
                            println!("Worker: Moving to delivery at {:?} using TSWAP", delivery);
                            while current_pos != delivery {
                                let nearby = nearby_agents.get_nearby(current_pos, 5);
                                println!("[TSWAP] Current: {:?}, Goal: {:?}, Nearby agents: {}", current_pos, my_goal, nearby.len());

                                let next_pos = compute_next_move_with_tswap(
                                    current_pos,
                                    my_goal,
                                    &nearby,
                                    &grid,
                                    &pos2id,
                                    &tswap_nodes,
                                );

                                if next_pos == current_pos {
                                    println!("[TSWAP] Waiting due to collision avoidance...");
                                } else {
                                    println!("[TSWAP] Moving {} -> {}",
                                        format!("{:?}", current_pos),
                                        format!("{:?}", next_pos));
                                    current_pos = next_pos;
                                    my_point = Some(current_pos);
                                }

                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            }
                            println!("Worker: Reached delivery at {:?}", delivery);
                            my_point = Some(current_pos);
                        } else {
                            println!("Worker: invalid pickup or delivery location for task id={:?}", task.task_id);
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
                                println!("Failed to send completion notification: {e:?}");
                            } else {
                                println!("Completion notification ({}) sent", done_json);
                            }
                        }
                        println!("--------------------------");
                    }
                },
                _ => {}
            }
        }
    }
    #[allow(unreachable_code)]
    Ok(())
}
