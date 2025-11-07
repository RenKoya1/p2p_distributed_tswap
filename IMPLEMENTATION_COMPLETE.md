# Task Metrics & Performance Measurement System

タスク計測・パフォーマンス測定システムの実装が完了しました。

## 機能概要

このシステムは、分散TSWAP実装でのタスク処理時間を自動計測し、詳細な統計情報を提供します。

### 主な機能

1. **自動計測**: タスク送信、受取、処理開始、完了時刻を自動記録
2. **統計分析**: 平均処理時間、遅延、成功率などを計算
3. **CSV出力**: 詳細なデータを分析用CSV形式で保存
4. **大量タスク対応**: 複数エージェントへの効率的な分割送信

## クイックスタート

### 最も簡単な方法

```bash
./test_final.sh 10 100
```

- 10個のエージェント起動
- 100個のタスク送信
- 結果を `results/metrics_*.csv` に保存

### カスタマイズ

```bash
./test_final.sh <num_agents> <num_tasks>

# 例
./test_final.sh 5 50       # 5エージェント, 50タスク
./test_final.sh 50 1000    # 50エージェント, 1000タスク
```

## Manager コマンド（手動実行時）

Managerを起動後、以下のコマンドが使用できます：

```bash
cargo run --bin manager

# コマンド入力:
tasks 100      # 100個のタスクを送信
metrics        # 統計情報を表示
save test.csv  # 結果をCSVに保存
```

## ファイル構成

### 新規追加ファイル

- `src/map/task_metrics.rs` - タスク計測構造体とLogic
- `test_final.sh` - 自動テストスクリプト（推奨）
- `test_metrics.sh` - 詳細版テストスクリプト
- `analyze_metrics.py` - Python分析ツール
- `METRICS_GUIDE.md` - 詳細ガイド

### 更新ファイル

- `src/bin/manager.rs` - タスク分割・計測機能追加
- `src/bin/agent.rs` - 計測メトリクス送信機能追加
- `src/map/mod.rs` - task_metricsモジュール追加

## 計測項目

生成されるCSVには以下の情報が含まれます：

| カラム | 単位 | 説明 |
|--------|------|------|
| task_id | - | タスクID |
| peer_id | - | 処理したエージェントID |
| sent_time_ms | ms | Manager送信時刻 |
| received_time_ms | ms | Agent受取時刻 |
| start_time_ms | ms | 処理開始時刻 |
| completion_time_ms | ms | 完了時刻 |
| total_time_ms | ms | 送信〜完了時間 |
| processing_time_ms | ms | 処理実行時間 |
| startup_latency_ms | ms | 送信〜開始の遅延 |
| status | - | タスク状態 |

## 統計情報

スクリプト実行後、以下のような統計が表示されます：

```
📊 Task Statistics:
├─ Total Tasks: 100
├─ Completed: 98 (Success Rate: 98.0%)
├─ Failed: 2
├─ Avg Total Time: 45230 ms
├─ Avg Processing Time: 42100 ms
├─ Avg Startup Latency: 3130 ms
├─ Min/Max Total Time: 15300 ms / 87500 ms
└─ Min/Max Processing Time: 8900 ms / 75200 ms
```

## 使用例

### 例1: 基本的なテスト

```bash
$ ./test_final.sh 5 50

🚀 Starting Task Metrics Test
   Agents: 5, Tasks: 50
...
✅ Done!

📊 Results saved: /path/to/results/metrics_20251107_091022.csv
   Records: 50
```

### 例2: 大規模テスト

```bash
$ ./test_final.sh 50 500

# 約5分で完了
# 500個のタスク処理結果が保存される
```

### 例3: 手動操作

```bash
# Terminal 1
cargo run --bin manager

# Terminal 2, 3, ...
cargo run --bin agent

# Terminal 1で入力:
tasks 100
metrics
save mytest.csv
```

## トラブルシューティング

### CSVが生成されない

1. Manager のログを確認：
   ```bash
   tail -50 results/manager.log
   ```

2. 計測コマンドが実行されたか確認：
   ```bash
   grep "save\|metrics" results/manager.log
   ```

3. ファイルの書き込み権限を確認：
   ```bash
   ls -la results/
   ```

### タスクが処理されない

1. エージェントの接続を確認：
   ```bash
   grep "subscribed to topic" results/manager.log | wc -l
   ```

2. Gossipsubメッシュが形成されているか確認：
   ```bash
   grep "🔗 Peer" results/manager.log
   ```

## パフォーマンス最適化

### エージェント数の選択

- **少数（3-10個）**: デバッグ・テスト用
- **中程度（20-50個）**: バランス型
- **大規模（100+個）**: スケーラビリティテスト

### タスク数の推奨値

各エージェントあたり 50-100タスク：
- 5エージェント → 250-500タスク
- 10エージェント → 500-1000タスク
- 50エージェント → 2500-5000タスク

## 実装の詳細

### タスク計測フロー

1. **Manager**: タスク生成・送信時に`TaskMetric`を作成
2. **Agent**: タスク受取・開始・完了時に計測情報をManager に送信
3. **Manager**: 計測情報を受信して更新
4. **CSV出力**: 最終的に全計測データをCSV形式で出力

### 時間計測の精度

すべての時刻は Unix timestamp (ミリ秒単位) で記録されます：
- `SystemTime::now().duration_since(UNIX_EPOCH).as_millis() as u64`

## 今後の拡張

- [ ] グラフ生成機能（matplotlib）
- [ ] リアルタイムダッシュボード（Web UI）
- [ ] 分散トレーシング統合
- [ ] パフォーマンスプロファイリング

## 参考資料

- `METRICS_GUIDE.md` - 詳細ガイド
- `src/map/task_metrics.rs` - 実装詳細
- `analyze_metrics.py` - 分析スクリプト

## まとめ

このシステムにより、以下が可能になります：

✅ 大量のタスク処理時間を自動計測  
✅ マルチエージェント環境でのパフォーマンス分析  
✅ CSV形式での詳細データ出力  
✅ 統計情報の自動生成  
✅ スケーラビリティテスト

使用例：

```bash
# シンプルなテスト
./test_final.sh 10 100

# 大規模テスト
./test_final.sh 50 1000

# 手動操作
cargo run --bin manager
# (別のターミナル)
cargo run --bin agent × 複数
# (manager側で)
tasks 500
metrics
save results.csv
```

---

実装完了日: 2025/11/07
