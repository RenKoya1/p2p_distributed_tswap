use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::map;
type Point = map::map::Point;
pub type NodeReservation = HashSet<(Point, usize)>;
pub type EdgeReservation = HashSet<((Point, Point), usize)>;

#[derive(Copy, Clone, Eq, PartialEq)]
struct TimeNode {
    pos: Point,
    g: usize, // これが「時刻」でもある
    f: usize,
}
impl Ord for TimeNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.cmp(&self.f).then_with(|| other.g.cmp(&self.g))
    }
}
impl PartialOrd for TimeNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn heuristic(a: Point, b: Point) -> usize {
    let dx = (a.0 as isize - b.0 as isize).abs() as usize;
    let dy = (a.1 as isize - b.1 as isize).abs() as usize;
    dx + dy
}

pub fn astar_with_reservation(
    grid: &[Vec<char>],
    start: Point,
    goal: Point,
    node_res: &NodeReservation,
    edge_res: &EdgeReservation,
    start_time: usize,
) -> Option<Vec<Point>> {
    let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1), (0, 0)]; // (0,0)はWAIT
    let mut open = BinaryHeap::new();
    let mut g_score = HashMap::new();
    let mut came_from = HashMap::new();

    g_score.insert((start, start_time), 0);
    open.push(TimeNode {
        pos: start,
        g: start_time,
        f: start_time + heuristic(start, goal),
    });

    while let Some(TimeNode { pos, g, .. }) = open.pop() {
        if pos == goal {
            let mut path = Vec::new();
            let mut cur = (pos, g);
            while let Some(&prev) = came_from.get(&cur) {
                path.push(cur.0);
                cur = prev;
            }
            path.push(start);
            path.reverse();
            return Some(path);
        }

        for &(dx, dy) in &dirs {
            let nx = pos.0 as isize + dx;
            let ny = pos.1 as isize + dy;
            if nx < 0 || ny < 0 {
                continue;
            }
            let np = (nx as usize, ny as usize);

            if np.1 >= map::map::HEIGHT || np.0 >= map::map::WIDTH || grid[np.1][np.0] != '.' {
                continue;
            }
            let next_time = g + 1;
            if node_res.contains(&(np, next_time)) {
                continue;
            }

            if edge_res.contains(&((pos, np), next_time))
                || edge_res.contains(&((np, pos), next_time))
            {
                continue;
            }

            if node_res.contains(&(np, next_time)) || node_res.contains(&(pos, next_time)) {
                continue;
            }
            if edge_res.contains(&((np, pos), next_time - 1))
                && edge_res.contains(&((pos, np), next_time))
            {
                continue;
            }

            let tentative_g = next_time;
            let key = (np, tentative_g);
            let best = *g_score.get(&key).unwrap_or(&usize::MAX);
            if tentative_g < best {
                came_from.insert(key, (pos, g));
                g_score.insert(key, tentative_g);
                let f = tentative_g + heuristic(np, goal);
                open.push(TimeNode {
                    pos: np,
                    g: tentative_g,
                    f,
                });
            }
        }
    }
    None
}
