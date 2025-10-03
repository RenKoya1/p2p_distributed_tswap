# エージェント間の相互認識と交換処理の実装

## 1. すべてのエージェントが近くのエージェントを見る仕組み

### ✅ はい、すべてのエージェントが同じように動作します

各エージェントは**対等な分散ノード**として動作し、以下のサイクルを実行します：

```
┌─────────────────────────────────────────────────────┐
│ Agent 1                   Agent 2        Agent 3    │
│   ↓ブロードキャスト         ↓              ↓        │
│   ┌──────────────────────────────────────┐          │
│   │   Gossipsub P2P Network              │          │
│   │  (全エージェントが情報を共有)          │          │
│   └──────────────────────────────────────┘          │
│   ↓受信                   ↓受信          ↓受信      │
│   ・Agent2の位置を保存     ・Agent1の位置   ・Agent1の位置│
│   ・Agent3の位置を保存     ・Agent3の位置   ・Agent2の位置│
└─────────────────────────────────────────────────────┘
```

### 各エージェントの動作（行492-558）

**1秒ごとに自分の情報をブロードキャスト:**
```rust
// 行492-505: すべてのエージェントが実行
_ = tokio::time::sleep(...) => {
    let pos_json = serde_json::json!({
        "type": "position",
        "peer_id": local_peer_id_str,    // 自分のID
        "pos": [p.0, p.1],               // 自分の現在位置
        "goal": [my_goal.0, my_goal.1],  // 自分のゴール位置
        "timestamp": timestamp
    });
    swarm.publish(topic, pos_json);  // ← 全員に送信
}
```

**他のエージェントの情報を受信して保存:**
```rust
// 行533-558: すべてのエージェントが実行
SwarmEvent::Gossipsub(Event::Message { message, .. }) => {
    if val.get("type") == "position" {
        // 他のエージェントの情報を抽出
        let current_pos = (px, py);
        let goal_pos = (gx, gy);
        
        // ★ 自分のNearbyAgentsに保存
        nearby_agents.update(AgentInfo {
            peer_id: peer_id_str,
            current_pos,
            goal_pos,
            timestamp,
        });
    }
}
```

### 近隣エージェントのフィルタリング（行73-77）

各エージェントは移動の度に**自分の周囲5マス以内**のエージェントだけを取得：

```rust
fn get_nearby(&self, my_pos: Point, radius: usize) -> Vec<AgentInfo> {
    self.agents.values()
        .filter(|agent| 
            manhattan_distance(my_pos, agent.current_pos) <= radius
        )
        .cloned()
        .collect()
}
```

**例:**
```
Agent 1の位置: (10, 10)
半径: 5マス

Agent 2: (12, 12) → 距離 = |10-12| + |10-12| = 4 ✅ 範囲内
Agent 3: (20, 20) → 距離 = |10-20| + |10-20| = 20 ❌ 範囲外
Agent 4: (11, 14) → 距離 = |10-11| + |10-14| = 5 ✅ 範囲内

Agent 1が見る近隣エージェント = [Agent 2, Agent 4]
```

---

## 2. 交換処理の実装箇所

現在の実装には**3種類の交換処理**があります：

### 📌 タイプ1: タスク交換（Task Swap）

**目的:** 目的地に他のエージェントがいる場合、タスクを交換する

#### 送信側（行698-717）
```rust
// タスク受信時に、pickup/deliveryに他のエージェントがいるかチェック
for (peer, pos) in &peer_positions {
    if Some(*pos) == pickup || Some(*pos) == delivery {
        // ★ タスク交換リクエストを送信
        let swap_req = serde_json::json!({
            "type": "swap_request",
            "from_peer": local_peer_id_str,
            "to_peer": peer,
            "task": task  // 自分のタスクを送る
        }).to_string();
        swarm.publish(topic, swap_req.as_bytes());
        
        println!("Sent swap_request to {}", peer);
        swap_sent = true;
        break;
    }
}
```

#### 受信側（行598-623）
```rust
// swap_requestを受信
if val.get("type") == "swap_request" {
    if let (Some(from_peer), Some(task_val)) = 
        (val.get("from_peer"), val.get("task")) {
        
        println!("[SWAP] swap request from {}", from_peer_str);
        
        // ★ 自分のタスクを相手に送り返す
        let swap_response = serde_json::json!({
            "type": "swap_response",
            "from_peer": local_peer_id_str,
            "to_peer": from_peer_str,
            "task": my_task_val  // 自分のタスク
        });
        swarm.publish(topic, swap_response.as_bytes());
        
        // ★ 受信したタスクに切り替え
        if let Ok(new_task) = serde_json::from_value(task_val) {
            my_task = Some(new_task);
        }
    }
}
```

#### レスポンス受信側（行629-673）
```rust
// swap_responseを受信
if val.get("type") == "swap_response" {
    if let Some(task_val) = val.get("task") {
        println!("[SWAP] Received swapped task");
        
        // ★ 交換されたタスクを実行
        my_task = Some(new_task.clone());
        
        // TSWAPベースで新しいタスクを実行
        my_goal = pickup;
        while current_pos != pickup {
            let nearby = nearby_agents.get_nearby(current_pos, 5);
            let next_pos = compute_next_move_with_tswap(...);
            ...
        }
    }
}
```

---

### 📌 タイプ2: ゴール交換（Goal Swap）- ⚠️ 部分実装

**目的:** 次の位置にいるエージェントがゴールにいる場合、お互いのゴールを交換

#### 実装箇所（行231-236）
```rust
// 次の位置に他のエージェントがいるかチェック
if let Some(blocking_agent) = nearby_agents.iter()
    .find(|a| a.current_pos == next_pos) {
    
    // 相手がゴールにいる場合
    if blocking_agent.current_pos == blocking_agent.goal_pos {
        println!("[TSWAP] Agent at goal, requesting goal swap");
        // ⚠️ TODO: ゴール交換はメッセージングで行う（未実装）
        return my_pos; // 今回は移動せず待機
    }
}
```

**現状:** 
- ✅ ゴールにいるエージェントを検出
- ❌ 実際のゴール交換メッセージング処理は未実装
- 現在は待機するのみ

**完全実装の場合:**
```rust
// 提案: ゴール交換メッセージ
let goal_swap_req = serde_json::json!({
    "type": "goal_swap_request",
    "from_peer": local_peer_id_str,
    "to_peer": blocking_agent.peer_id,
    "my_goal": my_goal
});
```

---

### 📌 タイプ3: ターゲットローテーション（Target Rotation）- ⚠️ 部分実装

**目的:** デッドロックサイクルを検出した場合、複数エージェントのゴールを回転

#### 実装箇所（行238-276）
```rust
// デッドロック検出
let mut visited = HashSet::new();
let mut current_agent = blocking_agent;
let mut deadlock_chain = vec![my_pos];

loop {
    if visited.contains(&current_agent.current_pos) {
        // デッドロックサイクル検出
        if current_agent.current_pos == my_pos {
            println!("[TSWAP] Deadlock detected, need target rotation");
            // ⚠️ TODO: ターゲットローテーションはメッセージングで調整（未実装）
        }
        break;
    }
    
    visited.insert(current_agent.current_pos);
    deadlock_chain.push(current_agent.current_pos);
    
    // 次のブロッキングエージェントを探す
    let blocking_next = nearby_agents.iter().find(|a| {
        manhattan_distance(current_agent.current_pos, a.current_pos) <= 1
    });
    
    if let Some(next) = blocking_next {
        current_agent = next;
    } else {
        break;
    }
}

// 移動できない場合は待機
return my_pos;
```

**現状:**
- ✅ デッドロックサイクルを検出
- ✅ サイクルに関与するエージェントを特定
- ❌ 実際のターゲットローテーション協調処理は未実装
- 現在は待機するのみ

---

## 3. 交換処理フロー図

### タスク交換（Task Swap）の完全フロー

```
Agent 1                         Agent 2
  │                               │
  │ タスク受信: pickup=(5,5)      │ 現在位置: (5,5)
  │ Agent2が(5,5)にいる！          │ タスク実行中
  │                               │
  ├─ swap_request ──────────────→│
  │  {task: Task1}                │
  │                               │ swap_request受信
  │                               │
  │                               ├─ swap_response ───→│
  │                               │  {task: Task2}      │
  │                               │                     │
  │                               │ ★Task1に切り替え    │
  │ swap_response受信             │                     │
  │                               │                     │
  │ ★Task2に切り替え              │                     │
  │                               │                     │
  │ Task2を実行開始                │ Task1を実行開始      │
  ↓                               ↓
```

### TSWAP衝突回避の判断フロー

```
移動しようとする
  ↓
次の位置を計算 (A*)
  ↓
次の位置に他のエージェントがいる？
  ├─ NO → ✅ 移動
  │
  └─ YES → 相手の状態をチェック
        ↓
        相手がゴールにいる？
        ├─ YES → 🔄 ゴール交換要求（未実装）
        │         現在は待機
        │
        └─ NO → デッドロックサイクル？
              ├─ YES → 🔄 ターゲットローテーション（未実装）
              │         現在は待機
              │
              └─ NO → ⏸️ 単純待機
```

---

## 4. 実装の要約

### ✅ 完全実装済み
1. **位置情報共有**: すべてのエージェントが1秒ごとにブロードキャスト
2. **近隣エージェント管理**: 半径5マス以内のエージェントを認識
3. **タスク交換**: pickup/deliveryに他のエージェントがいる場合、タスクを交換
4. **衝突検出**: 次の位置に他のエージェントがいることを検出
5. **デッドロック検出**: サイクルを形成しているエージェントを検出

### ⚠️ 部分実装（検出のみ、交換未実装）
1. **ゴール交換**: 検出済みだが、メッセージ交換処理なし
2. **ターゲットローテーション**: 検出済みだが、協調処理なし

### 現在の動作
- 衝突を検出した場合 → **待機**
- 相手が移動したら → **移動再開**
- シンプルだが、デッドロックで永久待機の可能性あり

---

## 5. すべてのエージェントが相互に見る証拠

### コードの対称性

**Agent A:**
```rust
// 自分の位置をブロードキャスト
nearby_agents.update(AgentInfo { ... });

// 近隣エージェントを取得
let nearby = nearby_agents.get_nearby(my_pos, 5);
```

**Agent B:**
```rust
// ★ 同じコード
nearby_agents.update(AgentInfo { ... });
let nearby = nearby_agents.get_nearby(my_pos, 5);
```

**Agent C:**
```rust
// ★ 同じコード
nearby_agents.update(AgentInfo { ... });
let nearby = nearby_agents.get_nearby(my_pos, 5);
```

### 具体例

```
時刻 t=0:
Agent 1 位置: (10, 10), Goal: (15, 15)
Agent 2 位置: (12, 12), Goal: (5, 5)
Agent 3 位置: (20, 20), Goal: (1, 1)

各エージェントのNearbyAgents:
- Agent 1: {Agent 2} (距離4)
- Agent 2: {Agent 1} (距離4)
- Agent 3: {} (他のエージェントが遠い)

★ Agent 1とAgent 2はお互いを認識している
★ すべてのエージェントが同じロジックで動作している
```

---

## 結論

✅ **すべてのエージェントが近くのエージェントを見ています**
- P2Pネットワークで位置情報を共有
- 各エージェントが独立して近隣エージェントを管理
- 対等な分散システムとして動作

✅ **交換処理の実装箇所**
- **タスク交換**: 完全実装（行598-673, 698-717）
- **ゴール交換**: 検出のみ（行231-236）⚠️
- **ターゲットローテーション**: 検出のみ（行238-276）⚠️

現在は**タスク交換のみが完全に動作**し、他の交換処理は検出後に待機する動作になっています。
