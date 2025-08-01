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
use p2p_distributed_tswap::algorithm::a_star::{
    EdgeReservation, NodeReservation, astar_with_reservation,
};
use p2p_distributed_tswap::map::agent::Agent;
use p2p_distributed_tswap::map::agent::AgentState;
use p2p_distributed_tswap::map::make_node;
use p2p_distributed_tswap::map::map::MAP;
use p2p_distributed_tswap::map::map::Point;
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
    use std::collections::HashMap;
    let mut peer_positions: HashMap<String, Point> = HashMap::new();
    let mut my_task: Option<p2p_distributed_tswap::map::task_generator::Task> = None;
    let mut last_position_broadcast = std::time::Instant::now();
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
                // Periodically broadcast own position
                if let Some(p) = my_point {
                    let pos_json = serde_json::json!({
                        "type": "position",
                        "peer_id": local_peer_id_str,
                        "pos": [p.0, p.1]
                    }).to_string();
                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), pos_json.as_bytes()) {
                        println!("Failed to broadcast position: {e:?}");
                    }
                }
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
                    // 位置情報受信
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&message.data) {
                        if val.get("type") == Some(&serde_json::Value::String("position".to_string())) {
                            if let (Some(peer_id), Some(pos_arr)) = (val.get("peer_id"), val.get("pos")) {
                                if let (Some(peer_id_str), Some(arr)) = (peer_id.as_str(), pos_arr.as_array()) {
                                    if arr.len() == 2 {
                                        if let (Some(x), Some(y)) = (arr[0].as_u64(), arr[1].as_u64()) {
                                            peer_positions.insert(peer_id_str.to_string(), (x as usize, y as usize));
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
                            // 新しいタスクのpickup/deliveryで経路計算をやり直す
                            let node_res: std::collections::HashSet<((usize, usize), usize)> = std::collections::HashSet::new();
                            let edge_res: std::collections::HashSet<(((usize, usize), (usize, usize)), usize)> = std::collections::HashSet::new();
                            let pickup = Some(new_task.pickup);
                            let delivery = Some(new_task.delivery);
                            if let (Some(pickup), Some(delivery), Some(mut current_pos)) = (pickup, delivery, my_point) {
                                // 1. Move from current position to pickup
                                if current_pos != pickup {
                                    println!("Worker: Moving to pickup at {:?}", pickup);
                                    match astar_with_reservation(&grid, current_pos, pickup, &node_res, &edge_res, 0) {
                                        Some(path) => {
                                            println!("Worker: path to pickup for swapped task id={:?}: {:?}", new_task.task_id, path);
                                            for (step, pos) in path.iter().enumerate() {
                                                println!("Worker: swapped task id={:?} to pickup step {:?}: pos={:?}", new_task.task_id, step, pos);
                                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                                current_pos = *pos;
                                            }
                                        }
                                        None => {
                                            println!("Worker: path to pickup not found for swapped task id={:?}", new_task.task_id);
                                            continue;
                                        }
                                    }
                                }
                                // 2. Move from pickup to delivery
                                match astar_with_reservation(&grid, pickup, delivery, &node_res, &edge_res, 0) {
                                    Some(path) => {
                                        println!("Worker: path to delivery for swapped task id={:?}: {:?}", new_task.task_id, path);
                                        for (step, pos) in path.iter().enumerate() {
                                            println!("Worker: swapped task id={:?} to delivery step {:?}: pos={:?}", new_task.task_id, step, pos);
                                            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                                            current_pos = *pos;
                                        }
                                    }
                                    None => {
                                        println!("Worker: path to delivery not found for swapped task id={:?}", new_task.task_id);
                                        continue;
                                    }
                                }
                                // Update my_point to the new position
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
                        let node_res: std::collections::HashSet<((usize, usize), usize)> = std::collections::HashSet::new();
                        let edge_res: std::collections::HashSet<(((usize, usize), (usize, usize)), usize)> = std::collections::HashSet::new();
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
                        if let (Some(pickup), Some(delivery), Some(mut current_pos)) = (pickup, delivery, my_point) {
                            // 1. Move from current position to pickup
                            if current_pos != pickup {
                                println!("Worker: Moving to pickup at {:?}", pickup);
                                match astar_with_reservation(&grid, current_pos, pickup, &node_res, &edge_res, 0) {
                                    Some(path) => {
                                        println!("Worker: path to pickup for task id={:?}: {:?}", task.task_id, path);
                                        for (step, pos) in path.iter().enumerate() {
                                            println!("Worker: task id={:?} to pickup step {:?}: pos={:?}", task.task_id, step, pos);
                                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                            current_pos = *pos;
                                        }
                                    }
                                    None => {
                                        println!("Worker: path to pickup not found for task id={:?}", task.task_id);
                                        continue;
                                    }
                                }
                            }
                            // 2. Move from pickup to delivery
                            match astar_with_reservation(&grid, pickup, delivery, &node_res, &edge_res, 0) {
                                Some(path) => {
                                    println!("Worker: path to delivery for task id={:?}: {:?}", task.task_id, path);
                                    for (step, pos) in path.iter().enumerate() {
                                        println!("Worker: task id={:?} to delivery step {:?}: pos={:?}", task.task_id, step, pos);
                                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                                        current_pos = *pos;
                                    }
                                }
                                None => {
                                    println!("Worker: path to delivery not found for task id={:?}", task.task_id);
                                    continue;
                                }
                            }
                            // Update my_point to the new position
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
