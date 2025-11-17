use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// タスク計測情報
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskMetric {
    pub task_id: u64,
    pub peer_id: String,
    pub sent_time: u64, // managerがタスクを送信した時刻（Unix timestamp, ms）
    pub received_time: Option<u64>, // agentがタスクを受け取った時刻
    pub start_time: Option<u64>, // agentがタスク処理を開始した時刻
    pub completion_time: Option<u64>, // agentがタスク完了した時刻
    pub status: TaskStatus, // タスクの状態
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,   // 送信待ち
    Sent,      // 送信済み
    Received,  // 受け取り済み
    Running,   // 処理中
    Completed, // 完了
    Failed,    // 失敗
}

impl TaskMetric {
    pub fn new(task_id: u64, peer_id: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        TaskMetric {
            task_id,
            peer_id,
            sent_time: now,
            received_time: None,
            start_time: None,
            completion_time: None,
            status: TaskStatus::Sent,
        }
    }

    /// タスク完了時の処理時間（ms）を取得
    pub fn get_total_time(&self) -> Option<u64> {
        self.completion_time.map(|ct| ct - self.sent_time)
    }

    /// エージェント側での処理時間（ms）を取得
    pub fn get_agent_processing_time(&self) -> Option<u64> {
        match (self.start_time, self.completion_time) {
            (Some(start), Some(end)) => Some(end - start),
            _ => None,
        }
    }

    /// managerから送信から処理開始までの遅延（ms）を取得
    pub fn get_startup_latency(&self) -> Option<u64> {
        self.start_time.map(|st| st - self.sent_time)
    }
}

/// タスク計測マネージャー
pub struct TaskMetricsCollector {
    pub metrics: HashMap<u64, TaskMetric>,
}

impl TaskMetricsCollector {
    pub fn new() -> Self {
        TaskMetricsCollector {
            metrics: HashMap::new(),
        }
    }

    pub fn add_metric(&mut self, metric: TaskMetric) {
        self.metrics.insert(metric.task_id, metric);
    }

    pub fn update_received(&mut self, task_id: u64) {
        if let Some(metric) = self.metrics.get_mut(&task_id) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            metric.received_time = Some(now);
            metric.status = TaskStatus::Received;
        }
    }

    pub fn update_started(&mut self, task_id: u64) {
        if let Some(metric) = self.metrics.get_mut(&task_id) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            metric.start_time = Some(now);
            metric.status = TaskStatus::Running;
        }
    }

    pub fn update_completed(&mut self, task_id: u64) {
        if let Some(metric) = self.metrics.get_mut(&task_id) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            metric.completion_time = Some(now);
            metric.status = TaskStatus::Completed;
        }
    }

    pub fn update_failed(&mut self, task_id: u64) {
        if let Some(metric) = self.metrics.get_mut(&task_id) {
            metric.status = TaskStatus::Failed;
        }
    }

    /// 統計情報を取得
    pub fn get_statistics(&self) -> TaskStatistics {
        let completed: Vec<&TaskMetric> = self
            .metrics
            .values()
            .filter(|m| m.status == TaskStatus::Completed)
            .collect();

        let total_times: Vec<u64> = completed
            .iter()
            .filter_map(|m| m.get_total_time())
            .collect();

        let processing_times: Vec<u64> = completed
            .iter()
            .filter_map(|m| m.get_agent_processing_time())
            .collect();

        let startup_latencies: Vec<u64> = completed
            .iter()
            .filter_map(|m| m.get_startup_latency())
            .collect();

        let avg_total_time = if !total_times.is_empty() {
            total_times.iter().sum::<u64>() / total_times.len() as u64
        } else {
            0
        };

        let avg_processing_time = if !processing_times.is_empty() {
            processing_times.iter().sum::<u64>() / processing_times.len() as u64
        } else {
            0
        };

        let avg_startup_latency = if !startup_latencies.is_empty() {
            startup_latencies.iter().sum::<u64>() / startup_latencies.len() as u64
        } else {
            0
        };

        TaskStatistics {
            total_tasks: self.metrics.len(),
            completed_tasks: completed.len(),
            failed_tasks: self
                .metrics
                .values()
                .filter(|m| m.status == TaskStatus::Failed)
                .count(),
            avg_total_time,
            avg_processing_time,
            avg_startup_latency,
            min_total_time: total_times.iter().cloned().min().unwrap_or(0),
            max_total_time: total_times.iter().cloned().max().unwrap_or(0),
            min_processing_time: processing_times.iter().cloned().min().unwrap_or(0),
            max_processing_time: processing_times.iter().cloned().max().unwrap_or(0),
        }
    }

    /// CSV形式で出力（ヘッダー付き）
    pub fn to_csv_string(&self) -> String {
        let mut csv = String::from(
            "task_id,peer_id,sent_time_ms,received_time_ms,start_time_ms,completion_time_ms,total_time_ms,processing_time_ms,startup_latency_ms,status\n",
        );

        let mut tasks: Vec<_> = self.metrics.values().collect();
        tasks.sort_by_key(|m| m.task_id);

        for metric in tasks {
            let total_time = metric
                .get_total_time()
                .map(|t| t.to_string())
                .unwrap_or_default();
            let processing_time = metric
                .get_agent_processing_time()
                .map(|t| t.to_string())
                .unwrap_or_default();
            let startup_latency = metric
                .get_startup_latency()
                .map(|t| t.to_string())
                .unwrap_or_default();

            let status_str = match metric.status {
                TaskStatus::Pending => "pending",
                TaskStatus::Sent => "sent",
                TaskStatus::Received => "received",
                TaskStatus::Running => "running",
                TaskStatus::Completed => "completed",
                TaskStatus::Failed => "failed",
            };

            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{}\n",
                metric.task_id,
                metric.peer_id,
                metric.sent_time,
                metric.received_time.unwrap_or(0),
                metric.start_time.unwrap_or(0),
                metric.completion_time.unwrap_or(0),
                total_time,
                processing_time,
                startup_latency,
                status_str
            ));
        }

        csv
    }
}

/// タスク統計情報
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatistics {
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub avg_total_time: u64,
    pub avg_processing_time: u64,
    pub avg_startup_latency: u64,
    pub min_total_time: u64,
    pub max_total_time: u64,
    pub min_processing_time: u64,
    pub max_processing_time: u64,
}

impl std::fmt::Display for TaskStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "📊 Task Statistics:\n\
            ├─ Total Tasks: {}\n\
            ├─ Completed: {} (Success Rate: {:.1}%)\n\
            ├─ Failed: {}\n\
            ├─ Avg Total Time: {} ms\n\
            ├─ Avg Processing Time: {} ms\n\
            ├─ Avg Startup Latency: {} ms\n\
            ├─ Min/Max Total Time: {} ms / {} ms\n\
            └─ Min/Max Processing Time: {} ms / {} ms",
            self.total_tasks,
            self.completed_tasks,
            if self.total_tasks > 0 {
                (self.completed_tasks as f64 / self.total_tasks as f64) * 100.0
            } else {
                0.0
            },
            self.failed_tasks,
            self.avg_total_time,
            self.avg_processing_time,
            self.avg_startup_latency,
            self.min_total_time,
            self.max_total_time,
            self.min_processing_time,
            self.max_processing_time,
        )
    }
}

/// Path computation metrics shared between centralized and decentralized managers.
#[derive(Default, Debug, Clone)]
pub struct PathComputationMetrics {
    samples: Vec<u128>, // microseconds
}

impl PathComputationMetrics {
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Record a new duration sample using a [`Duration`].
    pub fn record_duration(&mut self, duration: Duration) {
        self.samples.push(duration.as_micros());
    }

    /// Record a new sample directly in microseconds.
    pub fn record_micros(&mut self, micros: u128) {
        self.samples.push(micros);
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn get_statistics(&self) -> Option<PathComputationStatistics> {
        if self.samples.is_empty() {
            return None;
        }

        let mut sorted = self.samples.clone();
        sorted.sort_unstable();

        let sum: u128 = sorted.iter().sum();
        let avg = sum as f64 / sorted.len() as f64;
        let min = *sorted.first().unwrap();
        let max = *sorted.last().unwrap();

        Some(PathComputationStatistics {
            samples: sorted.len(),
            avg_micros: avg,
            min_micros: min,
            max_micros: max,
        })
    }

    pub fn to_csv_string(&self) -> String {
        let mut csv = String::from("sample_index,duration_micros,duration_millis\n");
        for (idx, &sample) in self.samples.iter().enumerate() {
            let millis = sample as f64 / 1000.0;
            csv.push_str(&format!("{idx},{sample},{millis:.3}\n"));
        }
        csv
    }
}

#[derive(Debug, Clone)]
pub struct PathComputationStatistics {
    pub samples: usize,
    pub avg_micros: f64,
    pub min_micros: u128,
    pub max_micros: u128,
}

impl PathComputationStatistics {
    pub fn avg_millis(&self) -> f64 {
        self.avg_micros / 1000.0
    }

    pub fn min_millis(&self) -> f64 {
        self.min_micros as f64 / 1000.0
    }

    pub fn max_millis(&self) -> f64 {
        self.max_micros as f64 / 1000.0
    }
}

impl std::fmt::Display for PathComputationStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "⏱️ Path Computation Stats:\n\
            ├─ Samples: {}\n\
            ├─ Avg: {:.3} ms\n\
            ├─ Min: {:.3} ms\n\
            └─ Max: {:.3} ms",
            self.samples,
            self.avg_millis(),
            self.min_millis(),
            self.max_millis()
        )
    }
}
