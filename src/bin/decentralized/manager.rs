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

use std::collections::HashMap;
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::{io, io::AsyncBufReadExt, select};
fn parse_map() -> Vec<Vec<char>> {
    let grid = MAP
        .replace('\r', "")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().collect::<Vec<char>>())
        .collect::<Vec<_>>();

    // debug print
    for row in &grid {
        println!("{}", row.iter().collect::<String>());
    }

    grid
}

#[derive(NetworkBehaviour)]
struct MapdBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Check for --clean flag to ignore mDNS discoveries
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
                .heartbeat_interval(Duration::from_millis(500)) // Heartbeat every 500ms
                .heartbeat_initial_delay(Duration::from_millis(100)) // Initial heartbeat after 100ms (immediate mesh construction)
                .mesh_n_low(1) // Minimum mesh peers set to 1 (default 4)
                .mesh_n(2) // Target mesh peers set to 2 (default 6)
                .mesh_n_high(3) // Maximum mesh peers set to 3 (default 12)
                .validation_mode(gossipsub::ValidationMode::Permissive)
                .message_id_fn(message_id_fn)
                .history_length(5)  // メッセージ履歴を5に制限（デフォルト5だが明示）
                .history_gossip(3)  // Gossip履歴を3に制限（デフォルト3だが明示）
                .max_transmit_size(1_048_576)  // 最大送信サイズを1MBに制限
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
    println!("Peer ID: {}", swarm.local_peer_id());

    // Create grid (pass appropriate grid in actual use)
    let grid = Arc::new(parse_map());
    let mut task_gen = TaskGeneratorAgent::new(&grid);
    let mut stdin = io::BufReader::new(io::stdin()).lines();

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    println!("✅ Manager started fresh!");
    println!("⏳ Clearing any cached peer information...");

    // Give a brief moment to ensure clean state
    tokio::time::sleep(Duration::from_millis(500)).await;

    println!("Enter messages via STDIN and they will be sent to connected peers using MAPD topic");
    println!("Type 'task' to generate and send a task to agents.");
    println!(
        "Use 'metrics' for summary stats, 'save <filename>' for task metrics CSV, and 'save path <filename>' for path computation CSV."
    );
    println!(
        "⚠️  IMPORTANT: Wait 3-5 seconds after all agents connect before sending tasks (for Gossipsub mesh to form)!"
    );
    println!(
        "💡 TIP: Look for '🔗 Peer XXX subscribed to topic: mapd' messages to confirm mesh is ready!"
    );
    println!("⏳ Waiting 1 second for initial Gossipsub mesh setup...");

    // Wait for Gossipsub mesh initialization
    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("✅ Manager ready! Listening for agents...");

    // Management variables
    let mut known_peers: HashSet<libp2p::PeerId> = HashSet::new();
    // Peers subscribed to topic (joined Gossipsub mesh)
    let mut subscribed_peers: HashSet<libp2p::PeerId> = HashSet::new();
    // Task in progress for each peer: peer_id -> Option<Task>
    let mut peer_task_map: HashMap<libp2p::PeerId, Option<Task>> = HashMap::new();
    // Task ID to peer mapping: task_id -> peer_id
    let mut task_peer_map: HashMap<u64, libp2p::PeerId> = HashMap::new();
    // Task generation counter
    let mut task_counter: u64 = 0;
    // Track current position of each agent: peer_id -> (x, y)
    let mut peer_positions: HashMap<String, (usize, usize)> = HashMap::new();

    // === Task Metrics Collection ===
    let mut metrics_collector = TaskMetricsCollector::new();
    let mut path_metrics = PathComputationMetrics::new();
    println!("📊 Task metrics collection initialized");
    println!("⏱️ Path computation metrics collection initialized");

    loop {
        select! {
            Ok(Some(line)) = stdin.next_line() => {
                let trimmed = line.trim();

                // メトリクス表示コマンド
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

                // Peerをリセットするコマンド
                if trimmed == "reset" {
                    known_peers.clear();
                    subscribed_peers.clear();
                    peer_task_map.clear();
                    task_peer_map.clear();
                    peer_positions.clear();
                    metrics_collector = TaskMetricsCollector::new();
                    task_counter = 0;
                    path_metrics.clear();
                    println!("✅ All peers and state cleared. Ready for fresh start!");
                    continue;
                }

                // CSV保存コマンド
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
                        Err(e) => println!("⚠️  Failed to save metrics: {e:?}"),
                    }
                    continue;
                }

                // タスク分割・送信コマンド
                if trimmed.starts_with("tasks ") {
                    let num_str = &trimmed[6..];
                    if let Ok(num_tasks) = num_str.parse::<usize>() {
                        // Gossipsubから実際に購読しているピアを取得して同期
                        for peer in swarm.behaviour_mut().gossipsub.all_peers() {
                            if peer.1.iter().any(|t| t.as_str() == "mapd") {
                                subscribed_peers.insert(peer.0.clone());
                            }
                        }

                        println!("📡 Sending {} tasks to subscribed peers...", num_tasks);
                        println!("   Subscribed peers: {}", subscribed_peers.len());

                        let mut sent_count = 0;
                        let mut round = 0;

                        // ラウンドベースでタスクを配分
                        while sent_count < num_tasks {
                            let mut sent_in_round = false;
                            for peer_id in &subscribed_peers {
                                if sent_count >= num_tasks {
                                    break;
                                }

                                let busy = peer_task_map.get(peer_id).and_then(|t| t.as_ref()).is_some();
                                if !busy {
                                    if let Some(mut task) = task_gen.generate_task() {
                                        task_counter += 1;
                                        let task_id = task_counter;
                                        task.peer_id = Some(peer_id.to_base58());
                                        task.task_id = Some(task_id);

                                        // タスク計測情報を作成
                                        let metric = TaskMetric::new(task_id, peer_id.to_base58());
                                        metrics_collector.add_metric(metric);

                                        match serde_json::to_vec(&task) {
                                            Ok(task_bytes) => {
                                                match swarm.behaviour_mut().gossipsub.publish(topic.clone(), task_bytes) {
                                                    Ok(_) => {
                                                        println!("✅ Task {} sent to {} (round {})", task_id, peer_id, round + 1);
                                                        peer_task_map.insert(peer_id.clone(), Some(task.clone()));
                                                        task_peer_map.insert(task_id, peer_id.clone());
                                                        sent_count += 1;
                                                        sent_in_round = true;
                                                    }
                                                    Err(e) => {
                                                        println!("⚠️  Task publish error for {}: {e:?}", peer_id);
                                                    }
                                                }
                                            },
                                            Err(e) => println!("Task serialization error: {e:?}"),
                                        }
                                        tokio::time::sleep(Duration::from_millis(100)).await;
                                    }
                                }
                            }

                            if !sent_in_round {
                                println!("⚠️  No agents available in round {}", round + 1);
                                break;
                            }
                            round += 1;
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }

                        println!("✅ Sent {} tasks in {} rounds", sent_count, round);
                        println!("💡 Tip: Use 'metrics' to view statistics, 'save <filename>' for task metrics, or 'save path <filename>' for path metrics");
                        continue;
                    } else {
                        println!("⚠️  Invalid number of tasks. Usage: tasks <number>");
                        continue;
                    }
                }

                if trimmed == "task" {
                    // Gossipsubから実際に購読しているピアを取得して同期
                    for peer in swarm.behaviour_mut().gossipsub.all_peers() {
                        if peer.1.iter().any(|t| t.as_str() == "mapd") {
                            subscribed_peers.insert(peer.0.clone());
                        }
                    }

                    println!("Known peers (mDNS): {:?}", known_peers);
                    println!("Subscribed peers (Gossipsub): {:?}", subscribed_peers);
                    println!("📡 Sending tasks to subscribed peers...");

                    let mut assigned = false;

                    // subscribed_peersのみに送信
                    for peer_id in &subscribed_peers {
                        let busy = peer_task_map.get(peer_id).and_then(|t| t.as_ref()).is_some();
                        if !busy {
                            if let Some(mut task) = task_gen.generate_task() {
                                // タスクIDを付与
                                task_counter += 1;
                                let task_id = task_counter;
                                task.peer_id = Some(peer_id.to_base58());
                                task.task_id = Some(task_id);

                                // タスク計測情報を作成
                                let metric = TaskMetric::new(task_id, peer_id.to_base58());
                                metrics_collector.add_metric(metric);

                                match serde_json::to_vec(&task) {
                                    Ok(task_bytes) => {
                                        match swarm.behaviour_mut().gossipsub.publish(topic.clone(), task_bytes) {
                                            Ok(_) => {
                                                println!("✅ Task {} sent to {peer_id}: {:?}", task_id, task);
                                                peer_task_map.insert(peer_id.clone(), Some(task.clone()));
                                                task_peer_map.insert(task_id, peer_id.clone());
                                                assigned = true;
                                            }
                                            Err(e) => {
                                                println!("⚠️  Task publish error for {peer_id}: {e:?}");
                                            }
                                        }
                                    },
                                    Err(e) => println!("Task serialization error: {e:?}"),
                                }
                                tokio::time::sleep(Duration::from_millis(150)).await;
                            } else {
                                println!("Task generation failed (not enough free cells)");
                            }
                        }
                    }                    if !assigned {
                        if subscribed_peers.is_empty() {
                            println!("⚠️  No peers have subscribed to the topic yet.");
                            println!("💡 Tip: Wait for '🔗 Peer XXX subscribed to topic: mapd' messages, then try 'task' again.");
                        } else {
                            println!("⚠️  All subscribed peers are busy with tasks.");
                        }
                    }
                } else if trimmed != "metrics" && trimmed != "task" && !trimmed.starts_with("save ") && !trimmed.starts_with("tasks ") {
                    if let Err(e) = swarm
                        .behaviour_mut().gossipsub
                        .publish(topic.clone(), line.as_bytes()) {
                        println!("Publish error: {e:?}");
                    }
                }
            }
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    if ignore_mdns {
                        // In clean mode, ignore all mDNS discoveries
                        for (peer_id, _multiaddr) in list {
                            println!("⏭️  Ignoring mDNS peer (--clean mode): {peer_id}");
                        }
                    } else {
                        for (peer_id, _multiaddr) in list {
                            println!("mDNS discovered a new peer: {peer_id}");
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                            known_peers.insert(peer_id.clone());
                            peer_task_map.entry(peer_id.clone()).or_insert(None);

                            // 少し待ってからGossipsubの購読状態をチェック
                            tokio::time::sleep(Duration::from_millis(100)).await;

                            // ピアがトピックに購読しているかチェック
                            for peer_info in swarm.behaviour_mut().gossipsub.all_peers() {
                                if peer_info.0 == &peer_id && peer_info.1.iter().any(|t| t.as_str() == "mapd") {
                                    subscribed_peers.insert(peer_id.clone());
                                    println!("   ✅ Peer {} is already subscribed to 'mapd'", peer_id);
                                    break;
                                }
                            }
                        }
                    }
                },
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discover peer has expired: {peer_id}");
                        swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                        known_peers.remove(&peer_id);
                        subscribed_peers.remove(&peer_id);
                        peer_task_map.remove(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic })) => {
                    println!("🔗 Peer {} subscribed to topic: {}", peer_id, topic);
                    subscribed_peers.insert(peer_id);
                    println!("   ✅ Total subscribed peers: {}", subscribed_peers.len());
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Unsubscribed { peer_id, topic })) => {
                    println!("❌ Peer {} unsubscribed from topic: {}", peer_id, topic);
                    subscribed_peers.remove(&peer_id);
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                    propagation_source: peer_id,
                    message_id: _id,
                    message,
                })) => {
                    let msg_str = String::from_utf8_lossy(&message.data);

                    // occupied_requestの処理
                    if let Ok(request) = serde_json::from_str::<serde_json::Value>(&msg_str) {
                        if request.get("type") == Some(&serde_json::Value::String("occupied_request".to_string())) {
                            println!("📍 Received occupied_request from {peer_id}");

                            // 現在占有されている位置のリストを作成
                            let occupied: Vec<(usize, usize)> = peer_positions.values().cloned().collect();

                            // タイムスタンプを追加して毎回ユニークなメッセージにする
                            let timestamp = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis();

                            let response = serde_json::json!({
                                "type": "occupied_response",
                                "occupied": occupied,
                                "timestamp": timestamp,
                                "from_peer": peer_id.to_base58()
                            });

                            if let Ok(response_bytes) = serde_json::to_vec(&response) {
                                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), response_bytes) {
                                    println!("⚠️  Failed to send occupied_response: {e:?}");
                                } else {
                                    println!("✅ Sent occupied_response with {} positions (timestamp: {})", occupied.len(), timestamp);
                                }
                            }
                            continue;
                        }

                        // 位置情報の更新（position_updateメッセージ）
                        if request.get("type") == Some(&serde_json::Value::String("position_update".to_string())) {
                            if let (Some(peer_id_str), Some(pos)) = (
                                request.get("peer_id").and_then(|v| v.as_str()),
                                request.get("position").and_then(|v| v.as_array())
                            ) {
                                if pos.len() == 2 {
                                    if let (Some(x), Some(y)) = (pos[0].as_u64(), pos[1].as_u64()) {
                                        peer_positions.insert(peer_id_str.to_string(), (x as usize, y as usize));
                                        println!("📍 Updated position for {}: ({}, {})", peer_id_str, x, y);
                                    }
                                }
                            }
                            continue;
                        }

                        // タスク計測情報の受信
                        if let Some(msg_type) = request.get("type").and_then(|v| v.as_str()) {
                            match msg_type {
                                "task_metric_received" => {
                                    if let Some(task_id) = request.get("task_id").and_then(|v| v.as_u64()) {
                                        metrics_collector.update_received(task_id);
                                        println!("   📊 Task {} received by agent", task_id);
                                    }
                                    continue;
                                }
                                "task_metric_started" => {
                                    if let Some(task_id) = request.get("task_id").and_then(|v| v.as_u64()) {
                                        metrics_collector.update_started(task_id);
                                        println!("   📊 Task {} started processing", task_id);
                                    }
                                    continue;
                                }
                                "task_metric_completed" => {
                                    if let Some(task_id) = request.get("task_id").and_then(|v| v.as_u64()) {
                                        metrics_collector.update_completed(task_id);
                                        println!("   📊 Task {} marked as completed", task_id);
                                    }
                                    continue;
                                }
                                "path_metric" => {
                                    if let Some(duration) = request.get("duration_micros").and_then(|v| v.as_u64()) {
                                        path_metrics.record_micros(duration as u128);
                                        println!(
                                            "⏱️ Path metric from {}: {:.3} ms",
                                            peer_id,
                                            duration as f64 / 1000.0
                                        );
                                    }
                                    continue;
                                }
                                _ => {}
                            }
                        }
                    }

                    // Receive completion notification as JSON, and redistribute tasks only if status=="done" and task_id exists
                    if let Ok(done_msg) = serde_json::from_str::<serde_json::Value>(&msg_str) {
                        if done_msg.get("status") == Some(&serde_json::Value::String("done".to_string())) {
                            let task_id = done_msg.get("task_id").and_then(|v| v.as_u64());
                            println!("✅ Received task completion notification: {peer_id}, task_id: {:?}", task_id);

                            peer_task_map.insert(peer_id.clone(), None);
                            // 新しいタスクを生成して配布
                            if let Some(mut task) = task_gen.generate_task() {
                                task_counter += 1;
                                let new_task_id = task_counter;
                                task.peer_id = Some(peer_id.to_base58());
                                task.task_id = Some(new_task_id);

                                // 新しいタスクの計測情報を作成
                                let metric = TaskMetric::new(new_task_id, peer_id.to_base58());
                                metrics_collector.add_metric(metric);

                                match serde_json::to_vec(&task) {
                                    Ok(task_bytes) => {
                                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), task_bytes) {
                                            println!("Task publish error: {e:?}");
                                        } else {
                                            println!("✅ Task {} sent to {peer_id}: {:?}", new_task_id, task);
                                            peer_task_map.insert(peer_id.clone(), Some(task.clone()));
                                            task_peer_map.insert(new_task_id, peer_id.clone());
                                        }
                                    },
                                    Err(e) => println!("Task serialization error: {e:?}"),
                                }
                            } else {
                                println!("Task generation failed (not enough free cells)");
                            }
                        }
                    }
                },
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Local node is listening on {address}");
                }
                _ => {}
            }
        }
    }
}
