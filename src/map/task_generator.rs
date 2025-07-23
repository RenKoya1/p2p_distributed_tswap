use crate::map;
use crate::map::make_node;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Task {
    pub pickup: map::map::Point,
    pub delivery: map::map::Point,
    pub peer_id: Option<String>, // タスクの宛先peer id (Base58文字列)
    pub task_id: Option<u64>,    // タスクID
}

pub struct TaskGeneratorAgent<'a> {
    grid: &'a [Vec<char>],
    rng: rand::rngs::ThreadRng,
}

impl<'a> TaskGeneratorAgent<'a> {
    pub fn new(grid: &'a [Vec<char>]) -> Self {
        Self {
            grid,
            rng: rand::thread_rng(),
        }
    }

    pub fn generate_task(&mut self) -> Option<Task> {
        let (pickup, delivery) = make_node::generate_start_goal_pair(self.grid);

        Some(Task {
            pickup,
            delivery,
            peer_id: None,
            task_id: None,
        })
    }

    pub fn generate_multiple_tasks(&mut self, count: usize) -> Vec<Task> {
        let mut tasks = Vec::new();
        for _ in 0..count {
            if let Some(task) = self.generate_task() {
                tasks.push(task);
            } else {
                break;
            }
        }
        tasks
    }
}
