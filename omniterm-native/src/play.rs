// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! The little packet snake. It has no terminal, network or filesystem access.
use std::collections::VecDeque;

pub const COLUMNS: i16 = 20;
pub const ROWS: i16 = 12;
pub type Cell = (i16, i16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { Up, Right, Down, Left }

impl Direction {
    fn delta(self) -> Cell {
        match self { Self::Up => (0, -1), Self::Right => (1, 0), Self::Down => (0, 1), Self::Left => (-1, 0) }
    }
    fn opposite(self, other: Self) -> bool {
        let (x, y) = self.delta();
        other.delta() == (-x, -y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Ready, Running, Paused, Over, Won }

pub struct Snake {
    pub body: VecDeque<Cell>,
    pub food: Option<Cell>,
    pub phase: Phase,
    pub score: u16,
    pub direction: Direction,
    pending: Option<Direction>,
    rng: u64,
}

impl Snake {
    pub fn new(seed: u64) -> Self {
        Self { body: [(5, 6), (4, 6), (3, 6)].into(), food: Some((12, 6)), phase: Phase::Ready, score: 0, direction: Direction::Right, pending: None, rng: seed.max(1) }
    }
    pub fn start(&mut self) {
        if matches!(self.phase, Phase::Over | Phase::Won) {
            *self = Self::new(self.rng.wrapping_add(0x9e3779b97f4a7c15));
        }
        self.phase = Phase::Running;
    }
    pub fn pause(&mut self) {
        if self.phase == Phase::Running { self.phase = Phase::Paused; }
    }
    pub fn turn(&mut self, direction: Direction) {
        // At most one turn per tick: Right -> Up -> Left must not reverse into the neck.
        if self.phase == Phase::Running && self.pending.is_none() && !self.direction.opposite(direction) && self.direction != direction {
            self.pending = Some(direction);
        }
    }
    pub fn step(&mut self) {
        if self.phase != Phase::Running { return; }
        self.direction = self.pending.take().unwrap_or(self.direction);
        let (dx, dy) = self.direction.delta();
        let head = self.body[0];
        let next = (head.0 + dx, head.1 + dy);
        let eating = self.food == Some(next);
        let occupied = self.body.len() - usize::from(!eating);
        if next.0 < 0 || next.0 >= COLUMNS || next.1 < 0 || next.1 >= ROWS || self.body.iter().take(occupied).any(|cell| *cell == next) {
            self.phase = Phase::Over;
            return;
        }
        self.body.push_front(next);
        if eating {
            self.score += 1;
            self.spawn_food();
        } else {
            self.body.pop_back();
        }
    }
    fn spawn_food(&mut self) {
        let free: Vec<_> = (0..ROWS).flat_map(|y| (0..COLUMNS).map(move |x| (x, y))).filter(|cell| !self.body.contains(cell)).collect();
        if free.is_empty() {
            self.food = None;
            self.phase = Phase::Won;
            return;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.food = Some(free[(self.rng % free.len() as u64) as usize]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grows_and_places_food_off_the_body() {
        let mut game = Snake::new(42);
        game.food = Some((6, 6)); game.start(); game.step();
        assert_eq!((game.score, game.body.len()), (1, 4));
        assert!(!game.body.contains(&game.food.unwrap()));
    }
    #[test]
    fn rejects_reverse_and_two_turns_between_ticks() {
        let mut game = Snake::new(42); game.start();
        game.turn(Direction::Left); game.step();
        assert_eq!(game.body[0], (6, 6));
        game.turn(Direction::Up); game.turn(Direction::Left); game.step();
        assert_eq!(game.body[0], (6, 5));
        assert_eq!(game.phase, Phase::Running);
    }
    #[test]
    fn can_move_into_the_vacated_tail() {
        let mut game = Snake::new(42);
        game.body = [(2, 2), (2, 3), (1, 3), (1, 2)].into();
        game.direction = Direction::Up; game.food = Some((8, 8)); game.start();
        game.turn(Direction::Left); game.step();
        assert_eq!(game.body[0], (1, 2)); assert_eq!(game.phase, Phase::Running);
    }
    #[test]
    fn collisions_stop_and_pause_does_not_advance() {
        let mut game = Snake::new(42); game.start(); game.pause();
        let body = game.body.clone(); game.step(); assert_eq!(game.body, body);
        game.start(); for _ in 0..COLUMNS { game.step(); }
        assert_eq!(game.phase, Phase::Over);
        assert!(game.body.iter().all(|(x, y)| (0..COLUMNS).contains(x) && (0..ROWS).contains(y)));
    }
    #[test]
    fn full_board_wins_without_retrying_random_positions() {
        let mut game = Snake::new(42);
        game.body = (0..ROWS).flat_map(|y| (0..COLUMNS).map(move |x| (x, y))).filter(|c| *c != (6, 6)).collect();
        game.body.retain(|c| *c != (5, 6)); game.body.push_front((5, 6));
        game.food = Some((6, 6)); game.start(); game.step();
        assert_eq!(game.phase, Phase::Won); assert_eq!(game.food, None);
    }
}
