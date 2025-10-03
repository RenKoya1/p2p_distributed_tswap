# 実行手順

## 問題の説明
ログに表示されている「Worker: path to pickup for task id=...」というメッセージは、**古いバージョンのコード**から出力されています。

現在のコードではTSWAPベースの移動ロジックが実装されており、以下のようなログが出力されるはずです：
- `Worker: Moving to pickup at (x, y) using TSWAP`
- `[TSWAP] Current: (x,y), Goal: (gx,gy), Nearby agents: N`
- `[TSWAP] Moving (x1,y1) -> (x2,y2)`

## 解決方法

### 1. クリーンビルド済み
すでに`cargo clean`と`cargo build`を実行しているので、最新のバイナリがビルドされています。

### 2. 新しいバイナリを実行

#### ターミナル1: Manager
```bash
cd "/Users/renkoya/Library/Mobile Documents/com~apple~CloudDocs/CS/Lab/p2p_distributed_tswap"
cargo run --bin manager
```

#### ターミナル2: Agent 1
```bash
cd "/Users/renkoya/Library/Mobile Documents/com~apple~CloudDocs/CS/Lab/p2p_distributed_tswap"
cargo run --bin agent
```

#### ターミナル3: Agent 2
```bash
cd "/Users/renkoya/Library/Mobile Documents/com~apple~CloudDocs/CS/Lab/p2p_distributed_tswap"
cargo run --bin agent
```

### 3. タスクを割り当て
Managerのターミナルで:
```
task
```

## 期待される出力

### Agent側で以下のようなログが表示されます：

```
Received task: Task { pickup: (9, 16), delivery: (1, 9), ... }
Worker: Moving to pickup at (9, 16) using TSWAP
[TSWAP] Current: (23, 13), Goal: (9, 16), Nearby agents: 2
[TSWAP] Moving (23, 13) -> (22, 13)
[TSWAP] Current: (22, 13), Goal: (9, 16), Nearby agents: 2
...
```

もし他のエージェントと衝突しそうな場合：
```
[TSWAP] Waiting due to collision avoidance...
```

## 警告について

以下の警告は実行には影響しません：
```
warning: value assigned to `my_point` is never read
warning: field `id` is never read
```

これらは最適化の余地があることを示していますが、機能には問題ありません。

## トラブルシューティング

### 古いログが表示される場合
1. すべてのターミナルを閉じる
2. 新しいターミナルを開く
3. 上記の手順で実行

### それでも古いログが表示される場合
```bash
# targetディレクトリを完全に削除
rm -rf target/
# 再ビルド
cargo build --bin agent
cargo build --bin manager
# 実行
cargo run --bin agent
```

## TSWAPの動作確認

複数のAgentを起動して、同じエリアにタスクを割り当てると、TSWAPによる衝突回避が確認できます：

1. 3つのエージェントを起動
2. Managerで連続して `task` と入力
3. エージェントのログで `[TSWAP] Waiting due to collision avoidance...` が表示されることを確認
