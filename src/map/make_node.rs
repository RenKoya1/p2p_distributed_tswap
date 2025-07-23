use crate::map;
use rand::seq::SliceRandom;
use rand::thread_rng;

pub fn get_free_cells(map: &[Vec<char>]) -> Vec<map::map::Point> {
    let mut free = Vec::new();
    for (y, row) in map.iter().enumerate() {
        for (x, c) in row.iter().enumerate() {
            if *c == '.' {
                free.push((x, y));
            }
        }
    }
    free
}

pub fn generate_start_goal_pairs(
    map: &[Vec<char>],
    agent_count: usize,
) -> Vec<(map::map::Point, map::map::Point)> {
    let mut free_cells = get_free_cells(map);
    let mut rng = thread_rng();
    free_cells.shuffle(&mut rng);
    free_cells
        .chunks_exact(2)
        .take(agent_count)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

pub fn generate_start_goal_pair(map: &[Vec<char>]) -> (map::map::Point, map::map::Point) {
    let mut free_cells = get_free_cells(map);
    let mut rng = rand::thread_rng(); // 明示的に乱数生成器を指定

    free_cells.shuffle(&mut rng);

    assert!(
        free_cells.len() >= 2,
        "Free cells must be at least 2 to create a start-goal pair"
    );

    (free_cells[0], free_cells[1])
}

pub fn generate_start_positions(grid: &[Vec<char>], agent_count: usize) -> Vec<map::map::Point> {
    let mut free = get_free_cells(grid);
    free.shuffle(&mut rand::thread_rng());
    free.into_iter().take(agent_count).collect()
}
