use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::map::agent::AgentState;
use crate::map::map::Point;
use crate::map::task_generator::Task;

#[derive(Clone)]
struct Node {
    id: usize,
    pos: Point,
    neighbors: Vec<usize>,
}

#[derive(Clone, Copy, Debug)]
struct Agent {
    id: usize,
    v: usize, // current Node id
    g: usize, // goal Node id
}

impl PartialEq for Agent {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Agent {}
impl PartialOrd for Agent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Agent {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

pub fn tswap_mapd(
    grid: &[Vec<char>],
    initial_positions: Vec<Point>,
    tasks: &[Task],
) -> Vec<Vec<(Point, AgentState)>> {
    // --- Grid to Node graph conversion ---
    let mut nodes = vec![];
    let h = grid.len();
    let w = grid[0].len();
    let mut pos2id = HashMap::new();
    let mut id2pos = vec![];
    let mut node_id_counter = 0;
    for y in 0..h {
        for x in 0..w {
            if grid[y][x] != '@' {
                pos2id.insert((x, y), node_id_counter);
                id2pos.push((x, y));
                node_id_counter += 1;
            }
        }
    }
    for (id, &(x, y)) in id2pos.iter().enumerate() {
        let mut neighbors = vec![];
        for (dx, dy) in [(0, 1), (1, 0), (0, -1), (-1, 0)] {
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if nx >= 0 && ny >= 0 {
                let npos = (nx as usize, ny as usize);
                if pos2id.contains_key(&npos) {
                    neighbors.push(pos2id[&npos]);
                }
            }
        }
        nodes.push(Node {
            id,
            pos: (x, y),
            neighbors,
        });
    }

    let num_agents = initial_positions.len();
    let mut paths: Vec<Vec<(Point, AgentState)>> = vec![vec![]; num_agents];
    let mut task_used = vec![false; tasks.len()];

    #[derive(PartialEq, Clone, Copy)]
    enum InternalAgentState {
        Idle,
        ToPickup,
        ToDelivery,
    }
    let mut agent_internal_states = vec![InternalAgentState::Idle; num_agents];
    let mut agent_tasks: Vec<Option<Task>> = vec![None; num_agents];

    let mut agents: Vec<Agent> = (0..num_agents)
        .map(|i| {
            let start_node = pos2id[&initial_positions[i]];
            Agent {
                id: i,
                v: start_node,
                g: start_node,
            }
        })
        .collect();

    let mut timestep = 0;
    loop {
        // --- State and Task Management ---
        for i in 0..num_agents {
            if agents[i].v == agents[i].g {
                match agent_internal_states[i] {
                    InternalAgentState::ToPickup => {
                        agent_internal_states[i] = InternalAgentState::ToDelivery;
                        if let Some(task) = agent_tasks[i].as_ref() {
                            agents[i].g = pos2id[&task.delivery];
                        }
                    }
                    InternalAgentState::ToDelivery => {
                        agent_internal_states[i] = InternalAgentState::Idle;
                        agent_tasks[i] = None;
                    }
                    InternalAgentState::Idle => {}
                }
            }

            if agent_internal_states[i] == InternalAgentState::Idle {
                let current_pos = id2pos[agents[i].v];
                if let Some((task_idx, _)) = tasks
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !task_used[*i])
                    .map(|(i, task)| (i, manhattan_distance(current_pos, task.pickup)))
                    .min_by_key(|&(_, dist)| dist)
                {
                    task_used[task_idx] = true;
                    let task = &tasks[task_idx];
                    agent_tasks[i] = Some(task.clone());
                    agent_internal_states[i] = InternalAgentState::ToPickup;
                    agents[i].g = pos2id[&task.pickup];
                }
            }
        }

        tswap_step(&mut agents, &nodes);

        // --- Record Paths ---
        for i in 0..num_agents {
            let pos = id2pos[agents[i].v];
            let state = match agent_internal_states[i] {
                InternalAgentState::Idle => AgentState::IDLE,
                InternalAgentState::ToPickup => AgentState::PICKING,
                InternalAgentState::ToDelivery => {
                    if agents[i].v == agents[i].g {
                        AgentState::DELIVERED
                    } else {
                        AgentState::CARRYING
                    }
                }
            };
            paths[i].push((pos, state));
        }

        timestep += 1;

        // --- Termination Condition ---
        let all_tasks_done = task_used.iter().all(|&used| used);
        let all_agents_idle = agent_internal_states
            .iter()
            .all(|&s| s == InternalAgentState::Idle);
        if (all_tasks_done && all_agents_idle) || timestep > 2000 {
            break;
        }
    }
    paths
}

fn tswap_step(agents: &mut [Agent], nodes: &[Node]) {
    let n = agents.len();

    // Goal swapping phase
    for i in 0..n {
        if agents[i].v == agents[i].g {
            continue;
        }

        let path = get_path(agents[i].v, agents[i].g, nodes);
        if path.len() < 2 {
            continue;
        }
        let u = path[1];

        if let Some(j) = agents.iter().position(|b| b.v == u) {
            if i == j {
                continue;
            }

            if agents[j].v == agents[j].g {
                // Agent j is at its goal, swap goals
                let g_i = agents[i].g;
                let g_j = agents[j].g;
                agents[i].g = g_j;
                agents[j].g = g_i;
            } else {
                // Deadlock detection
                let mut a_p = vec![i];
                let mut current_b_idx = j;
                let mut deadlock_found = false;

                loop {
                    let b_v = agents[current_b_idx].v;
                    let b_g = agents[current_b_idx].g;

                    if b_v == b_g {
                        break;
                    }

                    let b_path = get_path(b_v, b_g, nodes);
                    if b_path.len() < 2 {
                        break;
                    }
                    let w = b_path[1];

                    if let Some(c_idx) = agents.iter().position(|c| c.v == w) {
                        if a_p.contains(&current_b_idx) {
                            a_p.clear();
                            break;
                        }
                        a_p.push(current_b_idx);
                        current_b_idx = c_idx;

                        if current_b_idx == i {
                            deadlock_found = true;
                            break;
                        }
                    } else {
                        break;
                    }
                }

                if deadlock_found && a_p.len() > 1 {
                    // Rotate targets
                    let first_agent_idx = a_p[0];
                    let last_goal = agents[a_p[a_p.len() - 1]].g;
                    for k in (1..a_p.len()).rev() {
                        let prev_g = agents[a_p[k - 1]].g;
                        agents[a_p[k]].g = prev_g;
                    }
                    agents[first_agent_idx].g = last_goal;
                }
            }
        }
    }

    // Movement phase - 全エージェントが移動可能
    for i in 0..n {
        if agents[i].v == agents[i].g {
            continue;
        }

        let path = get_path(agents[i].v, agents[i].g, nodes);
        if path.len() < 2 {
            continue;
        }
        let u = path[1];

        // 移動先が空いている、または相互交換の場合に移動
        if let Some(j) = agents.iter().position(|b| b.v == u) {
            if i != j {
                // Check if this is a mutual swap
                let path_j = get_path(agents[j].v, agents[j].g, nodes);
                if path_j.len() >= 2 && path_j[1] == agents[i].v {
                    // Mutual swap: both agents exchange positions
                    let temp_v = agents[i].v;
                    agents[i].v = agents[j].v;
                    agents[j].v = temp_v;
                }
            }
        } else {
            // 移動先が空いている場合は通常の移動
            agents[i].v = u;
        }
    }
}

fn get_path(start: usize, goal: usize, nodes: &[Node]) -> Vec<usize> {
    if start == goal {
        return vec![start];
    }

    #[derive(Clone)]
    struct AstarNode {
        node_id: usize,
        g_cost: usize,
        f_cost: usize,
    }

    impl PartialEq for AstarNode {
        fn eq(&self, other: &Self) -> bool {
            self.f_cost == other.f_cost
        }
    }

    impl Eq for AstarNode {}

    impl PartialOrd for AstarNode {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for AstarNode {
        fn cmp(&self, other: &Self) -> Ordering {
            other
                .f_cost
                .cmp(&self.f_cost)
                .then_with(|| other.g_cost.cmp(&self.g_cost))
        }
    }

    let mut open_list = BinaryHeap::new();
    let mut came_from = HashMap::new();
    let mut g_score = HashMap::new();

    let heuristic = |node_id: usize| -> usize {
        let (x1, y1) = nodes[node_id].pos;
        let (x2, y2) = nodes[goal].pos;
        ((x1 as isize - x2 as isize).abs() + (y1 as isize - y2 as isize).abs()) as usize
    };

    g_score.insert(start, 0);
    let start_node = AstarNode {
        node_id: start,
        g_cost: 0,
        f_cost: heuristic(start),
    };
    open_list.push(start_node);

    while let Some(current) = open_list.pop() {
        let current_id = current.node_id;

        if current_id == goal {
            let mut path = vec![];
            let mut current_node = current_id;
            path.push(current_node);

            while let Some(&parent) = came_from.get(&current_node) {
                path.push(parent);
                current_node = parent;
            }
            path.reverse();
            return path;
        }

        for &neighbor_id in &nodes[current_id].neighbors {
            let tentative_g = current.g_cost + 1;

            if tentative_g < *g_score.get(&neighbor_id).unwrap_or(&usize::MAX) {
                came_from.insert(neighbor_id, current_id);
                g_score.insert(neighbor_id, tentative_g);

                let h_cost = heuristic(neighbor_id);
                let f_cost = tentative_g + h_cost;

                let neighbor_node = AstarNode {
                    node_id: neighbor_id,
                    g_cost: tentative_g,
                    f_cost,
                };

                open_list.push(neighbor_node);
            }
        }
    }

    let mut best_neighbor = start;
    let mut min_dist = heuristic(start);

    for &neighbor_id in &nodes[start].neighbors {
        let dist = heuristic(neighbor_id);
        if dist < min_dist {
            min_dist = dist;
            best_neighbor = neighbor_id;
        }
    }

    vec![start, best_neighbor]
}

fn manhattan_distance(p1: Point, p2: Point) -> usize {
    ((p1.0 as isize - p2.0 as isize).abs() + (p1.1 as isize - p2.1 as isize).abs()) as usize
}
