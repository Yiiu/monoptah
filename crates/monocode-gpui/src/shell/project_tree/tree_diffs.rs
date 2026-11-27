use super::*;
use monocode_core::scm::{DiffStats, diff_stats};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Diffs {
    values: HashMap<String, (Instant, Option<DiffStats>)>,
    pending: HashSet<String>,
}

impl Diffs {
    pub(super) fn display(&self, path: &str) -> Option<String> {
        let stats = self.values.get(path)?.1.as_ref()?;
        Some(format!("+{} −{}", stats.additions, stats.deletions))
    }
}

impl Shell {
    pub(super) fn queue_tree_diff(&mut self, path: &str, cx: &mut Context<Self>) {
        if cfg!(test)
            || self
                .tree
                .diffs
                .values
                .get(path)
                .is_some_and(|(checked, _)| checked.elapsed() < Duration::from_secs(20))
            || !self.tree.diffs.pending.insert(path.to_owned())
        {
            return;
        }
        let path = path.to_owned();
        let cwd = PathBuf::from(&path);
        let work = cx
            .background_executor()
            .spawn(async move { diff_stats(&cwd).ok() });
        cx.spawn(async move |this, cx| {
            let stats = work.await;
            let _ = this.update(cx, |this, cx| {
                this.tree.diffs.pending.remove(&path);
                this.tree.diffs.values.insert(path, (Instant::now(), stats));
                this.notify_sidebar(cx);
            });
        })
        .detach();
    }
}
