//! A virtualized project/worktree tree over the shell's session index.
mod render;
mod sections;

use super::project_rail::RailDragKind;
use super::*;
use crate::disclosure::{self, Disclosure};
use crate::session_mark::BadgeLook;
use gpui::{ListAlignment, ListOffset, ListState};
use monocode_core::{
    agent_status::Activity, files::path_key, session_filters::Status, time::now_ms,
};
use std::{
    borrow::Cow,
    cell::RefCell,
    ops::Range,
    time::{Duration, Instant},
};

pub(super) const COLLAPSED_WIDTH: f32 = 48.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub needs_you: usize,
    pub running: usize,
    pub unread: usize,
}

impl Counts {
    fn add(&mut self, activity: &Activity) {
        match activity {
            Activity::NeedsYou { .. } => self.needs_you += 1,
            Activity::Running { .. } | Activity::BackgroundWait { .. } => self.running += 1,
            Activity::Done { unread: true, .. } => self.unread += 1,
            _ => {}
        }
    }

    fn color(self) -> Option<gpui::Hsla> {
        if self.needs_you > 0 {
            Some(warning_color())
        } else if self.running > 0 {
            Some(agent_running_color())
        } else if self.unread > 0 {
            Some(agent_unread_color())
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProjectEntry {
    Heading(&'static str),
    Group(String),
    Project(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TreeRow {
    Monos,
    Header(ProjectEntry),
    Section {
        project: String,
        name: &'static str,
        count: usize,
        open: bool,
    },
    Folder {
        project: String,
        id: String,
        count: usize,
        open: bool,
    },
    Worktree {
        project: String,
        path: String,
        branch: Option<String>,
        open: bool,
        counts: Counts,
    },
    Session {
        id: i64,
        reorder_scope: Option<String>,
        depth: u8,
        last: bool,
        branch: Option<String>,
        children: usize,
        orchestration: bool,
    },
    Orchestration {
        id: i64,
        represented: Vec<i64>,
    },
    Archived(usize),
    /// A node's rows while it opens or closes: one list item clipped to a
    /// growing or shrinking height, so the rows below glide with it.
    Reveal {
        key: String,
        rows: Vec<TreeRow>,
    },
}

/// How long a finished row's badge takes to pop in: 1.4 times the disclosure
/// time, with an overshoot, as in the approved prototype.
const BADGE_POP: Duration = Duration::from_millis(280);

/// A row easing between the unread look (summary, badge, bright title) and
/// the read one.
#[derive(Clone, Copy, Debug)]
struct UnreadFade {
    start: Instant,
    /// Towards unread: a turn just finished. Otherwise the row is demoted.
    arriving: bool,
}

impl UnreadFade {
    fn elapsed(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.start).as_secs_f32()
    }

    /// 1 for the unread look, 0 for the read one, on the disclosure's curve.
    fn look(&self, now: Instant) -> f32 {
        let t = (self.elapsed(now) / disclosure::DURATION.as_secs_f32()).min(1.);
        let eased = disclosure::ease_out(t);
        if self.arriving { eased } else { 1. - eased }
    }

    fn badge(&self, now: Instant) -> BadgeLook {
        if !self.arriving {
            let look = self.look(now);
            return BadgeLook {
                scale: 0.3 + 0.7 * look,
                opacity: look,
            };
        }
        let t = (self.elapsed(now) / BADGE_POP.as_secs_f32()).min(1.);
        let popped = back_out(t);
        BadgeLook {
            scale: 0.2 + 0.8 * popped,
            opacity: popped.clamp(0., 1.),
        }
    }

    fn running(&self, now: Instant) -> bool {
        let length = if self.arriving {
            BADGE_POP
        } else {
            disclosure::DURATION
        };
        now.saturating_duration_since(self.start) < length
    }
}

/// Ease-out with a small overshoot (CSS `cubic-bezier(.34, 1.56, .64, 1)`).
fn back_out(t: f32) -> f32 {
    const C1: f32 = 1.70158;
    const C3: f32 = C1 + 1.;
    1. + C3 * (t - 1.).powi(3) + C1 * (t - 1.).powi(2)
}

fn project_key(path: &str) -> String {
    format!("project:{path}")
}

fn group_key(id: &str) -> String {
    format!("group:{id}")
}

fn section_key(project: &str, name: &str) -> String {
    format!("section:{project}:{name}")
}

fn folder_key(project: &str, id: &str) -> String {
    format!("folder:{project}:{id}")
}

fn worktree_key(path: &str) -> String {
    format!("worktree:{path}")
}

fn children_key(id: i64) -> String {
    format!("children:{id}")
}

const ARCHIVED_KEY: &str = "archived";

/// Whether `row` starts the next block after a dragged project (or group).
fn ends_block(row: &TreeRow, kind: &RailDragKind) -> bool {
    match (row, kind) {
        (TreeRow::Header(ProjectEntry::Project(_)), RailDragKind::Group) => false,
        (TreeRow::Header(_) | TreeRow::Archived(_) | TreeRow::Monos, _) => true,
        (
            TreeRow::Session {
                reorder_scope: Some(_),
                ..
            }
            | TreeRow::Folder { .. }
            | TreeRow::Section { .. }
            | TreeRow::Worktree { .. },
            RailDragKind::Session(_),
        ) => true,
        (
            TreeRow::Folder { .. }
            | TreeRow::Section { .. }
            | TreeRow::Worktree { .. }
            | TreeRow::Session { depth: 0..=1, .. },
            RailDragKind::Folder(_),
        ) => true,
        _ => false,
    }
}

fn drag_block_id<'a>(row: &'a TreeRow, kind: &RailDragKind) -> Option<Cow<'a, str>> {
    match (row, kind) {
        (TreeRow::Header(ProjectEntry::Project(path)), RailDragKind::Project)
        | (TreeRow::Header(ProjectEntry::Group(path)), RailDragKind::Group) => {
            Some(Cow::Borrowed(path))
        }
        (TreeRow::Folder { project, id, .. }, RailDragKind::Folder(scope)) if project == scope => {
            Some(Cow::Borrowed(id))
        }
        (
            TreeRow::Session {
                id,
                reorder_scope: Some(section),
                ..
            },
            RailDragKind::Session(scope),
        ) if section == scope => Some(Cow::Owned(id.to_string())),
        _ => None,
    }
}

fn session_ids(rows: &[TreeRow], ids: &mut Vec<i64>) {
    for row in rows {
        match row {
            TreeRow::Session { id, .. } => ids.push(*id),
            TreeRow::Reveal { rows, .. } => session_ids(rows, ids),
            _ => {}
        }
    }
}

struct TreeCatalog {
    projects: HashMap<String, Vec<i64>>,
    counts: HashMap<String, Counts>,
    worktrees: HashMap<String, HashMap<String, Counts>>,
    children: HashMap<i64, Vec<i64>>,
    matches: HashSet<i64>,
    requests: Vec<i64>,
    archived: Vec<i64>,
}

#[derive(Clone)]
struct TreeRowOwner {
    project: String,
    folder: Option<String>,
    session: Option<(String, i64)>,
}

pub(super) struct TreeState {
    list: ListState,
    rows: Vec<TreeRow>,
    counts: HashMap<String, Counts>,
    requests: Vec<i64>,
    request_disclosure: Disclosure,
    projects: HashMap<String, bool>,
    worktrees: HashSet<String>,
    sessions: HashSet<i64>,
    pub(super) archived_open: Option<bool>,
    pub(super) folder_project: Option<String>,
    reveal_project: Option<String>,
    last_request: Option<i64>,
    row_anchors: RefCell<Vec<AnchorSlot>>,
    diffs: tree_diffs::Diffs,
    /// The project, folder and session blocks a row moves with during reordering.
    owners: Vec<Option<TreeRowOwner>>,
    /// Nodes opening or closing, by node key.
    disclosures: HashMap<String, Disclosure>,
    /// Sessions seen while they were the open one. Their row keeps the unread
    /// look until another session is opened; counts already treat them as read.
    lingering: HashSet<i64>,
    /// Rows easing between the unread and read looks.
    fades: HashMap<i64, UnreadFade>,
}

mod tree_diffs;

impl TreeState {
    pub(super) fn new(cx: &mut Context<Shell>) -> Self {
        let list = ListState::new(0, ListAlignment::Top, px(96.));
        let shell = cx.weak_entity();
        list.set_scroll_handler(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.sidebar_scrolled(cx);
                shell.tree.clear_anchors();
                shell.notify_sidebar(cx);
            });
        });
        Self {
            list,
            rows: Vec::new(),
            counts: HashMap::new(),
            requests: Vec::new(),
            request_disclosure: Disclosure::new(false),
            projects: HashMap::new(),
            worktrees: HashSet::new(),
            sessions: HashSet::new(),
            archived_open: None,
            folder_project: None,
            reveal_project: None,
            last_request: None,
            row_anchors: RefCell::new(Vec::new()),
            diffs: Default::default(),
            owners: Vec::new(),
            disclosures: HashMap::new(),
            lingering: HashSet::new(),
            fades: HashMap::new(),
        }
    }

    /// Something in the tree is moving: a node opening or closing, or a row
    /// changing between unread and read.
    fn animating(&self, now: Instant) -> bool {
        self.request_disclosure.animating(now)
            || self.disclosures.values().any(|d| d.animating(now))
            || self.fades.values().any(|fade| fade.running(now))
    }

    fn disclosing(&self, key: &str, now: Instant) -> bool {
        self.disclosures.get(key).is_some_and(|d| d.animating(now))
    }

    /// How open a node is drawn, from 0 (shut) to 1 (open), for its chevron.
    fn openness(&self, key: &str, open: bool, now: Instant) -> f32 {
        self.disclosures
            .get(key)
            .filter(|d| d.animating(now))
            .map_or(if open { 1. } else { 0. }, |d| d.fraction(now))
    }

    /// Each block spans its header and child rows, using the list's height
    /// estimates until those rows have been measured. Bounds are unshifted.
    pub(super) fn drag_blocks(
        &self,
        kind: &RailDragKind,
        include: impl Fn(&str) -> bool,
    ) -> Vec<(String, Bounds<Pixels>)> {
        let mut blocks = Vec::new();
        for (ix, row) in self.rows.iter().enumerate() {
            let Some(id) = drag_block_id(row, kind) else {
                continue;
            };
            if !include(&id) {
                continue;
            }
            let Some(head) = self.list.estimated_bounds_for_item(ix) else {
                continue;
            };
            let end = self.drag_block_end(ix, kind);
            let bottom = self
                .list
                .estimated_bounds_for_item(end - 1)
                .map_or(head.bottom(), |last| last.bottom());
            blocks.push((
                id.into_owned(),
                Bounds::from_corners(head.origin, gpui::point(head.right(), bottom)),
            ));
        }
        blocks
    }

    fn drag_block_end(&self, start: usize, kind: &RailDragKind) -> usize {
        self.rows[start + 1..]
            .iter()
            .position(|row| ends_block(row, kind))
            .map_or(self.rows.len(), |n| start + 1 + n)
    }

    fn drag_block_rows(&self, kind: &RailDragKind, id: &str) -> Option<Range<usize>> {
        let start = self
            .rows
            .iter()
            .position(|row| drag_block_id(row, kind).as_deref() == Some(id))?;
        Some(start..self.drag_block_end(start, kind))
    }

    pub(super) fn session_reorder_scope(&self, session: i64) -> Option<String> {
        self.rows.iter().find_map(|row| match row {
            TreeRow::Session {
                id, reorder_scope, ..
            } if *id == session => reorder_scope.clone(),
            _ => None,
        })
    }

    pub(super) fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.list.viewport_bounds()
    }

    pub(super) fn scroll_top(&self) -> Pixels {
        -self.list.scroll_px_offset_for_scrollbar().y
    }

    pub(super) fn scroll_while_dragging(&self, velocity: f32, seconds: f32) -> bool {
        let top = self.scroll_top();
        let max = self.list.max_offset_for_scrollbar().y;
        let next = (top + px(velocity * seconds)).clamp(px(0.), max);
        self.list.scroll_by(next - top);
        if velocity > 0. {
            next < max
        } else {
            next > px(0.)
        }
    }

    fn set_owners(&mut self) {
        let mut project = None;
        let mut folder = None;
        let mut session = None;
        self.owners = self
            .rows
            .iter()
            .map(|row| {
                match row {
                    TreeRow::Header(ProjectEntry::Project(path)) => {
                        project = Some(path);
                        folder = None;
                        session = None;
                    }
                    TreeRow::Header(_) | TreeRow::Archived(_) | TreeRow::Monos => {
                        project = None;
                        folder = None;
                        session = None;
                    }
                    TreeRow::Folder { id, .. } => {
                        folder = Some(id);
                        session = None;
                    }
                    TreeRow::Section { .. } | TreeRow::Worktree { .. } => {
                        folder = None;
                        session = None;
                    }
                    TreeRow::Session {
                        id,
                        depth,
                        reorder_scope,
                        ..
                    } => {
                        if *depth <= 1 {
                            folder = None;
                        }
                        if let Some(scope) = reorder_scope {
                            session = Some((scope.clone(), *id));
                        }
                    }
                    _ => {}
                }
                project.map(|project| TreeRowOwner {
                    project: project.clone(),
                    folder: folder.cloned(),
                    session: session.clone(),
                })
            })
            .collect();
    }

    fn clear_anchors(&self) {
        for slot in self.row_anchors.borrow_mut().drain(..) {
            slot.set(None);
        }
    }

    pub(super) fn track_anchor(&self, anchor: &AnchorSlot) {
        self.row_anchors.borrow_mut().push(anchor.clone());
    }

    fn replace_rows(&mut self, next: Vec<TreeRow>) {
        let first = self
            .rows
            .iter()
            .zip(&next)
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = self.rows[first..]
            .iter()
            .rev()
            .zip(next[first..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        if first + suffix != self.rows.len() || first + suffix != next.len() {
            self.list.splice_with_height_hint(
                first..self.rows.len() - suffix,
                next.len() - first - suffix,
                px(28.),
            );
            self.rows = next;
            self.set_owners();
        }
        if let Some(project) = self.reveal_project.take()
            && let Some(ix) = self.rows.iter().position(|row| {
                matches!(row, TreeRow::Header(ProjectEntry::Project(path)) if path == &project)
            })
        {
            self.list.scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) });
        }
    }
}

impl Shell {
    fn tree_session_worktree<'a>(&'a self, id: i64, project: &'a str, cx: &'a App) -> &'a str {
        if self.is_cloud(id) {
            return project;
        }
        self.session_working_copy(id, cx)
    }

    fn tree_catalog(&self, cx: &App) -> TreeCatalog {
        let start = self.workspace.session_filters.time.start(now_ms());
        let monos = self.monos.as_ref().map(|entity| entity.read(cx));
        let workers: HashMap<_, _> = self
            .orchestration_summaries
            .iter()
            .flat_map(|(&parent, run)| {
                run.tasks
                    .iter()
                    .filter_map(move |task| task.session.map(|id| (id, parent)))
            })
            .collect();
        let mut project_by_session = HashMap::new();
        let mut counts: HashMap<String, Counts> = HashMap::new();
        let mut worktrees: HashMap<String, HashMap<String, Counts>> = HashMap::new();
        let mut matches = HashSet::new();
        let mut archived = Vec::new();
        let mut requests = Vec::new();
        for (&id, summary) in &self.session_summaries {
            let status = self.agent_status(id);
            let launched_by_mono = monos.is_some_and(|m| m.tree_launch_mono(id).is_some());
            let project = self.project_of(id, &summary.cwd);
            let listed = launched_by_mono
                || (!self.sidebar_hidden_sessions.contains(&id)
                    && !monos.is_some_and(|m| m.hidden_session(id)))
                || self.started_by.contains_key(&id)
                || workers.contains_key(&id);
            if !summary.archived
                && !self.deleting.contains(&id)
                && status.needs_you()
                && !monos.is_some_and(|m| {
                    m.roster
                        .monos
                        .iter()
                        .any(|mono| mono.habit_sessions.contains(&id))
                })
            {
                requests.push(id);
            }
            if !listed || self.deleting.contains(&id) {
                continue;
            }
            if !summary.has_messages
                && matches!(status.activity, Activity::Idle)
                && !self.session_metadata(id, cx).has_saved_draft()
            {
                continue;
            }
            let time_and_status_match = self.workspace.session_filters.matches(
                self.session_metadata(id, cx).summary,
                Status {
                    working: status.is_busy(),
                    needs_approval: status.needs_you(),
                    done: status.unread_done(),
                },
                start,
                true,
            );
            if time_and_status_match {
                matches.insert(id);
            }
            if summary.archived {
                if time_and_status_match {
                    archived.push(id);
                }
                continue;
            }
            project_by_session.insert(id, project.to_owned());
            counts
                .entry(project.to_owned())
                .or_default()
                .add(&status.activity);
            if !project.starts_with("remote://") {
                worktrees
                    .entry(project.to_owned())
                    .or_default()
                    .entry(path_key(self.tree_session_worktree(id, project, cx)))
                    .or_default()
                    .add(&status.activity);
            }
        }

        // Mono main chats may not have a regular sidebar row.
        if let Some(monos) = monos {
            for view in monos.all_notification_chats(cx) {
                let id = view.read(cx).summary.id;
                if self.agent_status(id).needs_you()
                    && !self.deleting.contains(&id)
                    && !view.read(cx).summary.archived
                    && !monos
                        .roster
                        .monos
                        .iter()
                        .any(|mono| mono.habit_sessions.contains(&id))
                {
                    requests.push(id);
                }
            }
        }
        let sort_key = |id: &i64| {
            let activity = &self.agent_status(*id).activity;
            let updated = self.session_summaries.get(id).map_or(0, |s| s.updated_at);
            let at = match activity {
                Activity::Done { at, .. } => updated.max(*at),
                _ => updated,
            };
            (std::cmp::Reverse(at), *id)
        };
        requests.sort_unstable_by_key(sort_key);
        requests.dedup();
        archived.sort_unstable_by_key(sort_key);

        // Preserve a parent when an included child matches a filter. Flatten
        // Operator chains, so the activity underneath a session is one layer.
        let mut projects: HashMap<String, Vec<i64>> = HashMap::new();
        let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
        let parent_of = |id| {
            workers
                .get(&id)
                .or_else(|| self.started_by.get(&id))
                .copied()
        };
        for (&id, project) in &project_by_session {
            if !matches.contains(&id) {
                continue;
            }
            let mut root = id;
            let mut seen = HashSet::from([id]);
            while let Some(parent) = parent_of(root) {
                if project_by_session.get(&parent) != Some(project) || !seen.insert(parent) {
                    break;
                }
                root = parent;
            }
            projects.entry(project.clone()).or_default().push(root);
            if id != root {
                children.entry(root).or_default().push(id);
            }
        }
        for ids in projects.values_mut().chain(children.values_mut()) {
            ids.sort_unstable_by_key(sort_key);
            ids.dedup();
        }
        TreeCatalog {
            projects,
            counts,
            worktrees,
            children,
            matches,
            requests,
            archived,
        }
    }

    fn tree_project_entries(&self, show_collapsed_groups: bool) -> Vec<ProjectEntry> {
        let workspace = &self.workspace;
        let mut entries = Vec::new();
        let pinned = |path: &String| workspace.project_rail_pinned.contains(path);
        let project = |path: &String| ProjectEntry::Project(path.clone());
        if workspace.projects.iter().any(pinned) {
            entries.push(ProjectEntry::Heading("Pinned"));
            entries.extend(workspace.projects.iter().filter(|p| pinned(p)).map(project));
        }
        for group in &workspace.project_rail_groups {
            entries.push(ProjectEntry::Group(group.id.clone()));
            if show_collapsed_groups || !group.collapsed {
                entries.extend(
                    workspace
                        .projects
                        .iter()
                        .filter(|path| {
                            !pinned(path)
                                && workspace.project_rail_group(path) == Some(group.id.as_str())
                        })
                        .map(project),
                );
            }
        }
        entries.push(ProjectEntry::Heading("Projects"));
        entries.extend(
            workspace
                .projects
                .iter()
                .filter(|path| !pinned(path) && workspace.project_rail_group(path).is_none())
                .map(project),
        );
        entries
    }

    fn tree_filtered(&self) -> bool {
        self.workspace.session_filters.active(false)
    }

    fn tree_project_open(&self, path: &str) -> bool {
        self.tree
            .projects
            .get(path)
            .copied()
            .unwrap_or(path == self.workspace.project)
            || self.tree_filtered()
    }

    fn tree_branch(&self, id: i64, cx: &App) -> Option<String> {
        self.sessions
            .get(&id)
            .and_then(|v| v.read(cx).branch_name().map(str::to_owned))
            .or_else(|| {
                self.session_details
                    .get(&id)
                    .and_then(|details| details.branch.clone())
            })
    }

    fn tree_rows(&self, catalog: &TreeCatalog, cx: &App) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        if self.workspace.preferences.monos {
            rows.push(TreeRow::Monos);
        }
        let mut entries = self.tree_project_entries(true).into_iter().peekable();
        while let Some(entry) = entries.next() {
            match entry {
                ProjectEntry::Group(id) => {
                    let open = self.tree_group_open(&id);
                    let key = group_key(&id);
                    rows.push(TreeRow::Header(ProjectEntry::Group(id)));
                    let mut projects = Vec::new();
                    while matches!(entries.peek(), Some(ProjectEntry::Project(_))) {
                        if let Some(ProjectEntry::Project(path)) = entries.next() {
                            projects.push(path);
                        }
                    }
                    self.tree_children(&mut rows, key, open, |rows| {
                        for path in projects {
                            self.tree_project_block(rows, path, catalog, cx);
                        }
                    });
                }
                ProjectEntry::Project(path) => {
                    self.tree_project_block(&mut rows, path, catalog, cx)
                }
                heading => rows.push(TreeRow::Header(heading)),
            }
        }
        if !catalog.archived.is_empty() {
            rows.push(TreeRow::Archived(catalog.archived.len()));
            let open = self
                .tree
                .archived_open
                .unwrap_or(self.workspace.preferences.show_archived);
            self.tree_children(&mut rows, ARCHIVED_KEY.into(), open, |rows| {
                self.tree_session_rows(rows, &catalog.archived, (1, true), None, catalog, cx)
            });
        }
        rows
    }

    fn tree_group_open(&self, id: &str) -> bool {
        self.tree_filtered()
            || self
                .workspace
                .project_rail_groups
                .iter()
                .any(|group| group.id == id && !group.collapsed)
    }

    fn tree_project_block(
        &self,
        rows: &mut Vec<TreeRow>,
        path: String,
        catalog: &TreeCatalog,
        cx: &App,
    ) {
        let ids = catalog.projects.get(&path).map_or(&[][..], Vec::as_slice);
        if ids.is_empty() && self.tree_filtered() {
            return;
        }
        let open = self.tree_project_open(&path);
        let key = project_key(&path);
        rows.push(TreeRow::Header(ProjectEntry::Project(path.clone())));
        self.tree_children(rows, key, open, |rows| {
            self.tree_project_rows(rows, &path, ids, catalog, cx);
        });
    }

    /// Adds a node's rows under it. At rest they are list items of their own;
    /// while the node opens or closes they travel as one [`TreeRow::Reveal`]
    /// that the disclosure clips.
    fn tree_children(
        &self,
        rows: &mut Vec<TreeRow>,
        key: String,
        open: bool,
        build: impl FnOnce(&mut Vec<TreeRow>),
    ) {
        if !self.tree.disclosing(&key, Instant::now()) {
            if open {
                build(rows);
            }
            return;
        }
        let mut inner = Vec::new();
        build(&mut inner);
        if !inner.is_empty() {
            rows.push(TreeRow::Reveal { key, rows: inner });
        }
    }

    /// Starts a node opening or closing. Reduced motion makes it instant.
    fn tree_disclose(&mut self, key: String, open: bool, cx: &App) {
        let now = Instant::now();
        let animate = !cx.reduce_motion();
        match self.tree.disclosures.get_mut(&key) {
            // A toggle in the middle of one continues from where it is.
            Some(disclosure) if disclosure.animating(now) => disclosure.set(open, animate, now),
            _ => {
                let mut disclosure = Disclosure::new(!open);
                disclosure.set(open, animate, now);
                self.tree.disclosures.insert(key, disclosure);
            }
        }
    }

    /// Something in the open tree is moving, so the sidebar keeps drawing
    /// frames: a node opening or closing, a row changing between unread and
    /// read, or projects stepping aside for a drag.
    pub(super) fn tree_animating(&self) -> bool {
        self.workspace.tree_layout()
            && (self.tree.animating(Instant::now())
                || self.rail_shifting()
                || self.tree_drop_frame())
    }

    /// Keeps the tree's unread looks in step with a session's new activity.
    /// Being seen changes no count: only how the open row is drawn lingers.
    pub(super) fn tree_note_unread(
        &mut self,
        id: i64,
        was_unread: bool,
        activity: &Activity,
        cx: &App,
    ) {
        if !self.workspace.tree_layout() {
            return;
        }
        let now = Instant::now();
        let animate = !cx.reduce_motion();
        let unread = matches!(activity, Activity::Done { unread: true, .. });
        let done = matches!(activity, Activity::Done { .. });
        if !done {
            // A new turn: the row shows it at once.
            self.tree.lingering.remove(&id);
            self.tree.fades.remove(&id);
        } else if unread && !was_unread {
            self.tree.lingering.remove(&id);
            if animate {
                self.tree.fades.insert(
                    id,
                    UnreadFade {
                        start: now,
                        arriving: true,
                    },
                );
            }
        } else if was_unread && !unread {
            if self.tree_row_in_view(id, cx) {
                // A turn that finishes in the open session is seen at once: let
                // its badge finish popping in, since it ends on the look the row
                // lingers in.
                if self.tree.fades.get(&id).is_some_and(|fade| !fade.arriving) {
                    self.tree.fades.remove(&id);
                }
                self.tree.lingering.insert(id);
            } else if animate {
                self.tree.fades.insert(
                    id,
                    UnreadFade {
                        start: now,
                        arriving: false,
                    },
                );
            } else {
                self.tree.fades.remove(&id);
            }
        }
    }

    fn tree_row_in_view(&self, id: i64, cx: &App) -> bool {
        self.workspace.active == Some(id)
            && !self.app_view_open()
            && self.settings.is_none()
            && self.app_search.is_none()
            && !self.mono_active(cx)
    }

    pub(super) fn settle_tree_unread_now(&mut self) {
        self.tree.lingering.clear();
        self.tree.fades.clear();
    }

    /// Demote rows whose session is no longer visible and expire finished fades.
    pub(super) fn tree_settle_unread(&mut self, cx: &App) -> bool {
        let now = Instant::now();
        let animate = !cx.reduce_motion();
        let left: Vec<i64> = self
            .tree
            .lingering
            .iter()
            .copied()
            .filter(|id| !self.tree_row_in_view(*id, cx))
            .collect();
        let demoted = !left.is_empty();
        for id in left {
            self.tree.lingering.remove(&id);
            if animate {
                self.tree.fades.insert(
                    id,
                    UnreadFade {
                        start: now,
                        arriving: false,
                    },
                );
            }
        }
        self.tree
            .fades
            .retain(|_, fade| animate && fade.running(now));
        self.tree.disclosures.retain(|_, d| d.animating(now));
        demoted
    }

    /// How unread `id`'s row is drawn, 0 (read) to 1 (unread), and its badge.
    /// Only a finished session has an unread look.
    fn tree_unread_look(&self, id: i64, activity: &Activity) -> (f32, BadgeLook) {
        let Activity::Done { unread, .. } = activity else {
            return (0., BadgeLook::FULL);
        };
        let now = Instant::now();
        if let Some(fade) = self.tree.fades.get(&id).filter(|fade| fade.running(now)) {
            return (fade.look(now), fade.badge(now));
        }
        let look = *unread || self.tree.lingering.contains(&id);
        (if look { 1. } else { 0. }, BadgeLook::FULL)
    }

    fn tree_project_rows(
        &self,
        rows: &mut Vec<TreeRow>,
        path: &str,
        ids: &[i64],
        catalog: &TreeCatalog,
        cx: &App,
    ) {
        let groups = self.project_groups_for(path);
        let reminders: Vec<_> = self
            .reminders
            .iter()
            .map(|r| (r.session_id, r.due_at))
            .collect();
        let folders: Vec<_> = groups
            .folders
            .iter()
            .map(|f| f.session_ids.as_slice())
            .collect();
        let split = sections::sections(
            ids,
            |id| self.session_metadata(id, cx).summary.pinned,
            &reminders,
            &folders,
        );
        for (name, ids, collapsed) in [
            ("Pinned", split.pinned, groups.pinned_collapsed),
            ("Reminders", split.reminders, groups.reminders_collapsed),
        ] {
            if ids.is_empty() {
                continue;
            }
            let open = !collapsed || self.tree_filtered();
            rows.push(TreeRow::Section {
                project: path.to_owned(),
                name,
                count: ids.len(),
                open,
            });
            let key = section_key(path, name);
            self.tree_children(rows, key.clone(), open, |rows| {
                let scope = (name == "Pinned").then_some(key.as_str());
                self.tree_session_rows(rows, &ids, (2, true), scope, catalog, cx)
            });
        }
        for (folder, ids) in groups.folders.iter().zip(split.folders) {
            if ids.is_empty() {
                continue;
            }
            let open = !folder.collapsed || self.tree_filtered();
            rows.push(TreeRow::Folder {
                project: path.to_owned(),
                id: folder.id.clone(),
                count: ids.len(),
                open,
            });
            let key = folder_key(path, &folder.id);
            self.tree_children(rows, key.clone(), open, |rows| {
                self.tree_session_rows(rows, &ids, (2, true), Some(&key), catalog, cx)
            });
        }
        let mut by_worktree: Vec<(String, Vec<i64>)> = Vec::new();
        let mut known = HashMap::new();
        for &id in &split.loose {
            let copy = self.tree_session_worktree(id, path, cx);
            let key = path_key(copy);
            let index = *known.entry(key).or_insert_with(|| {
                by_worktree.push((copy.to_owned(), Vec::new()));
                by_worktree.len() - 1
            });
            by_worktree[index].1.push(id);
        }
        let Some(copies) = catalog
            .worktrees
            .get(path)
            .filter(|_| by_worktree.len() > 1)
        else {
            self.tree_session_rows(
                rows,
                &split.loose,
                (1, false),
                Some(&worktree_key(path)),
                catalog,
                cx,
            );
            return;
        };
        let copy_key = path_key(path);
        by_worktree.sort_by_key(|(copy, _)| path_key(copy) != copy_key);
        for (copy, members) in by_worktree {
            let open = !self.tree.worktrees.contains(&copy) || self.tree_filtered();
            let counts = copies.get(&path_key(&copy)).copied().unwrap_or_default();
            let branch = members
                .iter()
                .filter(|&&id| !self.is_cloud(id))
                .find_map(|id| self.tree_branch(*id, cx));
            let key = worktree_key(&copy);
            rows.push(TreeRow::Worktree {
                project: path.to_owned(),
                path: copy,
                branch,
                open,
                counts,
            });
            self.tree_children(rows, key.clone(), open, |rows| {
                self.tree_session_rows(rows, &members, (2, false), Some(&key), catalog, cx)
            });
        }
    }

    fn tree_session_rows(
        &self,
        rows: &mut Vec<TreeRow>,
        ids: &[i64],
        (depth, show_branch): (u8, bool),
        reorder_scope: Option<&str>,
        catalog: &TreeCatalog,
        cx: &App,
    ) {
        let ordered;
        let ids = if let Some(order) =
            reorder_scope.and_then(|scope| self.workspace.tree_session_order.get(scope))
        {
            let by_rank: HashMap<_, _> = order
                .iter()
                .enumerate()
                .map(|(index, id)| (*id, index))
                .collect();
            let mut sessions = ids.to_vec();
            // New sessions retain their natural activity order before the ranked ones.
            sessions.sort_by_key(|id| by_rank.get(id).copied());
            ordered = sessions;
            &ordered
        } else {
            ids
        };
        for (ix, &id) in ids.iter().enumerate() {
            let children = catalog.children.get(&id).map_or(&[][..], Vec::as_slice);
            let orchestration = self
                .orchestration_summaries
                .get(&id)
                .filter(|run| !run.tasks.is_empty());
            let child_count = orchestration.map_or(children.len(), |run| {
                children.len()
                    + run
                        .tasks
                        .iter()
                        .filter(|task| task.session.is_none_or(|id| !children.contains(&id)))
                        .count()
            });
            rows.push(TreeRow::Session {
                id,
                reorder_scope: reorder_scope.map(str::to_owned),
                depth,
                last: ix + 1 == ids.len(),
                branch: show_branch.then(|| self.tree_branch(id, cx)).flatten(),
                children: child_count,
                orchestration: orchestration.is_some(),
            });
            let open = self.tree.sessions.contains(&id)
                || (!catalog.matches.contains(&id) && !children.is_empty());
            self.tree_children(rows, children_key(id), open, |rows| {
                if !self.tree_filtered() && orchestration.is_some() && child_count > children.len()
                {
                    rows.push(TreeRow::Orchestration {
                        id,
                        represented: children.to_vec(),
                    });
                }
                for (ix, &child) in children.iter().enumerate() {
                    rows.push(TreeRow::Session {
                        id: child,
                        reorder_scope: None,
                        depth: depth + 1,
                        last: ix + 1 == children.len(),
                        branch: None,
                        children: 0,
                        orchestration: false,
                    });
                }
            });
        }
    }

    pub(super) fn record_tree_session_order(
        &mut self,
        scope: String,
        previous: &[String],
        reordered: Vec<String>,
    ) {
        let original: Vec<i64> = previous.iter().filter_map(|id| id.parse().ok()).collect();
        let visible: HashSet<_> = original.iter().copied().collect();
        let saved = self.workspace.tree_session_order.entry(scope).or_default();
        let unranked: Vec<_> = original
            .into_iter()
            .filter(|id| !saved.contains(id))
            .collect();
        saved.splice(0..0, unranked);
        let mut next = reordered
            .into_iter()
            .filter_map(|id| id.parse::<i64>().ok());
        for id in saved {
            if visible.contains(id)
                && let Some(reordered) = next.next()
            {
                *id = reordered;
            }
        }
    }

    pub(super) fn tree_session_ids(&self, cx: &App) -> Vec<i64> {
        let mut ids = Vec::new();
        session_ids(&self.tree_rows(&self.tree_catalog(cx), cx), &mut ids);
        ids
    }

    pub(super) fn focus_tree_session_worktree(&mut self, id: i64, cx: &App) {
        let Some(summary) = self.session_summaries.get(&id) else {
            return;
        };
        let project = self.project_of(id, &summary.cwd).to_owned();
        let path = self.tree_session_worktree(id, &project, cx).to_owned();
        if path_key(&path) == path_key(&project) || project.starts_with("remote://") {
            self.worktree_focus.remove(&project);
        } else {
            let branch = self.tree_branch(id, cx);
            self.worktree_focus.insert(
                project,
                crate::worktree_switcher::WorktreeFocus { path, branch },
            );
        }
    }

    pub(super) fn open_tree_session(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_status(id).needs_you() {
            self.tree.last_request = Some(id);
        }
        let opened_mono = self
            .monos
            .as_ref()
            .filter(|monos| monos.read(cx).roster.of_session(id).is_some())
            .cloned()
            .is_some_and(|monos| {
                monos.update(cx, |monos, cx| {
                    monos.open_notification_session(id, window, cx)
                })
            });
        if !opened_mono && self.session_summaries.contains_key(&id) {
            self.focus_tree_session_worktree(id, cx);
            self.open(id, window, cx);
        }
        if self.peek_open() {
            self.set_peek(false, cx);
        }
    }

    pub(super) fn open_next_tree_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace.tree_layout() {
            return;
        }
        let ids = self.tree_catalog(cx).requests;
        let last = self.tree.last_request.or(self.workspace.active);
        if let Some(id) = cycle(&ids, last.as_ref(), 1).copied() {
            self.tree.last_request = Some(id);
            self.open_tree_session(id, window, cx);
        }
    }

    pub(super) fn step_tree_session(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self.tree_session_ids(cx);
        if let Some(id) = cycle(&ids, self.workspace.active.as_ref(), direction).copied() {
            self.open_tree_session(id, window, cx);
        }
    }

    pub(super) fn step_tree_project(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let projects: Vec<_> = self
            .tree_project_entries(true)
            .into_iter()
            .filter_map(|item| match item {
                ProjectEntry::Project(path) => Some(path),
                _ => None,
            })
            .collect();
        if let Some(path) = cycle(&projects, Some(&self.workspace.project), direction) {
            self.reveal_tree_project(path.clone(), cx);
            self.select_project(path.clone(), window, cx);
        }
    }

    fn reveal_tree_project(&mut self, project: String, cx: &mut Context<Self>) {
        if let Some(id) = self
            .workspace
            .project_rail_group(&project)
            .map(str::to_owned)
            && let Some(group) = self
                .workspace
                .project_rail_groups
                .iter_mut()
                .find(|g| g.id == id)
        {
            group.collapsed = false;
        }
        self.tree.projects.insert(project.clone(), true);
        self.tree.reveal_project = Some(project);
        self.invalidate_chrome(cx);
        cx.notify();
    }
}

fn cycle<'a, T: PartialEq>(items: &'a [T], current: Option<&T>, direction: isize) -> Option<&'a T> {
    let len = items.len() as isize;
    if len == 0 {
        return None;
    }
    let active = current.and_then(|current| items.iter().position(|item| item == current));
    let index = active.map_or(if direction < 0 { len - 1 } else { 0 }, |ix| {
        (ix as isize + direction).rem_euclid(len)
    });
    items.get(index as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::NavigationLayout;
    use gpui::TestAppContext;
    use monocode_core::agent_status::{AgentStatus, Ask};

    #[gpui::test]
    fn needs_you_opens_the_other_project_and_uses_its_worktree(cx: &mut TestAppContext) {
        let (window, requested, previous) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                shell
                    .workspace
                    .set_navigation_layout(NavigationLayout::Tree);
                assert_eq!(shell.workspace.active, Some(previous));
                let copy = "/work/another-project-feature";
                shell
                    .session_worktrees
                    .insert(requested, Some(copy.to_owned()));
                shell.agent_statuses.insert(
                    requested,
                    AgentStatus {
                        activity: Activity::NeedsYou {
                            kind: Ask::Question,
                        },
                        ..Default::default()
                    },
                );
                assert_eq!(shell.tree_catalog(cx).requests, [requested]);
                shell.open_next_tree_request(window, cx);
                assert_eq!(shell.workspace.active, Some(requested));
                assert_eq!(shell.workspace.project, "/work/another-project");
                assert_eq!(shell.working_copy(), copy);
                let tab = shell.workspace.tab_of(requested).unwrap();
                assert_eq!(
                    shell.workspace.tab_workspaces.get(&tab).map(String::as_str),
                    Some(copy)
                );
            })
            .unwrap();
    }

    /// The Tree layout drops Focused's project header, whose menu was the only
    /// visible way into Settings on Windows. The tree must offer its own, both
    /// open and collapsed, and the shortcut must keep working.
    #[gpui::test]
    fn tree_layout_reaches_settings_open_collapsed_and_by_shortcut(cx: &mut TestAppContext) {
        use gpui::{VisualTestContext, px, size};
        cx.update(|cx| cx.set_reduce_motion(true));
        cx.update(crate::shell::bind_keys);
        let (window, _, _) = crate::shell::tests::window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(1400.), px(900.)));
        window
            .update(visual, |shell, _, cx| {
                shell.set_navigation_layout(NavigationLayout::Tree, cx)
            })
            .unwrap();
        visual.run_until_parked();

        let gear = visual
            .debug_bounds("tree-settings")
            .expect("the open tree shows a Settings button");
        visual.simulate_click(gear.center(), Default::default());
        visual.run_until_parked();
        assert!(
            window
                .update(visual, |shell, _, _| shell.settings.is_some())
                .unwrap()
        );

        window
            .update(visual, |shell, window, cx| {
                shell.close_settings(window, cx);
                shell.toggle_primary_sidebar(window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        let gear = visual
            .debug_bounds("tree-status-settings")
            .expect("the collapsed status bar shows a Settings button");
        visual.simulate_click(gear.center(), Default::default());
        visual.run_until_parked();
        assert!(
            window
                .update(visual, |shell, _, _| shell.settings.is_some())
                .unwrap()
        );

        window
            .update(visual, |shell, window, cx| {
                shell.close_settings(window, cx);
                shell.focus_active(window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.simulate_keystrokes("secondary-,");
        visual.run_until_parked();
        assert!(
            window
                .update(visual, |shell, _, _| shell.settings.is_some())
                .unwrap()
        );
    }

    #[gpui::test]
    fn only_the_tree_layout_uses_the_tree_mono_heading(cx: &mut TestAppContext) {
        use gpui::{VisualTestContext, px, size};
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = crate::shell::tests::window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(1400.), px(900.)));
        for (layout, height) in [
            (NavigationLayout::Tree, 26.),
            (NavigationLayout::Focused, 28.),
            (NavigationLayout::Classic, 30.),
        ] {
            window
                .update(visual, |shell, _, cx| {
                    shell.workspace.project_rail_open = true;
                    shell.workspace.sidebar_visible = true;
                    shell.set_navigation_layout(layout, cx);
                })
                .unwrap();
            visual.run_until_parked();
            let heading = visual
                .debug_bounds("sidebar-mono-heading")
                .expect("Mono heading");
            assert_eq!(heading.size.height, px(height));
        }
    }

    #[gpui::test]
    fn tree_peek_badges_take_the_floating_surface_in_every_appearance(cx: &mut TestAppContext) {
        let (window, mut visual, _, _) = tree_window(cx);
        for (theme, opacity) in [("dark", 85), ("dark", 100), ("light", 85)] {
            window
                .update(&mut visual, |shell, _, cx| {
                    shell.workspace.preferences.theme = theme.into();
                    shell.workspace.preferences.sidebar_opacity = opacity;
                    shell.invalidate_chrome(cx);
                    cx.notify();
                })
                .unwrap();
            visual.refresh().unwrap();
            window
                .update(&mut visual, |shell, window, cx| {
                    assert_eq!(shell.tree_badge_plate(), shell_fill());
                    shell.toggle_primary_sidebar(window, cx);
                    shell.set_peek(true, cx);
                    assert!(shell.peek_open());
                    assert_eq!(shell.tree_badge_plate(), floating_fill());
                })
                .unwrap();
            visual.refresh().unwrap();
            assert!(visual.debug_bounds("sidebar-peek").is_some());
            window
                .update(&mut visual, |shell, window, cx| {
                    shell.toggle_primary_sidebar(window, cx);
                    assert_eq!(shell.tree_badge_plate(), shell_fill());
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn tree_group_clicks_disclose_the_group_members(cx: &mut TestAppContext) {
        let (window, mut visual, _, _) = tree_window(cx);
        window
            .update(&mut visual, |shell, _, cx| {
                shell
                    .workspace
                    .project_rail_groups
                    .push(crate::workspace::ProjectRailGroup {
                        id: "release".into(),
                        name: "Release".into(),
                        ..Default::default()
                    });
                shell
                    .workspace
                    .project_rail_assignments
                    .insert("/work/monocode".into(), "release".into());
                shell.notify_sidebar(cx);
            })
            .unwrap();
        visual.run_until_parked();
        let header = visual
            .debug_bounds("tree-group-release")
            .expect("Tree group");
        assert_eq!(header.size.height, px(30.));
        visual.update(|_, cx| cx.set_reduce_motion(false));
        visual.simulate_click(header.center(), Default::default());
        window
            .update(&mut visual, |shell, _, cx| {
                let key = group_key("release");
                assert!(!shell.tree_group_open("release"));
                assert!(shell.tree.disclosing(&key, Instant::now()));
                let rows = shell.tree_rows(&shell.tree_catalog(cx), cx);
                assert!(rows.iter().any(|row| matches!(
                    row, TreeRow::Reveal { key: k, rows } if k == &key && !rows.is_empty()
                )));
            })
            .unwrap();
        visual.simulate_click(header.center(), Default::default());
        window
            .update(&mut visual, |shell, _, _| {
                assert!(shell.tree_group_open("release"));
                assert!(shell.tree.disclosing(&group_key("release"), Instant::now()));
            })
            .unwrap();
        visual.update(|_, cx| cx.set_reduce_motion(true));
        visual.simulate_click(header.center(), Default::default());
        window
            .update(&mut visual, |shell, _, _| {
                assert!(!shell.tree_group_open("release"));
                assert!(!shell.tree.disclosing(&group_key("release"), Instant::now()));
            })
            .unwrap();
    }

    #[gpui::test]
    fn classic_group_headers_keep_their_geometry_and_toggle_in_place(cx: &mut TestAppContext) {
        use gpui::{VisualTestContext, px, size};
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = crate::shell::tests::window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(1400.), px(900.)));
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.project_rail_open = true;
                shell.set_navigation_layout(NavigationLayout::Classic, cx);
                shell
                    .workspace
                    .project_rail_groups
                    .push(crate::workspace::ProjectRailGroup {
                        id: "release".into(),
                        name: "Release".into(),
                        ..Default::default()
                    });
                shell
                    .workspace
                    .project_rail_assignments
                    .insert("/work/monocode".into(), "release".into());
                shell.projects_frame.update(cx, |_, cx| cx.notify());
            })
            .unwrap();
        visual.run_until_parked();
        let header = visual
            .debug_bounds("rail-group-release")
            .expect("Classic group");
        assert_eq!(header.size.height, px(32.));
        visual.simulate_click(header.center(), Default::default());
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds("rail-group-release"), Some(header));
        window
            .update(visual, |shell, _, _| {
                assert!(shell.workspace.project_rail_groups[0].collapsed);
                assert!(shell.tree.disclosures.is_empty());
            })
            .unwrap();
    }

    #[gpui::test]
    fn needs_you_uses_a_neutral_empty_state_and_discloses_requests(cx: &mut TestAppContext) {
        use monocode_core::agent_status::Ask;
        let (window, id, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(render::needs_you_color(0), content(0.4));
                assert_eq!(render::needs_you_color(1), warning_color());
                for reduce_motion in [false, true] {
                    cx.set_reduce_motion(reduce_motion);
                    shell.agent_statuses.remove(&id);
                    shell.tree_sidebar(cx);
                    assert!(shell.tree.requests.is_empty());
                    assert!(!shell.tree.request_disclosure.mounted(Instant::now()));
                    shell.agent_statuses.insert(
                        id,
                        AgentStatus {
                            activity: Activity::NeedsYou {
                                kind: Ask::Question,
                            },
                            ..Default::default()
                        },
                    );
                    shell.tree_sidebar(cx);
                    assert_eq!(shell.tree.requests, [id]);
                    let now = Instant::now();
                    assert!(shell.tree.request_disclosure.mounted(now));
                    assert_eq!(shell.tree.request_disclosure.animating(now), !reduce_motion);
                }
            })
            .unwrap();
    }

    #[gpui::test]
    fn only_loose_sessions_determine_whether_worktree_rows_are_needed(cx: &mut TestAppContext) {
        let (window, pinned, loose) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                let project = "/work/another-project";
                shell.tree.projects.insert(project.into(), true);
                shell.session_summaries.get_mut(&pinned).unwrap().pinned = true;
                shell.sessions[&pinned]
                    .clone()
                    .update(cx, |view, _| view.summary.pinned = true);
                let rows = shell.tree_rows(&shell.tree_catalog(cx), cx);
                assert!(rows.iter().any(|row| matches!(
                    row,
                    TreeRow::Section {
                        name: "Pinned",
                        count: 1,
                        ..
                    }
                )));
                assert!(
                    !rows
                        .iter()
                        .any(|row| matches!(row, TreeRow::Worktree { .. }))
                );

                let summary = shell.session_summaries.get_mut(&loose).unwrap();
                summary.cwd = project.into();
                summary.has_messages = true;
                if let Some(view) = shell.sessions.get(&loose).cloned() {
                    view.update(cx, |view, _| {
                        view.summary.cwd = project.into();
                        view.summary.has_messages = true;
                    });
                }
                shell
                    .session_worktrees
                    .insert(loose, Some("/work/topic".into()));
                let catalog = shell.tree_catalog(cx);
                assert_eq!(catalog.worktrees[project].len(), 2);
                let rows = shell.tree_rows(&catalog, cx);
                assert!(rows.iter().any(|row| matches!(
                    row, TreeRow::Session { id, depth: 1, .. } if *id == loose
                )));
                assert!(
                    !rows
                        .iter()
                        .any(|row| matches!(row, TreeRow::Worktree { .. }))
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn tree_project_menus_offer_codespaces_for_the_clicked_project(cx: &mut TestAppContext) {
        let (window, _, _) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, window, cx| {
                shell.workspace.set_navigation_layout(NavigationLayout::Tree);
                let github = "/work/another-project";
                shell.github_projects.insert(github.into(), true);
                shell.github_projects.insert("remote://example/project".into(), true);
                assert_ne!(shell.workspace.project, github);
                for (path, rail, offered) in [
                    (github, true, true),
                    (github, false, false),
                    ("/work/monocode", true, false),
                    ("remote://example/project", true, false),
                ] {
                    let (_, items, _, _) = shell.menu_items(
                        &MenuTarget::Project { path: path.into(), rail },
                        &mut Vec::new(),
                        window,
                        cx,
                    );
                    assert_eq!(
                        items.iter().any(|item| matches!(
                            item, MenuItem::Action(action) if action.id.as_ref() == "new-codespace-session"
                        )),
                        offered,
                        "{path}, rail={rail}",
                    );
                }
            })
            .unwrap();
    }

    /// A project row's hover "+" starts a session there without the context menu.
    #[gpui::test]
    fn a_project_row_starts_a_session_from_its_hover_button(cx: &mut TestAppContext) {
        use gpui::{VisualTestContext, px, size};
        cx.update(|cx| cx.set_reduce_motion(true));
        let (window, _, _) = crate::shell::tests::window(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(1400.), px(900.)));
        window
            .update(visual, |shell, _, cx| {
                shell.set_navigation_layout(NavigationLayout::Tree, cx)
            })
            .unwrap();
        visual.run_until_parked();
        let project = "/work/another-project";
        let before = window
            .update(visual, |shell, _, _| {
                (shell.workspace.active, shell.workspace.tabs.len())
            })
            .unwrap();

        let row = visual
            .debug_bounds(Box::leak(
                format!("tree-project-{project}").into_boxed_str(),
            ))
            .expect("the project row is drawn");
        visual.simulate_mouse_move(row.center(), None, Default::default());
        visual.run_until_parked();
        let plus = visual
            .debug_bounds(Box::leak(
                format!("tree-project-new-session-{project}").into_boxed_str(),
            ))
            .expect("hovering the project shows its New session button");
        visual.simulate_click(plus.center(), Default::default());
        visual.run_until_parked();

        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.project, project);
                let blank = shell
                    .pending_project_blanks
                    .iter()
                    .any(|key| key.starts_with(&format!("{project}\0")));
                assert!(
                    blank || (shell.workspace.active, shell.workspace.tabs.len()) != before,
                    "a new session is started in the clicked project"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn worktree_hover_button_creates_and_reuses_a_blank_in_that_copy(cx: &mut TestAppContext) {
        let (window, mut visual, main, topic) = tree_window(cx);
        let project = "/work/another-project";
        let copy = "/work/another-project-topic";
        let before = window
            .update(&mut visual, |shell, _, cx| {
                let summary = shell.session_summaries.get_mut(&topic).unwrap();
                summary.cwd = project.into();
                summary.has_messages = true;
                if let Some(view) = shell.sessions.get(&topic).cloned() {
                    view.update(cx, |view, _| {
                        view.summary.cwd = project.into();
                        view.summary.has_messages = true;
                    });
                }
                shell.session_worktrees.insert(topic, Some(copy.into()));
                shell.notify_sidebar(cx);
                shell.session_summaries.len()
            })
            .unwrap();
        visual.run_until_parked();
        let row = visual
            .debug_bounds("tree-worktree-/work/another-project-topic")
            .expect("the topic worktree is drawn");
        visual.simulate_mouse_move(row.center(), None, Default::default());
        visual.run_until_parked();
        let button = visual
            .debug_bounds("tree-worktree-new-session-/work/another-project-topic")
            .expect("hovering the topic worktree shows the New session button");
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();

        let blank = window
            .update(&mut visual, |shell, _, cx| {
                assert_eq!(shell.workspace.project, project);
                assert_eq!(shell.working_copy(), copy);
                let id = shell.workspace.active.expect("the blank has opened");
                assert_ne!(id, main);
                assert_ne!(id, topic);
                assert_eq!(shell.session_working_copy(id, cx), copy);
                assert!(shell.is_untouched_project_blank(id, cx));
                assert_eq!(shell.session_summaries.len(), before + 1);
                id
            })
            .unwrap();
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();
        window
            .update(&mut visual, |shell, _, _| {
                assert_eq!(shell.workspace.active, Some(blank));
                assert_eq!(shell.session_summaries.len(), before + 1);
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_hidden_mono_launch_appears_in_its_worktree_with_the_mascot(cx: &mut TestAppContext) {
        let (window, mut visual, ordinary, launched) = tree_window(cx);
        let project = "/work/another-project";
        let copy = "/work/another-project-review";
        let mono_id = window
            .update(&mut visual, |shell, _, cx| {
                let summary = shell.session_summaries.get_mut(&launched).unwrap();
                summary.cwd = project.into();
                summary.has_messages = true;
                shell.sessions[&launched].clone().update(cx, |chat, _| {
                    chat.summary.cwd = project.into();
                    chat.summary.has_messages = true;
                });
                shell.session_worktrees.insert(launched, Some(copy.into()));
                shell.sidebar_hidden_sessions.insert(launched);
                let mono_id = shell.mono_manager().update(cx, |monos, cx| {
                    let id = monos.roster.create(vec![project.into()]).id.clone();
                    let mono = monos.roster.get_mut(&id).unwrap();
                    mono.show_started_sessions_in_sidebar = false;
                    mono.launches.push(monocode_core::mono::Launch {
                        session_id: launched,
                        cwd: copy.into(),
                        title: "Review the release".into(),
                        harness: "claude".into(),
                        model: None,
                        origin_turn: 1,
                        notify_on_complete: false,
                        completed: false,
                        dismissed: false,
                        each: false,
                        visible: false,
                    });
                    cx.notify();
                    id
                });
                let rows = shell.tree_rows(&shell.tree_catalog(cx), cx);
                let worktree = rows
                    .iter()
                    .position(|row| matches!(row, TreeRow::Worktree { path, .. } if path == copy))
                    .expect("launch worktree row");
                assert!(matches!(
                    &rows[worktree + 1],
                    TreeRow::Session { id, depth: 2, .. } if *id == launched
                ));
                assert_eq!(
                    rows.iter()
                        .filter(|row| matches!(row, TreeRow::Session { id, .. } if *id == launched))
                        .count(),
                    1,
                );
                shell.notify_sidebar(cx);
                mono_id
            })
            .unwrap();
        visual.refresh().unwrap();
        let worktree = visual
            .debug_bounds("tree-worktree-/work/another-project-review")
            .unwrap();
        let row = format!("sidebar-session-{launched}").leak();
        let row = visual.debug_bounds(row).expect("rendered launch row");
        let mascot = format!("tree-session-mono-{launched}-{mono_id}").leak();
        let mascot = visual.debug_bounds(mascot).expect("rendered mascot");
        assert!(worktree.bottom() <= row.top());
        assert!(mascot.left() >= row.left() && mascot.right() <= row.right());
        let other = format!("tree-session-mono-{ordinary}-{mono_id}").leak();
        assert!(visual.debug_bounds(other).is_none());
    }

    #[gpui::test]
    fn collapsed_projects_still_count_waiting_work_but_exclude_archive(cx: &mut TestAppContext) {
        use monocode_core::agent_status::{Outcome, Wait};
        let (window, archived, running) = crate::shell::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell
                    .workspace
                    .set_navigation_layout(NavigationLayout::Tree);
                shell.session_summaries.get_mut(&archived).unwrap().archived = true;
                shell.tree.projects.insert("/work/monocode".into(), false);
                shell.agent_statuses.insert(
                    running,
                    AgentStatus {
                        activity: Activity::BackgroundWait {
                            reason: Wait::Subtasks(1),
                            since: 0,
                        },
                        ..Default::default()
                    },
                );
                shell.agent_statuses.insert(
                    archived,
                    AgentStatus {
                        activity: Activity::Done {
                            at: 0,
                            outcome: Outcome::Completed,
                            unread: true,
                        },
                        ..Default::default()
                    },
                );
                let catalog = shell.tree_catalog(cx);
                assert_eq!(catalog.archived, [archived]);
                assert_eq!(catalog.counts["/work/monocode"].running, 1);
                assert!(!catalog.counts.contains_key("/work/another-project"));
                assert!(
                    !shell
                        .tree_rows(&catalog, cx)
                        .iter()
                        .any(|row| { matches!(row, TreeRow::Session { .. }) })
                );
            })
            .unwrap();
    }

    fn tree_window(
        cx: &mut TestAppContext,
    ) -> (gpui::WindowHandle<Shell>, gpui::VisualTestContext, i64, i64) {
        use gpui::{VisualTestContext, px, size};
        cx.update(|cx| cx.set_reduce_motion(true));
        cx.update(crate::shell::bind_keys);
        let (window, a, b) = crate::shell::tests::window(cx);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(size(px(1400.), px(900.)));
        window
            .update(&mut visual, |shell, _, cx| {
                shell.set_navigation_layout(NavigationLayout::Tree, cx);
                for project in ["/work/another-project", "/work/monocode"] {
                    shell.tree.projects.insert(project.into(), true);
                }
            })
            .unwrap();
        visual.run_until_parked();
        (window, visual, a, b)
    }

    /// Opening an unread finished session marks it seen at once, but its row
    /// keeps the summary while it is the open one and drops it once another
    /// session is opened.
    #[gpui::test]
    fn an_opened_unread_row_keeps_its_summary_until_another_session_opens(cx: &mut TestAppContext) {
        use monocode_core::{
            agent_status::{Outcome, TurnEnd},
            store::SessionActivity,
        };
        let (window, mut visual, a, b) = tree_window(cx);
        let visual = &mut visual;
        let summary = format!("tree-session-summary-{a}").leak();
        window
            .update(visual, |shell, window, cx| {
                shell.session_activity.insert(
                    a,
                    SessionActivity {
                        last_turn: Some(TurnEnd {
                            at: 1_000,
                            outcome: Outcome::Completed,
                        }),
                        progress_summary: Some("Made the timeout configurable".into()),
                        ..Default::default()
                    },
                );
                shell.update_agent_status(a, None, cx);
                assert!(shell.agent_status(a).unread_done());
                shell.open(a, window, cx);
                shell.mark_agent_seen(a, cx);
                assert!(
                    !shell.agent_status(a).unread_done(),
                    "opening it counts it as seen at once"
                );
                assert_eq!(
                    shell.tree_catalog(cx).counts["/work/another-project"].unread,
                    0
                );
            })
            .unwrap();
        visual.run_until_parked();
        assert!(
            visual.debug_bounds(summary).is_some(),
            "the open row keeps its summary"
        );

        window
            .update(visual, |shell, window, cx| shell.open(b, window, cx))
            .unwrap();
        visual.run_until_parked();
        assert!(
            visual.debug_bounds(summary).is_none(),
            "leaving it demotes the row"
        );
        window
            .update(visual, |shell, _, _| {
                assert!(shell.tree.lingering.is_empty());
                assert!(
                    shell.tree.fades.is_empty(),
                    "reduced motion demotes at once"
                );
            })
            .unwrap();
    }

    fn open_finished_tree_row(
        shell: &mut Shell,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Shell>,
    ) {
        use monocode_core::{
            agent_status::{Outcome, TurnEnd},
            store::SessionActivity,
        };
        shell.session_activity.insert(
            id,
            SessionActivity {
                last_turn: Some(TurnEnd {
                    at: 1_000,
                    outcome: Outcome::Completed,
                }),
                progress_summary: Some("Updated the release checklist".into()),
                ..Default::default()
            },
        );
        shell.update_agent_status(id, None, cx);
        shell.open(id, window, cx);
        shell.mark_agent_seen(id, cx);
        assert!(shell.tree.lingering.contains(&id));
    }

    #[gpui::test]
    fn a_tree_layout_round_trip_drops_lingering_without_a_fade(cx: &mut TestAppContext) {
        let (window, mut visual, first, second) = tree_window(cx);
        window
            .update(&mut visual, |shell, w, cx| {
                cx.set_reduce_motion(false);
                open_finished_tree_row(shell, first, w, cx);
                shell.set_navigation_layout(NavigationLayout::Classic, cx);
                assert!(shell.tree.lingering.is_empty());
                assert!(shell.tree.fades.is_empty());
                shell.open(second, w, cx);
                shell.set_navigation_layout(NavigationLayout::Tree, cx);
                let activity = &shell.agent_status(first).activity;
                assert_eq!(shell.tree_unread_look(first, activity).0, 0.);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds(format!("tree-session-summary-{first}").leak())
                .is_none()
        );
    }

    #[gpui::test]
    fn opening_inbox_demotes_the_finished_row_even_if_the_session_stays_active(
        cx: &mut TestAppContext,
    ) {
        let (window, mut visual, first, _) = tree_window(cx);
        window
            .update(&mut visual, |shell, w, cx| {
                cx.set_reduce_motion(false);
                open_finished_tree_row(shell, first, w, cx);
                shell.open_inbox(w, cx);
                assert_eq!(shell.workspace.active, Some(first));
                assert!(!shell.tree.lingering.contains(&first));
                let fade = shell.tree.fades[&first];
                assert!(!fade.arriving);
                assert_eq!(fade.look(fade.start + disclosure::DURATION), 0.);

                shell.open(first, w, cx);
                cx.set_reduce_motion(true);
                open_finished_tree_row(shell, first, w, cx);
                shell.open_inbox(w, cx);
                assert!(shell.tree.fades.is_empty());
                let activity = &shell.agent_status(first).activity;
                assert_eq!(shell.tree_unread_look(first, activity).0, 0.);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds(format!("tree-session-summary-{first}").leak())
                .is_none()
        );
    }

    #[test]
    fn unread_fades_ease_out_and_the_badge_pops_past_full_size() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let demote = UnreadFade {
            start,
            arriving: false,
        };
        assert_eq!(demote.look(start), 1.);
        assert!(demote.look(at(100)) < 0.5, "ease-out is past halfway");
        assert_eq!(demote.look(at(200)), 0.);
        assert!(!demote.running(at(200)));
        let arrive = UnreadFade {
            start,
            arriving: true,
        };
        assert!(arrive.badge(start).opacity < 1e-6);
        assert!(
            (0..280)
                .step_by(10)
                .any(|ms| arrive.badge(at(ms)).scale > 1.),
            "the badge overshoots as it pops in"
        );
        let end = arrive.badge(at(280));
        assert!((end.scale - 1.).abs() < 1e-6 && (end.opacity - 1.).abs() < 1e-6);
        assert!(arrive.running(at(250)) && !arrive.running(at(280)));
    }

    /// A project closes through its disclosure: its rows stay mounted in one
    /// clipped item while it animates. Reduced motion closes it at once, and
    /// a click on the header toggles it.
    #[gpui::test]
    fn project_children_animate_through_a_disclosure_or_toggle_at_once(cx: &mut TestAppContext) {
        let (window, mut visual, a, _) = tree_window(cx);
        let visual = &mut visual;
        let project = "/work/another-project";
        let key = project_key(project);
        let shown = |shell: &Shell, cx: &App| {
            let rows = shell.tree_rows(&shell.tree_catalog(cx), cx);
            (
                rows.iter()
                    .any(|row| matches!(row, TreeRow::Session { id, .. } if *id == a)),
                rows.iter()
                    .any(|row| matches!(row, TreeRow::Reveal { key: k, .. } if *k == key)),
            )
        };
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(shown(shell, cx), (true, false));
                cx.set_reduce_motion(false);
                shell.tree.projects.insert(project.into(), false);
                shell.tree_disclose(key.clone(), false, cx);
                assert_eq!(
                    shown(shell, cx),
                    (false, true),
                    "closing keeps the rows mounted"
                );
                assert!(shell.tree_animating());
                let openness = shell.tree.openness(&key, false, Instant::now());
                assert!(openness > 0. && openness <= 1., "{openness}");
                // Turning back mid-way under reduced motion lands open at once.
                cx.set_reduce_motion(true);
                shell.tree.projects.insert(project.into(), true);
                shell.tree_disclose(key.clone(), true, cx);
                assert_eq!(shown(shell, cx), (true, false));
                assert_eq!(shell.tree.openness(&key, true, Instant::now()), 1.);
            })
            .unwrap();
        visual.run_until_parked();

        let header = visual
            .debug_bounds(format!("tree-project-{project}").leak())
            .unwrap();
        visual.simulate_click(header.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(shell.tree.projects.get(project), Some(&false));
                assert_eq!(shown(shell, cx), (false, false));
                assert!(!shell.tree_animating());
            })
            .unwrap();
    }

    #[gpui::test]
    fn collapsing_projects_and_worktrees_keeps_child_rows_aligned(cx: &mut TestAppContext) {
        let (window, mut visual, main, topic) = tree_window(cx);
        let visual = &mut visual;
        let project = "/work/another-project";
        let copy = "/work/another-project-topic";
        let child = format!("sidebar-session-{main}").leak();
        window
            .update(visual, |shell, _, cx| {
                let summary = shell.session_summaries.get_mut(&topic).unwrap();
                summary.cwd = project.into();
                summary.has_messages = true;
                if let Some(view) = shell.sessions.get(&topic).cloned() {
                    view.update(cx, |view, _| {
                        view.summary.cwd = project.into();
                        view.summary.has_messages = true;
                    });
                }
                shell.session_worktrees.insert(topic, Some(copy.into()));
                shell.notify_sidebar(cx);
            })
            .unwrap();

        for (key, header) in [
            (worktree_key(project), "tree-worktree-/work/another-project"),
            (project_key(project), "tree-project-/work/another-project"),
        ] {
            visual.refresh().unwrap();
            let content = visual.debug_bounds(child).expect("open child row");
            window
                .update(visual, |_, _, cx| cx.set_reduce_motion(false))
                .unwrap();
            let header = visual.debug_bounds(header).expect("open parent header");
            visual.simulate_click(header.center(), Default::default());
            window
                .update(visual, |shell, _, cx| {
                    let disclosure = shell.tree.disclosures.get_mut(&key).expect("closing node");
                    // Hold the start of the transition while the first clipped row is laid out.
                    let start = Instant::now() + Duration::from_secs(30);
                    *disclosure = Disclosure::new(true);
                    disclosure.set(false, true, start);
                    shell.notify_sidebar(cx);
                })
                .unwrap();
            visual.refresh().unwrap();
            let closing = visual.debug_bounds(child).expect("closing child row");
            assert_eq!(closing.left(), content.left(), "{key}");
            assert_eq!(closing.right(), content.right(), "{key}");

            window
                .update(visual, |_, _, cx| cx.set_reduce_motion(true))
                .unwrap();
            visual.simulate_click(header.center(), Default::default());
        }
    }

    /// While a project is dragged its whole block follows the pointer on a
    /// lifted plate, and the block it passes steps aside by the dragged
    /// block's height. Dropping saves the order shared with Classic.
    #[gpui::test]
    fn dragging_a_project_block_steps_the_next_one_aside(cx: &mut TestAppContext) {
        use gpui::{Modifiers, MouseButton, point, px};
        let (window, mut visual, _, _) = tree_window(cx);
        let visual = &mut visual;
        let (order, blocks) = window
            .update(visual, |shell, _, _| {
                (
                    shell.workspace.projects.clone(),
                    shell.tree.drag_blocks(&RailDragKind::Project, |_| true),
                )
            })
            .unwrap();
        assert_eq!(
            blocks.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(),
            order
        );
        let (first, first_block) = blocks[0].clone();
        let (second, second_block) = blocks[1].clone();
        assert!(
            first_block.size.height > px(30.) || second_block.size.height > px(30.),
            "an open project carries its session rows in its block"
        );
        let header_selector = format!("tree-project-{first}").leak();
        let header = visual.debug_bounds(header_selector).unwrap();
        let from = header.center();
        let to = point(from.x, second_block.center().y + px(4.));
        visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(
            point(from.x, from.y + px(6.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert_eq!(
            visual.debug_bounds(header_selector).unwrap().center().y,
            to.y
        );
        window
            .update(visual, |shell, _, _| {
                assert_eq!(
                    shell.project_rail_project_offset(&second),
                    -first_block.size.height.as_f32(),
                    "the passed block steps up by the dragged block's height"
                );
                assert_eq!(
                    shell.project_rail_project_offset(&first),
                    (to.y - from.y).as_f32(),
                    "the header stays under the pointer past the last sibling"
                );
                assert!(shell.project_rail_project_raised(&first));
                let lift = shell
                    .tree_lift_bounds()
                    .expect("the dragged block is lifted");
                assert_eq!(lift.size.height, first_block.size.height);
            })
            .unwrap();
        visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.projects, [second.clone(), first.clone()]);
                assert_eq!(shell.project_rail_project_offset(&first), 0.);
                assert_eq!(shell.project_rail_project_offset(&second), 0.);
                assert!(shell.tree_lift_bounds().is_none());
            })
            .unwrap();
    }

    fn add_tree_drag_projects(shell: &mut Shell, cx: &mut Context<Shell>) {
        shell
            .workspace
            .projects
            .extend((0..35).map(|index| format!("/work/motion-project-{index:02}")));
        shell.notify_sidebar(cx);
    }

    #[gpui::test]
    fn a_project_whose_header_scrolled_above_the_tree_can_still_step_aside(
        cx: &mut TestAppContext,
    ) {
        use gpui::{Modifiers, MouseButton, point};

        let (window, mut visual, _, _) = tree_window(cx);
        let (first, second) = window
            .update(&mut visual, |shell, _, cx| {
                add_tree_drag_projects(shell, cx);
                let first = shell.workspace.projects[0].clone();
                for _ in 0..5 {
                    let mut session = locked(&shell.store)
                        .unwrap()
                        .create("codex", &first)
                        .unwrap();
                    session.has_messages = true;
                    shell.session_summaries.insert(session.id, session);
                }
                (first, shell.workspace.projects[1].clone())
            })
            .unwrap();
        visual.run_until_parked();
        let target = window
            .update(&mut visual, |shell, _, cx| {
                let head = shell
                    .tree
                    .rows
                    .iter()
                    .position(|row| {
                        matches!(row, TreeRow::Header(ProjectEntry::Project(id)) if id == &first)
                    })
                    .unwrap();
                shell.tree.list.scroll_to(ListOffset {
                    item_ix: head + 1,
                    offset_in_item: px(8.),
                });
                assert!(shell.tree.list.bounds_for_item(head).is_none());
                shell.notify_sidebar(cx);
                shell.tree.viewport_bounds().top() + px(2.)
            })
            .unwrap();
        visual.run_until_parked();
        let from = visual
            .debug_bounds(format!("tree-project-{second}").leak())
            .unwrap()
            .center();
        let to = point(from.x, target);
        visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        window
            .update(&mut visual, |shell, _, _| {
                assert!(shell.project_rail_project_offset(&first) > 0.);
            })
            .unwrap();
        visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        window
            .update(&mut visual, |shell, _, _| {
                assert_eq!(&shell.workspace.projects[..2], &[second, first]);
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_wheel_scroll_retargets_the_tree_drag_without_moving_the_grabbed_header(
        cx: &mut TestAppContext,
    ) {
        use gpui::{Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, point};

        let (window, mut visual, _, _) = tree_window(cx);
        window
            .update(&mut visual, |shell, _, cx| {
                add_tree_drag_projects(shell, cx)
            })
            .unwrap();
        visual.run_until_parked();
        let selector = "tree-project-/work/motion-project-02";
        let from = visual.debug_bounds(selector).unwrap().center();
        let to = point(from.x, from.y + px(6.));
        let read = |visual: &mut gpui::VisualTestContext| {
            window
                .update(visual, |shell, _, _| {
                    let scroll = -shell.tree.list.scroll_px_offset_for_scrollbar().y;
                    let offset = shell.project_rail_project_offset("/work/motion-project-02");
                    (scroll, offset)
                })
                .unwrap()
        };
        visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        let before = read(&mut visual);
        visual.simulate_event(ScrollWheelEvent {
            position: to,
            delta: ScrollDelta::Pixels(point(px(0.), px(-180.))),
            ..Default::default()
        });
        visual.run_until_parked();
        visual.refresh().unwrap();
        let after = read(&mut visual);
        assert_eq!(after.0 - before.0, px(180.));
        assert_eq!(after.1 - before.1, 180.);
        assert_eq!(visual.debug_bounds(selector).unwrap().center().y, to.y);
        visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        window
            .update(&mut visual, |shell, _, _| {
                let projects = &shell.workspace.projects;
                let moved = projects.iter().position(|id| id.ends_with("-02")).unwrap();
                assert!(projects[moved - 1].ends_with("-08"));
                assert!(projects[moved + 1].ends_with("-09"));
            })
            .unwrap();
    }

    #[gpui::test]
    fn dropping_a_tree_block_keeps_the_plate_on_the_first_frame(cx: &mut TestAppContext) {
        use gpui::{Modifiers, MouseButton, point};

        let (window, mut visual, _, _) = tree_window(cx);
        let from = visual
            .debug_bounds("tree-project-/work/another-project")
            .unwrap()
            .center();
        window
            .update(&mut visual, |_, _, cx| cx.set_reduce_motion(false))
            .unwrap();
        visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(
            point(from.x, from.y + px(8.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        visual.simulate_mouse_move(from, MouseButton::Left, Modifiers::default());
        let plate = window
            .update(&mut visual, |shell, _, _| shell.tree_lift_bounds().unwrap())
            .unwrap();
        visual.simulate_mouse_up(from, MouseButton::Left, Modifiers::default());
        window
            .update(&mut visual, |shell, _, _| {
                assert_eq!(shell.tree_lift_bounds(), Some(plate));
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_held_tree_drag_scrolls_at_both_edges_even_with_reduce_motion(cx: &mut TestAppContext) {
        use gpui::{Modifiers, MouseButton, point};

        let (window, mut visual, _, _) = tree_window(cx);
        window
            .update(&mut visual, |shell, _, cx| {
                add_tree_drag_projects(shell, cx)
            })
            .unwrap();
        visual.run_until_parked();
        let from = visual
            .debug_bounds("tree-project-/work/motion-project-00")
            .unwrap()
            .center();
        let viewport = window
            .update(&mut visual, |shell, _, _| shell.tree.viewport_bounds())
            .unwrap();
        let bottom = point(from.x, viewport.bottom() - px(2.));
        let top = point(from.x, viewport.top() + px(2.));
        visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(bottom, MouseButton::Left, Modifiers::default());
        let advance = |visual: &mut gpui::VisualTestContext| {
            window
                .update(visual, |shell, _, cx| {
                    let now = Instant::now();
                    shell.advance_tree_rail_frame(now, cx);
                    shell.advance_tree_rail_frame(now + Duration::from_millis(24), cx);
                    -shell.tree.list.scroll_px_offset_for_scrollbar().y
                })
                .unwrap()
        };
        let down = advance(&mut visual);
        assert!(down > px(0.));
        visual.simulate_mouse_move(top, MouseButton::Left, Modifiers::default());
        let up = advance(&mut visual);
        assert!(up < down);
        visual.simulate_mouse_up(top, MouseButton::Left, Modifiers::default());
        assert_eq!(advance(&mut visual), up);
    }

    #[gpui::test]
    fn tree_session_drag_reorders_siblings_without_opening_a_folder(cx: &mut TestAppContext) {
        use gpui::{Modifiers, MouseButton, point};

        let (window, mut visual, first, _) = tree_window(cx);
        let visual = &mut visual;
        let project = "/work/another-project";
        let second = window
            .update(visual, |shell, _, cx| {
                let mut session = locked(&shell.store)
                    .unwrap()
                    .create("codex", project)
                    .unwrap();
                session.has_messages = true;
                session.updated_at = shell
                    .session_metadata(first, cx)
                    .summary
                    .updated_at
                    .saturating_sub(1);
                let id = session.id;
                shell.session_summaries.insert(id, session);
                shell.notify_sidebar(cx);
                id
            })
            .unwrap();
        visual.run_until_parked();
        let source_selector = format!("sidebar-session-{first}").leak();
        let target_selector = format!("sidebar-session-{second}").leak();
        let source = visual.debug_bounds(source_selector).unwrap();
        let target = visual.debug_bounds(target_selector).unwrap();
        assert!(source.top() < target.top());
        let from = source.center();
        let to = point(from.x, target.center().y);
        visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        let scope = window
            .update(visual, |shell, _, _| {
                shell
                    .tree
                    .session_reorder_scope(first)
                    .expect("source is a reorderable session")
            })
            .unwrap();
        visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(
                    shell.tree_session_rail_owns_position(to),
                    "the pointer is in the source's vertical strip"
                );
                assert!(shell.tree_rail_held(), "the session's drag is active");
                assert_eq!(
                    shell.rail_item_offset(&first.to_string(), &RailDragKind::Session(scope)),
                    (to.y - from.y).as_f32()
                );
            })
            .unwrap();
        visual.refresh().unwrap();
        assert_eq!(
            visual.debug_bounds(source_selector).unwrap().center().y,
            to.y
        );
        assert!(visual.debug_bounds(target_selector).unwrap().top() < source.bottom());
        visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                let ids = shell.tree_session_ids(cx);
                let index = ids.iter().position(|id| *id == second).unwrap();
                assert_eq!(ids[index + 1], first);
                assert!(shell.project_groups_for(project).folders.is_empty());
            })
            .unwrap();

        let from = visual.debug_bounds(source_selector).unwrap().center();
        let target = visual.debug_bounds(target_selector).unwrap();
        let to = point(target.right() - px(16.), target.center().y);
        visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(shell.sidebar_drop == Some(session_groups::ListDrop::Session(second)));
                assert!(!shell.tree_rail_held());
            })
            .unwrap();
        visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                let groups = shell.project_groups_for(project);
                assert_eq!(groups.folders.len(), 1);
                assert_eq!(groups.folders[0].session_ids, [first, second]);
            })
            .unwrap();
    }

    #[gpui::test]
    fn dragging_a_tree_folder_moves_its_sessions_and_reorders_its_project(cx: &mut TestAppContext) {
        use gpui::{Modifiers, MouseButton, point};

        let (window, mut visual, first, _) = tree_window(cx);
        let visual = &mut visual;
        let project = "/work/another-project";
        window
            .update(visual, |shell, _, cx| {
                let mut second = locked(&shell.store)
                    .unwrap()
                    .create("codex", project)
                    .unwrap();
                second.has_messages = true;
                let id = second.id;
                shell.session_summaries.insert(id, second);
                shell.update_groups_for(project.into(), cx, |groups| {
                    groups.create_folder(&[id], "second".into());
                    groups.create_folder(&[first], "first".into());
                    assert!(!groups.folder("first").unwrap().collapsed);
                    groups.toggle_folder("second");
                });
            })
            .unwrap();
        visual.run_until_parked();
        let kind = RailDragKind::Folder(project.into());
        let blocks = window
            .update(visual, |shell, _, _| {
                shell.tree.drag_blocks(&kind, |_| true)
            })
            .unwrap();
        let [(first_id, first_block), (second_id, second_block)] = blocks.as_slice() else {
            panic!("expected two measured folder blocks");
        };
        assert_eq!([first_id.as_str(), second_id.as_str()], ["first", "second"]);
        assert!(first_block.size.height > second_block.size.height);
        let from = point(first_block.center().x, first_block.top() + px(10.));
        let to = point(from.x, second_block.top() + px(10.));
        visual.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert!(shell.rail_item_raised("first", &kind));
                assert_eq!(
                    shell.tree_lift_bounds().unwrap().size.height,
                    first_block.size.height
                );
                assert_eq!(
                    shell.rail_item_offset("second", &kind),
                    -first_block.size.height.as_f32()
                );
            })
            .unwrap();
        visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                let groups = shell.project_groups_for(project);
                assert_eq!(
                    groups
                        .folders
                        .iter()
                        .map(|folder| folder.id.as_str())
                        .collect::<Vec<_>>(),
                    ["second", "first"]
                );
                assert!(
                    shell
                        .project_groups_for("/work/monocode")
                        .folders
                        .is_empty()
                );
            })
            .unwrap();
    }

    #[test]
    fn needs_you_iteration_wraps_and_recovers_if_a_request_has_resolved() {
        assert_eq!(cycle(&[1, 3], Some(&3), 1), Some(&1));
        assert_eq!(cycle(&[1, 3], Some(&2), 1), Some(&1));
        assert_eq!(cycle(&[1, 3], None, -1), Some(&3));
        assert_eq!(cycle(&[] as &[i32], None, 1), None);
    }
}
