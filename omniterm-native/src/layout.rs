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
    fn ratios_stay_inside_their_limits() {
        let mut tree = Node::Pane("a".into()).split("a", "b", true);
        tree.set_ratio(&[], 0.0);
        let Node::Split { ratio, .. } = tree else { unreachable!() };
        assert_eq!(ratio, MIN_RATIO);
    }
}
