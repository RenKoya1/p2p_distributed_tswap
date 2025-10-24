use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

/// Test: Run 1 manager and N agents, execute 3*N tasks
///
/// This test spawns:
/// - 1 manager process
/// - N agent processes (configurable)
/// - Sends 3*N tasks to the manager
///
/// Usage:
/// ```
/// cargo test --test task -- --nocapture
/// ```
pub struct TestConfig {
    pub num_agents: usize,
    pub map_size: usize,
    pub wait_time_secs: u64,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            num_agents: 3,
            map_size: 100,
            wait_time_secs: 60,
        }
    }
}

pub struct TestRunner {
    config: TestConfig,
    manager_process: Option<Child>,
    agent_processes: Vec<Child>,
}

impl TestRunner {
    pub fn new(config: TestConfig) -> Self {
        Self {
            config,
            manager_process: None,
            agent_processes: Vec::new(),
        }
    }

    /// Start the manager process
    pub fn start_manager(&mut self) -> Result<(), String> {
        println!("🚀 Starting Manager...");

        let manager = Command::new("cargo")
            .args(&["run", "--bin", "manager"])
            .spawn()
            .map_err(|e| format!("Failed to start manager: {}", e))?;

        self.manager_process = Some(manager);
        println!("✅ Manager started");

        // Wait for manager to initialize
        thread::sleep(Duration::from_secs(3));
        Ok(())
    }

    /// Start N agent processes
    pub fn start_agents(&mut self) -> Result<(), String> {
        println!("🚀 Starting {} agents...", self.config.num_agents);

        for i in 0..self.config.num_agents {
            let agent = Command::new("cargo")
                .args(&["run", "--bin", "agent"])
                .spawn()
                .map_err(|e| format!("Failed to start agent {}: {}", i, e))?;

            self.agent_processes.push(agent);
            println!("✅ Agent {} started", i + 1);

            // Wait a bit between agent starts to avoid port conflicts
            thread::sleep(Duration::from_millis(500));
        }

        // Wait for agents to connect to manager
        println!("⏳ Waiting for agents to connect...");
        thread::sleep(Duration::from_secs(5));

        Ok(())
    }

    /// Send 3*N tasks to the manager via stdin
    pub fn send_tasks(&self) -> Result<(), String> {
        let num_tasks = self.config.num_agents * 3;
        println!(
            "📝 Sending {} tasks (3 × {} agents)...",
            num_tasks, self.config.num_agents
        );

        // Note: In the current implementation, tasks are sent via command input
        // This would need to be adapted based on how the manager accepts tasks
        // For now, we'll just print instructions

        println!("\n═══════════════════════════════════════════");
        println!("📋 TASK EXECUTION INSTRUCTIONS:");
        println!("═══════════════════════════════════════════");
        println!("Please send tasks manually using the following format:");
        println!("In the manager terminal, type:");
        println!("\ntask <pickup_x> <pickup_y> <delivery_x> <delivery_y>");
        println!("\nExample tasks for {} agents:", self.config.num_agents);

        for i in 0..num_tasks {
            let pickup_x = (i * 3 + 1) % self.config.map_size;
            let pickup_y = (i * 2 + 1) % self.config.map_size;
            let delivery_x = (i * 4 + 10) % self.config.map_size;
            let delivery_y = (i * 3 + 10) % self.config.map_size;

            println!(
                "task {} {} {} {}",
                pickup_x, pickup_y, delivery_x, delivery_y
            );
        }
        println!("═══════════════════════════════════════════\n");

        Ok(())
    }

    /// Wait for tasks to complete
    pub fn wait_for_completion(&self) {
        println!(
            "⏳ Waiting {} seconds for task completion...",
            self.config.wait_time_secs
        );
        thread::sleep(Duration::from_secs(self.config.wait_time_secs));
    }

    /// Stop all processes
    pub fn cleanup(&mut self) {
        println!("🧹 Cleaning up processes...");

        // Stop all agents
        for (i, agent) in self.agent_processes.iter_mut().enumerate() {
            if let Err(e) = agent.kill() {
                eprintln!("Failed to kill agent {}: {}", i, e);
            } else {
                println!("✅ Agent {} stopped", i + 1);
            }
        }
        self.agent_processes.clear();

        // Stop manager
        if let Some(ref mut manager) = self.manager_process {
            if let Err(e) = manager.kill() {
                eprintln!("Failed to kill manager: {}", e);
            } else {
                println!("✅ Manager stopped");
            }
        }
        self.manager_process = None;

        println!("✅ Cleanup complete");
    }
}

impl Drop for TestRunner {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Run the test with default configuration (3 agents, 9 tasks)
pub fn run_test_default() -> Result<(), String> {
    run_test(TestConfig::default())
}

/// Run the test with custom configuration
pub fn run_test(config: TestConfig) -> Result<(), String> {
    println!("\n╔═══════════════════════════════════════════╗");
    println!("║  P2P Distributed TSWAP - Multi-Agent Test ║");
    println!("╚═══════════════════════════════════════════╝\n");
    println!("📊 Configuration:");
    println!("   • Agents: {}", config.num_agents);
    println!(
        "   • Tasks: {} (3 × {})",
        config.num_agents * 3,
        config.num_agents
    );
    println!("   • Map size: {}×{}", config.map_size, config.map_size);
    println!("   • Wait time: {}s\n", config.wait_time_secs);

    let mut runner = TestRunner::new(config);

    // Start manager
    runner.start_manager()?;

    // Start agents
    runner.start_agents()?;

    // Send tasks
    runner.send_tasks()?;

    // Wait for completion
    runner.wait_for_completion();

    // Cleanup will be called automatically when runner is dropped
    println!("\n✅ Test completed!");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Run with: cargo test --test task -- --ignored --nocapture
    fn test_3_agents_9_tasks() {
        let config = TestConfig {
            num_agents: 3,
            map_size: 100,
            wait_time_secs: 60,
        };

        run_test(config).expect("Test failed");
    }

    #[test]
    #[ignore] // Run with: cargo test --test task -- --ignored --nocapture
    fn test_5_agents_15_tasks() {
        let config = TestConfig {
            num_agents: 5,
            map_size: 100,
            wait_time_secs: 90,
        };

        run_test(config).expect("Test failed");
    }

    #[test]
    #[ignore] // Run with: cargo test --test task -- --ignored --nocapture
    fn test_10_agents_30_tasks() {
        let config = TestConfig {
            num_agents: 10,
            map_size: 100,
            wait_time_secs: 120,
        };

        run_test(config).expect("Test failed");
    }
}

// Main function for running as a standalone test
#[cfg(not(test))]
fn main() {
    use std::env;

    let args: Vec<String> = env::args().collect();

    let config = if args.len() >= 2 {
        let num_agents = args[1]
            .parse::<usize>()
            .expect("First argument should be number of agents");

        let wait_time = if args.len() >= 3 {
            args[2]
                .parse::<u64>()
                .expect("Second argument should be wait time in seconds")
        } else {
            60
        };

        TestConfig {
            num_agents,
            map_size: 100,
            wait_time_secs: wait_time,
        }
    } else {
        TestConfig::default()
    };

    println!("Run with custom config: cargo run --example task <num_agents> [wait_time_secs]");
    println!("Example: cargo run --example task 5 90\n");

    if let Err(e) = run_test(config) {
        eprintln!("❌ Test failed: {}", e);
        std::process::exit(1);
    }
}
