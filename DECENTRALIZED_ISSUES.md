# 分散版TSWAP実装の問題点と改善案

## 🚨 現在の問題

### 1. **ネットワーク通信のスケーラビリティ問題**

#### 問題の詳細
- **Gossipsubは全エージェントにブロードキャストする**
  - 現在の実装では、各エージェントの位置情報が全エージェントに送信されている
  - `publish(topic, message)` は、同じトピックを購読している全ピアにメッセージを配信
  
- **TSWAP半径（15マス）は計算にのみ使用され、通信には使用されていない**
  ```rust
  // エージェント側：半径15の近隣エージェントのみを計算に使用
  let nearby = nearby_agents.get_nearby(current_pos, 15, &local_peer_id_str);
  
  // しかし、位置情報のブロードキャストは全エージェントに送信される
  swarm.behaviour_mut().gossipsub.publish(topic.clone(), pos_json);  // 全員に届く
  ```

#### スケーラビリティへの影響
- **O(N²)の通信コスト**: N台のエージェントがそれぞれ全エージェントに位置情報を送信
- 100エージェント: 10,000メッセージ/秒（500ms間隔の場合）
- 1000エージェント: 1,000,000メッセージ/秒 ❌ **実用不可能**

---

### 2. **経路計画メトリクスの測定が不正確**

#### 問題の詳細
集中版と分散版で測定している内容が異なる：

**集中版（manager.rs）:**
```rust
// マネージャーが全エージェントの経路を一度に計算
let plan_start = std::time::Instant::now();
let instructions = plan_all_paths(&mut agents, ...);
let elapsed = plan_start.elapsed();
path_metrics.record_duration(elapsed);
```
- 測定内容：**全エージェントの経路計画時間の合計**
- 50エージェントの例：180ms（全体）

**分散版（agent.rs）:**
```rust
// 各エージェントが自分の経路のみ計算
let (action, duration_micros) = compute_next_move_with_tswap(...);
publish_path_metric(&mut swarm, &topic, &local_peer_id_str, duration_micros);
```
- 測定内容：**1エージェントの経路計画時間**
- 分散環境では各エージェントが個別に計算

#### 比較の問題
- **集中版**: 180ms（50エージェント全体）
- **分散版**: ？ms（1エージェント単位）
- **この2つは直接比較できない！**

---

## ✅ 改善案

### 改善1: **距離ベースのトピック分割（地理的パーティショニング）**

```rust
// エージェントの位置に基づいて動的にトピックを変更
fn get_region_topic(pos: Point) -> String {
    let region_size = 30;  // 30x30のグリッド単位
    let region_x = pos.0 / region_size;
    let region_y = pos.1 / region_size;
    format!("mapd_region_{}_{}", region_x, region_y)
}

// 近隣の9つの領域（自分 + 周囲8つ）のトピックを購読
fn subscribe_to_nearby_regions(swarm: &mut Swarm, pos: Point) {
    let region_size = 30;
    let my_region_x = pos.0 / region_size;
    let my_region_y = pos.1 / region_size;
    
    for dx in -1..=1 {
        for dy in -1..=1 {
            let topic_name = format!("mapd_region_{}_{}", 
                                    my_region_x + dx, 
                                    my_region_y + dy);
            let topic = gossipsub::IdentTopic::new(topic_name);
            swarm.behaviour_mut().gossipsub.subscribe(&topic);
        }
    }
}
```

**メリット:**
- 通信コストが **O(N²) → O(N × 局所密度)** に改善
- TSWAP半径（15）と整合性が取れる
- 動的に移動しても、領域を跨ぐときにトピックを変更すれば対応可能

---

### 改善2: **Direct Messaging（P2P通信）**

Gossipsubの代わりに、近隣エージェントとのみ直接通信：

```rust
// 近隣エージェントのPeerIdを保持
let nearby_peer_ids: HashSet<PeerId> = ...;

// 近隣エージェントにのみメッセージを送信
for peer_id in nearby_peer_ids {
    swarm.behaviour_mut()
        .request_response
        .send_request(&peer_id, position_update);
}
```

**メリット:**
- 真の分散型TSWAP
- 通信量が **O(近隣エージェント数)** に削減

---

### 改善3: **メトリクス測定の統一**

集中版と分散版で公平に比較するため、測定方法を統一：

**Option A: 集中版を分散版に合わせる**
```rust
// 集中版でも1エージェントごとの計算時間を記録
for agent in &mut agents {
    let start = std::time::Instant::now();
    plan_single_agent_path(agent, ...);
    let elapsed = start.elapsed();
    path_metrics.record_duration(elapsed);
}
```

**Option B: 分散版を集中版に合わせる**
```rust
// マネージャーが全エージェントの経路計画時間を集計
// （ただし、通信オーバーヘッドが含まれず不公平）
```

**推奨: Option A**
- 1エージェントあたりの平均計算時間を比較
- 集中版: `平均時間 = 合計時間 / エージェント数`
- 分散版: `平均時間 = 各エージェントの計算時間の平均`

---

## 📊 正しい評価方法

### スケーラビリティ評価

1. **計算コスト**
   - 集中版: O(N) - 全エージェントの経路を順次計算
   - 分散版: O(1) - 各エージェントが並列に計算
   
2. **通信コスト**（修正後）
   - 集中版: O(N) - マネージャーが全エージェントに指示
   - 分散版: O(局所密度) - 近隣エージェントとのみ通信
   
3. **測定すべき指標**
   - **1エージェントあたりの平均経路計画時間**
   - **ネットワーク帯域幅使用量** (bytes/sec)
   - **メッセージ数** (messages/sec)
   - **タスク完了率** (tasks/sec)
   - **エージェント数とのスケーラビリティ** (10, 50, 100, 500, 1000)

---

## 🎯 実装の優先順位

### Priority 1: **地理的パーティショニングの実装**
- 最も簡単で効果的
- 現在のGossipsub基盤を維持できる

### Priority 2: **メトリクス測定の統一**
- 公平な比較のため必須

### Priority 3: **Direct Messagingへの移行**
- より正確なTSWAP実装
- 実装コストが高い

---

## 📝 まとめ

現在の分散版実装は：
- ✅ TSWAPアルゴリズム自体は正しく実装されている
- ✅ 半径15の近隣エージェントのみを計算に使用している
- ❌ **ネットワーク通信が全エージェントにブロードキャストされている**
- ❌ **メトリクス測定が集中版と比較できない**

**真のスケーラビリティを実証するには、通信範囲の制限が必要です。**
