# 近くのエージェントの取得とTSWAPの実装フロー

## 1. 近隣エージェント情報の管理構造

### データ構造（行48-95）
```rust
// エージェント情報を保持
struct AgentInfo {
    peer_id: String,
    current_pos: Point,      // 現在位置
    goal_pos: Point,         // ゴール位置
    timestamp: u64,          // タイムスタンプ
}

// 近隣エージェントを管理
struct NearbyAgents {
    agents: HashMap<String, AgentInfo>,  // PeerID -> AgentInfo
    last_cleanup: std::time::Instant,
}
```

### 主要メソッド
1. **`update()`**: 他のエージェントの情報を更新/追加
2. **`get_nearby(radius)`**: 指定半径内のエージェントを取得（マンハッタン距離）
3. **`cleanup_old()`**: 古い情報を削除（10秒以上更新なし）

---

## 2. 近隣エージェント情報の取得フロー

### ステップ1: 位置情報のブロードキャスト（行492-505）
```rust
// 1秒ごとに自分の位置とゴールを送信
_ = tokio::time::sleep(...) => {
    let pos_json = serde_json::json!({
        "type": "position",
        "peer_id": local_peer_id_str,
        "pos": [p.0, p.1],           // 現在位置
        "goal": [my_goal.0, my_goal.1], // ゴール位置
        "timestamp": timestamp
    });
    swarm.publish(topic, pos_json);
}
```

### ステップ2: 他エージェントの位置情報を受信（行533-558）
```rust
SwarmEvent::Gossipsub(Event::Message { message, .. }) => {
    // "position"タイプのメッセージを処理
    if val.get("type") == "position" {
        // JSONから位置とゴールを抽出
        let current_pos = (px, py);
        let goal_pos = (gx, gy);
        
        // NearbyAgentsに追加/更新 ← ★ここで取得
        nearby_agents.update(AgentInfo {
            peer_id: peer_id_str,
            current_pos,
            goal_pos,
            timestamp,
        });
    }
}
```

### ステップ3: 近隣エージェントのフィルタリング（行73-77）
```rust
fn get_nearby(&self, my_pos: Point, radius: usize) -> Vec<AgentInfo> {
    self.agents.values()
        .filter(|agent| manhattan_distance(my_pos, agent.current_pos) <= radius)
        .cloned()
        .collect()
}
```

---

## 3. TSWAPによる衝突回避の実行

### タスク実行時に近隣エージェントを取得（行728-729）
```rust
while current_pos != pickup {
    // ★ 現在位置から半径5マス以内のエージェントを取得
    let nearby = nearby_agents.get_nearby(current_pos, 5);
    
    println!("[TSWAP] Current: {:?}, Goal: {:?}, Nearby agents: {}", 
             current_pos, my_goal, nearby.len());
    
    // TSWAPロジックで次の移動先を計算
    let next_pos = compute_next_move_with_tswap(
        current_pos,
        my_goal,
        &nearby,  // ← 近隣エージェント情報を渡す
        &grid,
        &pos2id,
        &tswap_nodes,
    );
    
    if next_pos == current_pos {
        println!("[TSWAP] Waiting due to collision avoidance...");
    } else {
        println!("[TSWAP] Moving {:?} -> {:?}", current_pos, next_pos);
        current_pos = next_pos;
    }
}
```

---

## 4. TSWAP衝突回避ロジック（行207-283）

### 処理フロー
```rust
fn compute_next_move_with_tswap(
    my_pos: Point,
    my_goal: Point,
    nearby_agents: &[AgentInfo],  // ← 近隣エージェント情報
    ...
) -> Point {
    // 1. ゴールに到達しているかチェック
    if my_pos == my_goal {
        return my_pos;
    }
    
    // 2. A*で次の移動先を計算
    let path = get_path(my_pos, my_goal, nodes);
    let next_pos = nodes[path[1]].pos;
    
    // 3. 次の位置に他のエージェントがいるかチェック
    if let Some(blocking_agent) = nearby_agents.iter()
        .find(|a| a.current_pos == next_pos) {
        
        // 4a. 相手がゴールにいる → ゴールスワップ
        if blocking_agent.current_pos == blocking_agent.goal_pos {
            println!("[TSWAP] Agent at goal, requesting goal swap");
            return my_pos; // 待機
        }
        
        // 4b. デッドロック検出
        let mut visited = HashSet::new();
        let mut current_agent = blocking_agent;
        
        loop {
            // サイクル検出
            if visited.contains(&current_agent.current_pos) {
                if current_agent.current_pos == my_pos {
                    println!("[TSWAP] Deadlock detected, need target rotation");
                }
                break;
            }
            
            visited.insert(current_agent.current_pos);
            
            // 次の阻害エージェントを探す
            let blocking_next = nearby_agents.iter()
                .find(|a| manhattan_distance(current_agent.current_pos, a.current_pos) <= 1);
            
            if let Some(next) = blocking_next {
                current_agent = next;
            } else {
                break;
            }
        }
        
        // 衝突があるので待機
        return my_pos;
    }
    
    // 5. 移動先が空いていれば移動
    next_pos
}
```

---

## 5. 全体フロー図

```
┌─────────────────────────────────────────────────────────┐
│ Agent 1                                                  │
│ 1秒ごと: position + goal をブロードキャスト              │
└────────────────┬────────────────────────────────────────┘
                 │
                 ▼ (Gossipsub P2P)
┌─────────────────────────────────────────────────────────┐
│ Agent 2 (自分)                                           │
│                                                          │
│ 1. 他エージェントのpositionメッセージを受信              │
│    └→ nearby_agents.update(AgentInfo)                   │
│                                                          │
│ 2. タスク実行時:                                         │
│    nearby = nearby_agents.get_nearby(current_pos, 5)    │
│    └→ 半径5マス以内のエージェント情報を取得              │
│                                                          │
│ 3. TSWAP衝突回避:                                        │
│    next_pos = compute_next_move_with_tswap(              │
│        current_pos, my_goal, &nearby, ...               │
│    )                                                     │
│    ├─ 次の位置に他エージェントがいる？                   │
│    │  ├─ YES: 相手がゴール？→ ゴールスワップ            │
│    │  │       デッドロック？→ ターゲットローテーション   │
│    │  │       それ以外→ 待機                             │
│    │  └─ NO: 移動                                        │
│                                                          │
│ 4. 自分のposition + goalをブロードキャスト               │
└─────────────────────────────────────────────────────────┘
```

---

## 6. 重要ポイント

### 近隣エージェント情報の取得タイミング
1. **常時**: 1秒ごとに全エージェントが位置情報をブロードキャスト
2. **受信時**: Gossipsubメッセージ受信時に`nearby_agents.update()`
3. **使用時**: タスク実行の各ステップで`get_nearby(current_pos, 5)`

### TSWAP判定
- **範囲**: マンハッタン距離で半径5マス以内
- **頻度**: 移動の各ステップ（500ms間隔）
- **判断**: 次の位置に他エージェントがいるか、デッドロックか

### 衝突回避の種類
1. **単純待機**: 次の位置に他エージェントがいる
2. **ゴールスワップ**: 相手がゴールにいて動けない
3. **ターゲットローテーション**: デッドロックサイクルを検出

---

## 7. デバッグログの見方

実行時のログ:
```
[TSWAP] Current: (23, 13), Goal: (9, 16), Nearby agents: 2
```
- 現在位置: (23, 13)
- ゴール: (9, 16)
- 半径5マス以内にいるエージェント数: 2

```
[TSWAP] Agent at goal, requesting goal swap
```
- 次の位置にいるエージェントがゴールにいるため、ゴールスワップを要求

```
[TSWAP] Waiting due to collision avoidance...
```
- 衝突を避けるため、このステップは移動せず待機
