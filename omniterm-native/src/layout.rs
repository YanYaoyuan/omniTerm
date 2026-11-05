// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Split panes in a tab: a binary tree of terminals, as in the desktop app's `layout.js`.

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Pane(String),
    Split { row: bool, ratio: f32, children: Box<[Node; 2]> },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub const MIN_RATIO: f32 = 0.1;

/// A divider between two children: where it sits and which split it resizes.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub path: Vec<usize>,
    pub row: bool,
    pub parent: Rect,
    pub position: f32,
}

impl Node {
    pub fn panes(&self) -> Vec<String> {
        match self {
            Self::Pane(id) => vec![id.clone()],
            Self::Split { children, .. } => children.iter().flat_map(Self::panes).collect(),
        }
    }

    /// Puts `new` beside `target`, after it, in a row (side by side) or column.
    pub fn split(&self, target: &str, new: &str, row: bool) -> Self {
        match self {
            Self::Pane(id) if id == target => {
                Self::Split { row, ratio: 0.5, children: Box::new([self.clone(), Self::Pane(new.to_string())]) }
            }
            Self::Pane(_) => self.clone(),
            Self::Split { row: r, ratio, children } => Self::Split {
                row: *r,
                ratio: *ratio,
                children: Box::new([children[0].split(target, new, row), children[1].split(target, new, row)]),
            },
        }
    }

    /// The tree without `target`, or `None` if nothing is left.
    pub fn remove(&self, target: &str) -> Option<Self> {
        match self {
            Self::Pane(id) => (id != target).then(|| self.clone()),
            Self::Split { row, ratio, children } => match (children[0].remove(target), children[1].remove(target)) {
                (Some(first), Some(second)) => Some(Self::Split { row: *row, ratio: *ratio, children: Box::new([first, second]) }),
                (first, second) => first.or(second),
            },
        }
    }

    pub fn set_ratio(&mut self, path: &[usize], value: f32) {
        if let Self::Split { ratio, children, .. } = self {
            match path.split_first() {
                None => *ratio = value.clamp(MIN_RATIO, 1.0 - MIN_RATIO),
                Some((index, rest)) => children[*index].set_ratio(rest, value),
            }
        }
    }

    /// Fractional rectangles for every pane, and the dividers between them.
    pub fn layout(&self, rect: Rect, path: &mut Vec<usize>, panes: &mut Vec<(String, Rect)>, dividers: &mut Vec<Divider>) {
        match self {
            Self::Pane(id) => panes.push((id.clone(), rect)),
            Self::Split { row, ratio, children } => {
                let (first, second, position) = if *row {
                    let w = rect.w * ratio;
                    (Rect { w, ..rect }, Rect { x: rect.x + w, w: rect.w - w, ..rect }, rect.x + w)
                } else {
                    let h = rect.h * ratio;
                    (Rect { h, ..rect }, Rect { y: rect.y + h, h: rect.h - h, ..rect }, rect.y + h)
                };
                dividers.push(Divider { path: path.clone(), row: *row, parent: rect, position });
                for (index, (child, area)) in children.iter().zip([first, second]).enumerate() {
                    path.push(index);
                    child.layout(area, path, panes, dividers);
                    path.pop();
                }
            }
        }
    }
}

/// Which side of a pane a dropped pane lands on: the edge nearest the pointer.
pub fn drop_side(x: f32, y: f32) -> Direction {
    let (left, right, up, down) = (x, 1.0 - x, y, 1.0 - y);
    let nearest = left.min(right).min(up).min(down);
    if nearest == left {
        Direction::Left
    } else if nearest == right {
        Direction::Right
    } else if nearest == up {
        Direction::Up
    } else {
        Direction::Down
    }
}

impl Node {
    /// Puts `new` on `side` of `target`, sharing its space equally.
    pub fn insert_beside(&self, target: &str, new: &str, side: Direction) -> Self {
        match self {
            Self::Pane(id) if id == target => {
                let row = matches!(side, Direction::Left | Direction::Right);
                let pane = Self::Pane(new.to_string());
                let children = if matches!(side, Direction::Left | Direction::Up) { [pane, self.clone()] } else { [self.clone(), pane] };
                Self::Split { row, ratio: 0.5, children: Box::new(children) }
            }
            Self::Pane(_) => self.clone(),
            Self::Split { row, ratio, children } => Self::Split { row: *row, ratio: *ratio, children: Box::new([children[0].insert_beside(target, new, side), children[1].insert_beside(target, new, side)]) },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Node {
    /// The closest pane in a direction, preferring panes that overlap along the other axis.
    pub fn neighbor_in(&self, from: &str, direction: Direction) -> Option<String> {
        let (mut panes, mut dividers) = (Vec::new(), Vec::new());
        self.layout(Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, &mut Vec::new(), &mut panes, &mut dividers);
        let origin = panes.iter().find(|(id, _)| id == from)?.1;
        let epsilon = 1e-6;
        let center = |rect: &Rect| (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        let (ox, oy) = center(&origin);
        panes
            .iter()
            .filter(|(id, _)| id != from)
            .filter(|(_, rect)| match direction {
                Direction::Left => rect.x + rect.w <= origin.x + epsilon,
                Direction::Right => rect.x >= origin.x + origin.w - epsilon,
                Direction::Up => rect.y + rect.h <= origin.y + epsilon,
                Direction::Down => rect.y >= origin.y + origin.h - epsilon,
            })
            .map(|(id, rect)| {
                let horizontal = matches!(direction, Direction::Left | Direction::Right);
                let overlaps = if horizontal {
                    rect.y < origin.y + origin.h - epsilon && rect.y + rect.h > origin.y + epsilon
                } else {
                    rect.x < origin.x + origin.w - epsilon && rect.x + rect.w > origin.x + epsilon
                };
                let (x, y) = center(rect);
                (id, (x - ox).hypot(y - oy) + if overlaps { 0.0 } else { 10.0 })
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id.clone())
    }

    /// Where focus goes when `closed` leaves `before`: its left or upper neighbor, in this tree.
    pub fn neighbor(&self, closed: &str, before: &Node) -> Option<String> {
        let ids = self.panes();
        [Direction::Left, Direction::Up].into_iter().filter_map(|direction| before.neighbor_in(closed, direction)).find(|id| ids.contains(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole() -> Rect {
        Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }
    }

    #[test]
    fn a_two_by_two_grid_gives_four_quarters() {
        let tree = Node::Pane("a".into()).split("a", "b", true).split("a", "c", false).split("b", "d", false);
        let (mut panes, mut dividers) = (Vec::new(), Vec::new());
        tree.layout(whole(), &mut Vec::new(), &mut panes, &mut dividers);
        assert_eq!(panes.len(), 4);
        assert_eq!(dividers.len(), 3);
        for (_, rect) in &panes {
            assert!((rect.w - 0.5).abs() < 1e-6 && (rect.h - 0.5).abs() < 1e-6);
        }
        assert_eq!(tree.panes(), ["a", "c", "b", "d"]);
    }

    #[test]
    fn removing_a_pane_gives_its_space_to_its_sibling() {
        let tree = Node::Pane("a".into()).split("a", "b", true);
        assert_eq!(tree.remove("b"), Some(Node::Pane("a".into())));
        assert_eq!(Node::Pane("a".into()).remove("a"), None);
    }

    #[test]
    fn neighbors_are_found_by_direction() {
        let tree = Node::Pane("a".into()).split("a", "b", true).split("b", "c", false);
        assert_eq!(tree.neighbor_in("a", Direction::Right).as_deref(), Some("b"));
        assert_eq!(tree.neighbor_in("c", Direction::Up).as_deref(), Some("b"));
        assert_eq!(tree.neighbor_in("c", Direction::Left).as_deref(), Some("a"));
        assert_eq!(tree.neighbor_in("a", Direction::Left), None);
        let after = tree.remove("c").unwrap();
        assert_eq!(after.neighbor("c", &tree).as_deref(), Some("a"));
    }

    #[test]
    fn dropped_panes_land_on_the_nearest_edge() {
        assert_eq!(drop_side(0.1, 0.5), Direction::Left);
        assert_eq!(drop_side(0.5, 0.95), Direction::Down);
        let tree = Node::Pane("a".into()).insert_beside("a", "b", Direction::Up);
        assert_eq!(tree.panes(), ["b", "a"]);
        let Node::Split { row, .. } = tree else { panic!() };
        assert!(!row);
    }

    #[test]
    fn ratios_stay_inside_their_limits() {
        let mut tree = Node::Pane("a".into()).split("a", "b", true);
        tree.set_ratio(&[], 0.0);
        let Node::Split { ratio, .. } = tree else { unreachable!() };
        assert_eq!(ratio, MIN_RATIO);
    }
}
