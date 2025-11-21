use futures::stream::StreamExt;
use libp2p::{
    gossipsub, mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use p2p_distributed_tswap::map::map::MAP;
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    hash::{Hash, Hasher},
    time::Duration,
};
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

#[allow(dead_code)]
fn broadcast_position(
    swarm: &mut libp2p::Swarm<MapdBehaviour>,
    topic: &gossipsub::IdentTopic,
    peer_id: &str,
    position: Point,
) {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let position_update = serde_json::json!({
        "type": "position_update",
        "peer_id": peer_id,
        "position": [position.0, position.1],
        "timestamp": timestamp
    });
    if let Ok(update_bytes) = serde_json::to_vec(&position_update) {
        let _ = swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic.clone(), update_bytes);
    }
}

// マネージャーからの移動指示
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MoveInstruction {
    peer_id: String,
    next_pos: Point,
    timestamp: u64,
}

#[derive(NetworkBehaviour)]
struct MapdBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🤖 ============================================");
    println!("🤖 [SIMPLE AGENT] Starting...");
    println!("🤖 This agent follows centralized instructions!");
    println!("🤖 ============================================");

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
                .heartbeat_interval(Duration::from_secs(3)) // 250ms→3秒: Agent側はさらに長く
                .heartbeat_initial_delay(Duration::from_secs(1)) // 初期遅延を1秒に
                .mesh_n_low(1) // Managerとのみ接続
                .mesh_n(1) // メッシュサイズ1: Manager 1対1接続
                .mesh_n_high(1) // 最大1: 他のAgentとメッシュ形成しない
                .validation_mode(gossipsub::ValidationMode::Permissive)
                .message_id_fn(message_id_fn)
                .history_length(2) // 最小履歴: Agentは履歴不要
                .history_gossip(1) // Gossip履歴最小化
                .max_transmit_size(131_072) // 128KB: 位置更新には十分
                .max_ihave_length(50) // IHAVE制限を半分に
                .max_ihave_messages(5) // IHAVEメッセージ数削減
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
    println!("✅ Simple Agent Peer ID: {}", local_peer_id_str);
    println!("✅ Subscribed to topic 'mapd'");

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // 初期位置決定（既存のagent.rsと同じロジック）
    let mut my_point: Option<Point> = None;
    let grid = parse_map();

    println!("[Initial Position] Agent will NOT connect to other agents via mDNS");
    println!("[Initial Position] Only Manager will discover and connect to this agent");
    let wait_duration = Duration::from_secs(3);
    let wait_start = std::time::Instant::now();

    while wait_start.elapsed() < wait_duration {
        let timeout = wait_duration - wait_start.elapsed();
        match tokio::time::timeout(
            std::cmp::min(timeout, Duration::from_millis(300)),
            swarm.select_next_some(),
        )
        .await
        {
            Ok(event) => match event {
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Discovered(_list))) => {
                    // Agent同士の接続を防ぐため、mDNS発見を完全に無視
                    // Managerだけがadd_explicit_peerを使用してエージェントに接続
                }
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("🎧 Listening on {address}");
                }
                _ => {}
            },
            Err(_) => {}
        }
    }

    println!("[Initial Position] Waiting for Gossipsub mesh...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 初期位置を取得（簡略化：グリッドから適当な空きセルを選択）
    use rand::seq::SliceRandom;
    use rand::thread_rng;

    let mut free_cells = vec![];
    for y in 0..grid.len() {
        for x in 0..grid[0].len() {
            if grid[y][x] != '@' {
                free_cells.push((x, y));
            }
        }
    }

    my_point = free_cells.choose(&mut thread_rng()).cloned();

    if let Some(p) = my_point {
        println!("📍 My initial position: {:?}", p);
        broadcast_position(&mut swarm, &topic, &local_peer_id_str, p);
    } else {
        println!("❌ No available position");
        return Ok(());
    }

    println!("✅ [READY] Simple Agent is ready!");
    println!("⏳ Waiting for peers and Gossipsub mesh formation...");

    // Managerとの接続とGossipsub mesh形成を待つ
    let discovery_start = std::time::Instant::now();
    let discovery_duration = Duration::from_secs(8);
    let mut subscribed_peers_count = 0;

    while discovery_start.elapsed() < discovery_duration {
        match tokio::time::timeout(Duration::from_millis(500), swarm.select_next_some()).await {
            Ok(event) => match event {
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Discovered(_list))) => {
                    // Agent同士の接続を防ぐため、mDNS発見を完全に無視
                    // Managerだけがこのエージェントに接続する
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(
                    gossipsub::Event::Subscribed { peer_id, .. },
                )) => {
                    println!(
                        "🎯 [AGENT] Peer {} subscribed to topic!",
                        &peer_id.to_base58()[..8]
                    );
                    subscribed_peers_count += 1;
                }
                _ => {}
            },
            Err(_) => {}
        }

        // 少なくとも1つのピアがsubscribeしたら、さらに1秒待ってから進む
        if subscribed_peers_count > 0 && discovery_start.elapsed() > Duration::from_secs(4) {
            println!(
                "✅ Found {} subscribed peers, finalizing mesh...",
                subscribed_peers_count
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
            break;
        }
    }

    if subscribed_peers_count == 0 {
        println!(
            "⚠️  No subscribed peers detected after {}s, proceeding anyway...",
            discovery_duration.as_secs()
        );
    }

    println!("🚀 Starting to broadcast position!");

    // 初期位置をマネージャーに複数回送信（確実に届くように）
    if let Some(p) = my_point {
        println!("📡 Broadcasting initial position {} times...", 3);
        for i in 0..3 {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            let position_update = serde_json::json!({
                "type": "position_update",
                "peer_id": local_peer_id_str,
                "position": [p.0, p.1],
                "timestamp": timestamp
            });
            if i == 0 {
                println!("📡 [DEBUG] Sending initial position: {:?}", position_update);
            }
            if let Ok(update_bytes) = serde_json::to_vec(&position_update) {
                match swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), update_bytes)
                {
                    Ok(_) => {
                        if i == 0 {
                            println!("✅ Sent initial position to manager: {:?}", p);
                        } else if i % 3 == 0 {
                            println!("📤 Retrying position broadcast ({}/10)...", i + 1);
                        }
                    }
                    Err(e) => {
                        println!("⚠️  Failed to send position (attempt {}): {:?}", i + 1, e);
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await; // 300ms→500ms
        }
        println!("✅ Initial position broadcast complete!");
    }

    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut last_position_broadcast = std::time::Instant::now();
    let mut my_task: Option<p2p_distributed_tswap::map::task_generator::Task> = None;

    loop {
        select! {
            Ok(Some(line)) = stdin.next_line() => {
                if let Err(e) = swarm
                    .behaviour_mut().gossipsub
                    .publish(topic.clone(), line.as_bytes()) {
                    println!("❌ Publish error: {e:?}");
                }
            }

            _ = tokio::time::sleep(Duration::from_millis(500)), if last_position_broadcast.elapsed() > Duration::from_secs(1) => {
                // 定期的に位置情報をマネージャーに送信（頻度を下げてネットワーク負荷削減）
                if let Some(p) = my_point {
                    broadcast_position(&mut swarm, &topic, &local_peer_id_str, p);
                }
                last_position_broadcast = std::time::Instant::now();
            }

            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("🎧 Listening on {address}");
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Discovered(_list))) => {
                    // Agent同士の接続を防ぐため、mDNS発見を完全に無視
                },
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Expired(_list))) => {
                    // Agent同士の接続を防ぐため、mDNS expiredも無視
                },
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic })) => {
                    println!("🔗 [AGENT] Peer {} subscribed to topic: {}", peer_id, topic);
                    if peer_id.to_base58() != local_peer_id_str {
                        println!("🎯 [AGENT] Manager likely connected: {}", peer_id);
                    }
                }
                SwarmEvent::Behaviour(MapdBehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&message.data) {
                        // マネージャーからの移動指示を受信
                        if val.get("type") == Some(&serde_json::Value::String("move_instruction".to_string())) {
                            if let Some(target_peer) = val.get("peer_id").and_then(|v| v.as_str()) {
                                if target_peer == local_peer_id_str {
                                    if let Some(next_pos_arr) = val.get("next_pos").and_then(|v| v.as_array()) {
                                        if next_pos_arr.len() == 2 {
                                            if let (Some(x), Some(y)) = (next_pos_arr[0].as_u64(), next_pos_arr[1].as_u64()) {
                                                let next_pos = (x as usize, y as usize);
                                                if Some(next_pos) != my_point {
                                                    println!("🚶 Moving: {:?} -> {:?}", my_point.unwrap(), next_pos);
                                                }
                                                my_point = Some(next_pos);
                                                // 移動後、即座に新しい位置をマネージャーに通知
                                                broadcast_position(&mut swarm, &topic, &local_peer_id_str, next_pos);
                                            }
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
                            println!("   Waiting for manager's instructions...");
                            println!("=========================");

                            my_task = Some(task.clone());

                            // タスク受信メトリクス
                            if let Some(task_id) = task.task_id {
                                let now_ms = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_millis() as u64;
                                let metric_msg = serde_json::json!({
                                    "type": "task_metric_received",
                                    "task_id": task_id,
                                    "peer_id": local_peer_id_str,
                                    "timestamp_ms": now_ms
                                }).to_string();
                                let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), metric_msg.as_bytes());

                                // タスク開始メトリクス
                                let metric_msg = serde_json::json!({
                                    "type": "task_metric_started",
                                    "task_id": task_id,
                                    "peer_id": local_peer_id_str,
                                    "timestamp_ms": now_ms
                                }).to_string();
                                let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), metric_msg.as_bytes());
                            }

                            // マネージャーの指示に従って移動するため、ここでは特に何もしない
                            // タスク完了判定は後でmy_pointをチェックして行う
                        }

                        // タスク完了の判定（位置ベース）
                        if let (Some(current_pos), Some(task)) = (my_point, my_task.as_ref()) {
                            if current_pos == task.delivery {
                                println!("🎉 [TASK COMPLETE] Reached delivery point!");

                                if let Some(task_id) = task.task_id {
                                    let now_ms = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap()
                                        .as_millis() as u64;
                                    let metric_msg = serde_json::json!({
                                        "type": "task_metric_completed",
                                        "task_id": task_id,
                                        "peer_id": local_peer_id_str,
                                        "timestamp_ms": now_ms
                                    }).to_string();
                                    let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), metric_msg.as_bytes());

                                    let done_json = serde_json::json!({
                                        "status": "done",
                                        "task_id": task_id
                                    }).to_string();

                                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), done_json.as_bytes()) {
                                        println!("❌ Failed to send completion: {e:?}");
                                    } else {
                                        println!("✅ Task completion notification sent");
                                    }
                                }

                                my_task = None;
                            }
                        }
                    }
                },
                _ => {}
            }
        }
    }
}
