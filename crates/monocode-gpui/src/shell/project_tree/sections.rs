//! Ordering and ownership for sessions that can belong to several sections.
use std::collections::HashSet;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Sections {
    pub pinned: Vec<i64>,
    pub reminders: Vec<i64>,
    pub folders: Vec<Vec<i64>>,
    pub loose: Vec<i64>,
}

pub(super) fn sections(
    ordered: &[i64],
    pinned: impl Fn(i64) -> bool,
    reminders: &[(i64, u64)],
    folders: &[&[i64]],
) -> Sections {
    let pins: Vec<_> = ordered.iter().copied().filter(|id| pinned(*id)).collect();
    let mut remaining: HashSet<_> = ordered.iter().copied().collect();
    remaining.retain(|id| !pinned(*id));

    let mut due = reminders.to_vec();
    due.sort_unstable_by_key(|&(id, at)| (at, id));
    let reminders = due
        .into_iter()
        .filter_map(|(id, _)| remaining.remove(&id).then_some(id))
        .collect();
    let folders = folders
        .iter()
        .map(|folder| {
            folder
                .iter()
                .copied()
                .filter(|id| remaining.remove(id))
                .collect()
        })
        .collect();

    Sections {
        pinned: pins,
        reminders,
        folders,
        loose: ordered
            .iter()
            .copied()
            .filter(|id| remaining.contains(id))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_uses_only_its_highest_priority_section() {
        assert_eq!(
            sections(
                &[1, 2, 3, 4, 5, 6],
                |id| id == 3 || id == 4,
                &[(4, 90), (2, 70), (1, 5), (12, 1)],
                &[&[1, 2, 3, 5], &[5]],
            ),
            Sections {
                pinned: vec![3, 4],
                reminders: vec![1, 2],
                folders: vec![vec![5], vec![]],
                loose: vec![6],
            },
        );
    }
}
