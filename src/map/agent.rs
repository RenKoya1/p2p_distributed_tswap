use std::cmp::Ordering;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Agent {
    pub id: usize,
    pub v: usize,
    pub g: usize,
}

#[derive(Clone, Copy, Debug)]
pub enum AgentState {
    PICKING,
    CARRYING,
    DELIVERED,
    IDLE,
}
