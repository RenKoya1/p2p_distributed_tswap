use futures::StreamExt;
use libp2p::{
    PeerId, gossipsub, identity, mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    hash::{Hash, Hasher},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{self, AsyncBufReadExt},
    select,
};

use bincode::serde::{decode_from_slice, encode_to_vec};

#[derive(Debug, Serialize, Deserialize)]
struct Post {
    username: String,
    content: String,
    timestamp: u64,
}

#[derive(NetworkBehaviour)]
struct MyBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let username = whoami::username();
    let key = identity::Keypair::generate_ed25519();
    let peer_id = PeerId::from(key.public());
    println!("Local peer id: {peer_id}");

    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|key| {
            let message_id_fn = |message: &gossipsub::Message| {
                let mut hasher = DefaultHasher::new();
                message.data.hash(&mut hasher);
                gossipsub::MessageId::from(hasher.finish().to_string())
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
            Ok(MyBehaviour { gossipsub, mdns })
        })?
        .build();

    let topic = gossipsub::IdentTopic::new("sns");
    swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

    let mut stdin = io::BufReader::new(io::stdin()).lines();

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    println!("Type /post [message] to broadcast");

    loop {
        select! {
            Ok(Some(line)) = stdin.next_line() => {
                if let Some(msg) = line.strip_prefix("/post ") {
                    let post = Post {
                        username: username.clone(),
                        content: msg.to_string(),
                        timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    };
                    let data: Vec<u8> = encode_to_vec(&post, bincode::config::standard())?;
                    swarm.behaviour_mut().gossipsub.publish(topic.clone(), data)?;
                }
            }
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discovered a new peer: {peer_id}");
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discover peer has expired: {peer_id}");
                        swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                    if let Ok((post, _)) = decode_from_slice::<Post, _>(&message.data, bincode::config::standard()) {
                        println!("[{}] {}: {}", post.timestamp, post.username, post.content);
                    }
                },
                SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, .. })) => {
                    println!("Subscribed peer: {peer_id}");
                },
                SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(gossipsub::Event::Unsubscribed { peer_id, .. })) => {
                    println!("Unsubscribed peer: {peer_id}");
                },
                SwarmEvent::Behaviour(_) => {},
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Listening on {address}");
                },
                _ => {}
            }
        }
    }
}
