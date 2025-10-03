use futures::stream::StreamExt;
use libp2p::{
    gossipsub, mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use p2p_distributed_tswap::map::map::MAP;
use p2p_distributed_tswap::map::task_generator::{Task, TaskGeneratorAgent};

use std::collections::HashMap;
use std::collections::{HashSet, hash_map::DefaultHasher};
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
    println!("Peer ID: {}", swarm.local_peer_id());

    // 仮のグリッドを作成（実際は適切なグリッドを渡すこと）
    let grid = Arc::new(parse_map());
    let mut task_gen = TaskGeneratorAgent::new(&grid);
    let mut stdin = io::BufReader::new(io::stdin()).lines();

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    println!("Enter messages via STDIN and they will be sent to connected peers using MAPD topic");
    println!("Type 'task' to generate and send a task to agents.");
    println!(
        "⚠️  IMPORTANT: Wait 1-2 seconds after all agents connect before sending tasks (for Gossipsub mesh to form)!"
    );
    println!(
        "💡 TIP: Look for '🔗 Peer XXX subscribed to topic: mapd' messages to confirm mesh is ready!"
    );

    // 管理用変数
    let mut known_peers: HashSet<libp2p::PeerId> = HashSet::new();
    // トピックに購読済みのピア（Gossipsubメッシュに参加済み）
    let mut subscribed_peers: HashSet<libp2p::PeerId> = HashSet::new();
    // 各peerの進行中タスク: peer_id -> Option<Task>
    let mut peer_task_map: HashMap<libp2p::PeerId, Option<Task>> = HashMap::new();
    // タスクIDとpeerの対応: task_id -> peer_id
    let mut task_peer_map: HashMap<u64, libp2p::PeerId> = HashMap::new();
    // タスク生成用カウンタ
    let mut task_counter: u64 = 0;

    loop {
        select! {
            Ok(Some(line)) = stdin.next_line() => {
                if line.trim() == "task" {
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
                                match serde_json::to_vec(&task) {
                                    Ok(task_bytes) => {
                                        match swarm.behaviour_mut().gossipsub.publish(topic.clone(), task_bytes) {
                                            Ok(_) => {
                                                println!("✅ Task sent to {peer_id}: {:?}", task);
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
                                tokio::time::sleep(Duration::from_millis(300)).await;
                            } else {
                                println!("Task generation failed (not enough free cells)");
                            }
                        }
                    }

                    if !assigned {
                        if subscribed_peers.is_empty() {
                            println!("⚠️  No peers have subscribed to the topic yet.");
                            println!("💡 Tip: Wait for '🔗 Peer XXX subscribed to topic: mapd' messages, then try 'task' again.");
                        } else {
                            println!("⚠️  All subscribed peers are busy with tasks.");
                        }
                    }
                } else {
                    if let Err(e) = swarm
                        .behaviour_mut().gossipsub
                        .publish(topic.clone(), line.as_bytes()) {
                        println!("Publish error: {e:?}");
                    }
                }
            }
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(MapdBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discovered a new peer: {peer_id}");
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        known_peers.insert(peer_id.clone());
                        peer_task_map.entry(peer_id.clone()).or_insert(None);
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
                    // 完了通知をJSONで受信し、status=="done"かつtask_idが存在する場合のみタスクを再配布
                    if let Ok(done_msg) = serde_json::from_str::<serde_json::Value>(&msg_str) {
                        if done_msg.get("status") == Some(&serde_json::Value::String("done".to_string())) {
                            let task_id = done_msg.get("task_id").and_then(|v| v.as_u64());
                            println!("タスク完了通知を受信: {peer_id}, task_id: {:?}", task_id);
                            peer_task_map.insert(peer_id.clone(), None);
                            // 新しいタスクを生成して配布
                            if let Some(mut task) = task_gen.generate_task() {
                                task_counter += 1;
                                let new_task_id = task_counter;
                                task.peer_id = Some(peer_id.to_base58());
                                task.task_id = Some(new_task_id);
                                match serde_json::to_vec(&task) {
                                    Ok(task_bytes) => {
                                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), task_bytes) {
                                            println!("Task publish error: {e:?}");
                                        } else {
                                            println!("Task sent to {peer_id}: {:?}", task);
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
