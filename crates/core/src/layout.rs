// SPDX-License-Identifier: GPL-3.0-or-later

//! Placing overlapping entries of one day side by side.

/// Position of one item: `lane` of `lanes` columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lane {
    pub lane: usize,
    pub lanes: usize,
}

/// `items` are `(start, end)` in any unit; the result has the same order.
/// Items that overlap (directly or through a chain) share the width.
pub fn lanes<T: PartialOrd + Copy>(items: &[(T, T)]) -> Vec<Lane> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        items[a]
            .0
            .partial_cmp(&items[b].0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                items[b]
                    .1
                    .partial_cmp(&items[a].1)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });

    let mut out = vec![Lane { lane: 0, lanes: 1 }; items.len()];
    let mut group: Vec<usize> = Vec::new();
    let mut lane_ends: Vec<T> = Vec::new();
    let mut group_end: Option<T> = None;

    let flush = |group: &mut Vec<usize>, lane_ends: &mut Vec<T>, out: &mut Vec<Lane>| {
        for &i in group.iter() {
            out[i].lanes = lane_ends.len().max(1);
        }
        group.clear();
        lane_ends.clear();
    };

    for i in order {
        let (start, end) = items[i];
        if let Some(ge) = group_end
            && start >= ge
        {
            flush(&mut group, &mut lane_ends, &mut out);
            group_end = None;
        }
        let lane = match lane_ends.iter().position(|e| *e <= start) {
            Some(l) => {
                lane_ends[l] = end;
                l
            }
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
        out[i].lane = lane;
        group.push(i);
        group_end = Some(match group_end {
            Some(ge) if ge > end => ge,
            _ => end,
        });
    }
    flush(&mut group, &mut lane_ends, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separate_items_use_full_width() {
        let l = lanes(&[(9.0, 10.0), (10.0, 11.0), (12.0, 13.0)]);
        assert!(l.iter().all(|x| x.lanes == 1 && x.lane == 0));
    }

    #[test]
    fn workshop_and_call_side_by_side() {
        let l = lanes(&[(8.0, 17.0), (12.0, 13.0), (15.0, 16.0)]);
        assert_eq!(l[0], Lane { lane: 0, lanes: 2 });
        assert_eq!(l[1], Lane { lane: 1, lanes: 2 });
        // Lane 1 is free again at 15:00.
        assert_eq!(l[2], Lane { lane: 1, lanes: 2 });
    }

    #[test]
    fn three_way_overlap() {
        let l = lanes(&[(9, 12), (10, 11), (10, 12)]);
        assert_eq!(l.iter().map(|x| x.lanes).collect::<Vec<_>>(), vec![3, 3, 3]);
        let mut used: Vec<_> = l.iter().map(|x| x.lane).collect();
        used.sort();
        assert_eq!(used, vec![0, 1, 2]);
    }
}
