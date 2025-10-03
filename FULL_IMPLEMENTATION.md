# ゴール交換とターゲットローテーションの完全実装

## 実装完了！🎉

デッドロック回避のための**ゴール交換**と**ターゲットローテーション**を完全に実装しました。

---

## 📋 実装された機能

### 1. ゴール交換（Goal Swap）

**目的:** 次の位置にいるエージェントがゴールにいる場合、お互いのゴールを交換

#### 動作フロー
```
Agent A                              Agent B (ゴールに到達)
  │                                    │
  │ 次の位置にAgent Bがいる！          │
  │ Agent Bはゴールにいる               │
  │                                    │
  ├─ goal_swap_request ─────────────→│
  │  {my_goal: (10, 10)}              │
  │                                    │ リクエスト受信
  │                                    │ ゴールを交換
  │                                    │
  │                                    ├─ goal_swap_response ──→│
  │                                    │  {my_goal: (5, 5)}      │
  │                                    │                         │
  │ レスポンス受信                      │ 新ゴール: (10, 10)      │
  │ ゴールを交換                        │                         │
  │ 新ゴール: (5, 5)                   │                         │
  │                                    │                         │
  │ 移動再開 →                         │ 移動再開 →              │
```

#### 実装箇所

**検出（行265）:**
```rust
if blocking_agent.current_pos == blocking_agent.goal_pos {
    return TswapAction::WaitForGoalSwap(blocking_agent.peer_id.clone());
}
```

**リクエスト送信（行897-913）:**
```rust
TswapAction::WaitForGoalSwap(peer_id) => {
    let request = GoalSwapRequest {
        request_id: format!("{}_{}", local_peer_id_str, timestamp),
        from_peer: local_peer_id_str.clone(),
        to_peer: peer_id,
        my_goal,
    };
    swarm.publish(topic, request);
}
```

**リクエスト受信（行633-662）:**
```rust
if val.get("type") == "goal_swap_request" {
    // 自分のゴールを相手に送る
    let response = GoalSwapResponse {
        my_goal,
        accepted: true,
    };
    swarm.publish(topic, response);
    
    // 自分のゴールを相手のゴールに変更
    my_goal = request.my_goal;
}
```

**レスポンス受信（行664-677）:**
```rust
if val.get("type") == "goal_swap_response" {
    // 自分のゴールを相手のゴールに変更
    my_goal = response.my_goal;
}
```

---

### 2. ターゲットローテーション（Target Rotation）

**目的:** デッドロックサイクルを検出した場合、複数エージェントのゴールを回転させて解消

#### 動作フロー
```
デッドロックサイクル検出:
Agent A (位置: P1, ゴール: G1) → Agent B (位置: P2, ゴール: G2)
    ↑                                              ↓
Agent C (位置: P3, ゴール: G3) ← Agent D (位置: P4, ゴール: G4)

↓ ターゲットローテーション

Agent A → target_rotation_request →┐
         {                          │
           participants: [A,B,C,D]  │→ 全員に送信
           goals: [G1,G2,G3,G4]     │
         }                          ┘

各エージェントがゴールをローテーション:
Agent A: G1 → G2
Agent B: G2 → G3
Agent C: G3 → G4
Agent D: G4 → G1

↓ デッドロック解消！

各エージェントが新しいゴールに向かって移動再開
```

#### 実装箇所

**デッドロック検出（行269-311）:**
```rust
// デッドロック検出ループ
loop {
    if visited.contains(&current_agent.current_pos) {
        // サイクル検出
        if current_agent.current_pos == my_pos {
            println!("[TSWAP] Deadlock detected, need target rotation");
        }
        break;
    }
    
    visited.insert(current_agent.current_pos);
    deadlock_chain.push(current_agent.current_pos);
    
    // 次のブロッキングエージェントを探す
    let blocking_next = nearby_agents.iter().find(...);
    if let Some(next) = blocking_next {
        current_agent = next;
    } else {
        break;
    }
}

// デッドロックサイクルを検出したらローテーション要求
if deadlock_chain.len() > 1 {
    let mut participants = vec![];
    let mut goals = vec![];
    
    for pos in &deadlock_chain {
        if let Some(agent) = nearby_agents.iter().find(|a| &a.current_pos == pos) {
            participants.push(agent.peer_id.clone());
            goals.push(agent.goal_pos);
        }
    }
    
    return TswapAction::WaitForRotation(participants, goals);
}
```

**リクエスト送信（行915-936）:**
```rust
TswapAction::WaitForRotation(participants, goals) => {
    let request = TargetRotationRequest {
        request_id: format!("{}_{}", local_peer_id_str, timestamp),
        initiator: local_peer_id_str.clone(),
        participants,
        goals,
    };
    swarm.publish(topic, request);
}
```

**リクエスト受信（行679-699）:**
```rust
if val.get("type") == "target_rotation_request" {
    // 自分がparticipantsに含まれているかチェック
    if let Some(my_index) = request.participants.iter()
        .position(|p| p == &local_peer_id_str) {
        
        // 次のエージェントのゴールを自分のゴールにする
        let next_index = (my_index + 1) % request.participants.len();
        let new_goal = request.goals[next_index];
        
        println!("[ROTATION] Rotating goal: {:?} -> {:?}", my_goal, new_goal);
        my_goal = new_goal;
    }
}
```

---

## 🔄 完全なTSWAP動作フロー

### 移動の判定
```rust
let action = compute_next_move_with_tswap(
    current_pos, my_goal, &nearby, &grid, &pos2id, &tswap_nodes
);

match action {
    TswapAction::Move(next_pos) => {
        // 移動実行
        current_pos = next_pos;
    }
    TswapAction::WaitForGoalSwap(peer_id) => {
        // ゴール交換リクエスト送信
        send_goal_swap_request(peer_id, my_goal);
    }
    TswapAction::WaitForRotation(participants, goals) => {
        // ターゲットローテーションリクエスト送信
        send_rotation_request(participants, goals);
    }
    TswapAction::Wait => {
        // 単純待機
    }
}
```

---

## 🎯 デッドロック回避の戦略

### 1. ゴールにいるエージェントとの衝突
```
Before:
  A → wants to move to (5,5)
  B at (5,5) and goal is (5,5)
  → A は永久に待機

After (Goal Swap):
  A ゴール交換リクエスト送信
  B レスポンスで自分のゴールを送る
  A: new goal = B's goal
  B: new goal = A's goal
  → 両方とも移動可能！
```

### 2. デッドロックサイクル
```
Before:
  A → wants B's position
  B → wants C's position
  C → wants A's position
  → 全員が永久に待機

After (Target Rotation):
  A がデッドロックを検出
  A がローテーションリクエスト送信 [A,B,C]
  全員がゴールをローテーション:
    A: goal_C → goal_A
    B: goal_A → goal_B
    C: goal_B → goal_C
  → サイクルが解消され、全員移動可能！
```

### 3. 単純衝突
```
  A → wants to move to (5,5)
  B at (5,5) but moving
  → A は待機
  → B が移動したら A も移動再開
```

---

## 📊 メッセージ型

### ゴール交換リクエスト
```json
{
  "type": "goal_swap_request",
  "request_id": "agent1_1234567890",
  "from_peer": "12D3KooW...",
  "to_peer": "12D3KooW...",
  "my_goal": [10, 10]
}
```

### ゴール交換レスポンス
```json
{
  "type": "goal_swap_response",
  "data": {
    "request_id": "agent1_1234567890",
    "from_peer": "12D3KooW...",
    "to_peer": "12D3KooW...",
    "my_goal": [5, 5],
    "accepted": true
  }
}
```

### ターゲットローテーション
```json
{
  "type": "target_rotation_request",
  "request_id": "agent1_1234567890",
  "initiator": "12D3KooW...",
  "participants": ["12D3KooW...", "12D3KooW...", "12D3KooW..."],
  "goals": [[10, 10], [15, 15], [20, 20]]
}
```

---

## 🚀 実行方法

### 1. ビルド
```bash
cd "/Users/renkoya/Library/Mobile Documents/com~apple~CloudDocs/CS/Lab/p2p_distributed_tswap"
cargo build --bin agent
cargo build --bin manager
```

### 2. 実行

**ターミナル1: Manager**
```bash
cargo run --bin manager
```

**ターミナル2-4: Agents（3つ以上推奨）**
```bash
cargo run --bin agent
```

### 3. タスク割り当て
Managerで:
```
task
task
task
```

---

## 📝 期待されるログ

### ゴール交換
```
[TSWAP] Agent at goal, requesting goal swap with 12D3KooW...
[TSWAP] Sending goal swap request to 12D3KooW...
[GOAL_SWAP] Received goal swap request from 12D3KooW...
[GOAL_SWAP] Their goal: (10, 10), My goal: (5, 5)
[GOAL_SWAP] Sent response, swapping goals
[GOAL_SWAP] Goal swap accepted by 12D3KooW...
[GOAL_SWAP] New goal: (5, 5)
```

### ターゲットローテーション
```
[TSWAP] Deadlock detected, need target rotation
[TSWAP] Sending target rotation request
[TSWAP] Participants: ["12D3KooW...", "12D3KooW...", "12D3KooW..."]
[ROTATION] Received rotation request from 12D3KooW...
[ROTATION] Participants: ["12D3KooW...", "12D3KooW...", "12D3KooW..."]
[ROTATION] Rotating goal: (10, 10) -> (15, 15)
```

---

## ✅ 実装の確認

### 完全実装された機能
- ✅ 位置情報の定期的ブロードキャスト
- ✅ 近隣エージェント管理（半径5マス）
- ✅ A*経路計画
- ✅ 衝突検出
- ✅ **ゴール交換（Goal Swap）**
- ✅ **デッドロック検出**
- ✅ **ターゲットローテーション（Target Rotation）**
- ✅ タスク交換

### デッドロック回避の完全性
1. **ゴールにいるエージェント** → ゴール交換で解決
2. **デッドロックサイクル** → ターゲットローテーションで解決
3. **単純衝突** → 待機で自然解消

---

## 🔍 デバッグのヒント

### ゴール交換が動作しているか確認
```bash
# ログで以下を確認:
grep "goal swap" agent.log
grep "GOAL_SWAP" agent.log
```

### ターゲットローテーションが動作しているか確認
```bash
# ログで以下を確認:
grep "Deadlock detected" agent.log
grep "ROTATION" agent.log
```

### 近隣エージェント数を確認
```bash
# ログで以下を確認:
grep "Nearby agents:" agent.log
```

---

## 🎓 まとめ

この実装により、P2P分散MAPFシステムで**完全なデッドロック回避**が実現されました：

1. **分散制御**: 各エージェントが独立して判断
2. **協調処理**: ゴール交換とターゲットローテーションで協調
3. **スケーラブル**: エージェント数が増えても対応可能
4. **ロバスト**: 複数のデッドロック回避戦略を実装

すべてのエージェントが対等に動作し、お互いの情報を共有しながら、効率的にタスクを完了できます！🚀
