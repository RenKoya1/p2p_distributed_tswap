# TSWAPベースの衝突回避実装

## 概要
このシステムは、P2P分散環境でTSWAP（Token Swapping）アルゴリズムを使用して、複数のエージェント間の衝突を回避します。

## 主な機能

### 1. 近距離エージェント情報の共有
- **位置情報ブロードキャスト**: 各エージェントは1秒ごとに自身の現在位置とゴール位置をブロードキャスト
- **近隣エージェント管理**: `NearbyAgents`構造体で他のエージェントの情報を管理
- **範囲フィルタリング**: 指定された半径内（デフォルト5マス）のエージェントのみを考慮

### 2. TSWAPによる衝突回避

#### 基本動作
```rust
fn compute_next_move_with_tswap(
    my_pos: Point,
    my_goal: Point,
    nearby_agents: &[AgentInfo],
    ...
) -> Point
```

**アルゴリズムの流れ:**

1. **ゴール到達チェック**: 既にゴールにいる場合は移動しない
2. **次の移動先計算**: A*アルゴリズムで最短経路の次のノードを取得
3. **衝突検出**: 次の位置に他のエージェントがいるかチェック
4. **TSWAP判定**:
   - **ゴールスワップ**: 相手がゴールにいる場合、ゴール位置を交換（メッセージベース）
   - **デッドロック検出**: 循環待機状態を検出し、ターゲットローテーションで解決
5. **移動実行**: 安全な場合のみ移動、そうでなければ待機

### 3. メッセージフォーマット

#### 位置情報ブロードキャスト
```json
{
  "type": "position",
  "peer_id": "12D3KooW...",
  "pos": [x, y],
  "goal": [gx, gy],
  "timestamp": 1234567890
}
```

#### ゴールスワップリクエスト（将来実装）
```json
{
  "type": "goal_swap",
  "from_peer": "12D3KooW...",
  "to_peer": "12D3KooW...",
  "my_goal": [x, y]
}
```

### 4. タスク実行フロー

```
1. Managerからタスクを受信
   ↓
2. pickup地点をmy_goalに設定
   ↓
3. TSWAPベースで1ステップずつ移動
   - 近隣エージェント情報を取得
   - compute_next_move_with_tswap()で次の位置を計算
   - 衝突があれば待機、なければ移動
   ↓
4. pickup到達
   ↓
5. delivery地点をmy_goalに設定
   ↓
6. TSWAPベースで移動（手順3と同様）
   ↓
7. delivery到達 → 完了通知
```

## 実行方法

### 1. Managerを起動
```bash
cargo run --bin manager
```

### 2. 複数のAgentを起動（別々のターミナル）
```bash
# Agent 1
cargo run --bin agent

# Agent 2
cargo run --bin agent

# Agent 3
cargo run --bin agent
```

### 3. タスクを割り当て
Managerのターミナルで:
```
task
```

## TSWAPの利点

1. **分散制御**: 中央サーバーなしで各エージェントが自律的に衝突回避
2. **デッドロック回避**: 循環待機状態を検出し、ゴールスワップで解決
3. **効率的**: 不要な待機を最小限に抑える
4. **スケーラブル**: エージェント数が増えても対応可能

## デバッグ情報

実行時のログ:
- `[TSWAP] Current: (x,y), Goal: (gx,gy), Nearby agents: N` - 現在の状態
- `[TSWAP] Moving (x1,y1) -> (x2,y2)` - 移動実行
- `[TSWAP] Waiting due to collision avoidance...` - 衝突回避のため待機
- `[TSWAP] Agent at goal, requesting goal swap` - ゴールスワップ要求
- `[TSWAP] Deadlock detected, need target rotation` - デッドロック検出

## 今後の改善点

1. **ゴールスワップの完全実装**: 現在はメッセージ送信まで（実際の交換処理を追加）
2. **ターゲットローテーション**: デッドロック時の協調的なゴール再配置
3. **優先度システム**: タスクの緊急度に基づいた優先順位付け
4. **動的範囲調整**: 混雑状況に応じた近隣エージェント範囲の調整
