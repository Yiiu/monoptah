//! GPUI rows and the two presentations of the project tree.
use super::*;

#[derive(Default)]
struct LiftedTreeRows {
    start: usize,
    heights: Vec<Pixels>,
    overlays: Vec<AnyElement>,
}

impl Shell {
    pub(in crate::shell) fn tree_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // A held drag uses the same rows it measured on press. Activity and
        // disclosure updates reshape the list when the gesture is released.
        if !self.tree_rail_held() {
            self.tree_settle_unread(cx);
            let catalog = self.tree_catalog(cx);
            let rows = self.tree_rows(&catalog, cx);
            self.tree.counts = catalog.counts;
            self.tree.requests = catalog.requests;
            self.tree.replace_rows(rows);
        }
        self.tree.clear_anchors();
        let has_requests = !self.tree.requests.is_empty();
        self.tree.request_disclosure.set(
            has_requests,
            has_requests && !cx.reduce_motion(),
            Instant::now(),
        );
        let plate = self.tree_badge_plate();
        let lift = self.tree_lift_plate();
        let LiftedTreeRows {
            start,
            heights,
            overlays,
        } = self.tree_dragged_rows(plate, cx).unwrap_or_default();
        div()
            .id("project-tree-sidebar")
            .debug_selector(|| "project-tree-sidebar".into())
            .track_focus(&self.sidebar_focus)
            .key_context("SessionSidebar")
            .on_action(
                cx.listener(|this, _: &session_groups::RenameSidebarItem, window, cx| {
                    this.rename_sidebar_item(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_action(cx.listener(
                |this, _: &session_groups::DeleteSidebarSelection, window, cx| {
                    this.delete_sidebar_selection(window, cx);
                    cx.stop_propagation();
                },
            ))
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .when(self.peek_open(), |nav| nav.pr(px(34.)))
                    .child(self.sidebar_navigation(cx)),
            )
            .child(self.tree_request_queue(&self.tree.requests, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    // Before the list, so the plate is painted under the lifted rows.
                    .children(lift)
                    .children(overlays)
                    .child(
                        gpui::list(
                            self.tree.list.clone(),
                            cx.processor(move |this, index: usize, _, cx| {
                                if let Some(height) =
                                    index.checked_sub(start).and_then(|ix| heights.get(ix))
                                {
                                    return div().h(*height).into_any_element();
                                }
                                this.tree_list_row(index, plate, cx)
                            }),
                        )
                        .size_full(),
                    ),
            )
            .child(self.tree_footer(cx))
            .into_any()
    }

    pub(super) fn tree_badge_plate(&self) -> gpui::Hsla {
        if self.sidebar_peek.is_some() {
            floating_fill()
        } else {
            shell_fill()
        }
    }

    /// The tree's own way into Settings: Focused reached it through the project
    /// header's menu, which this layout replaces.
    fn tree_footer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_shrink_0()
            .h(px(36.))
            .px(px(6.))
            .flex()
            .items_center()
            .border_t_1()
            .border_color(content(0.04))
            .child(
                icon_button("tree-settings", "Settings", "settings")
                    .debug_selector(|| "tree-settings".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(crate::settings::Section::General, window, cx)
                    })),
            )
            .into_any()
    }

    /// The card a dragged block is lifted on: drawn above the blocks it
    /// passes and below its own rows, which are deferred after it.
    fn tree_lift_plate(&self) -> Option<AnyElement> {
        let lift = self.tree_lift_bounds()?;
        let viewport = self.tree.list.viewport_bounds();
        // The rows keep 6 px from the edges and 2 px below each.
        let plate = div()
            .debug_selector(|| "tree-lift-plate".into())
            .absolute()
            .left(lift.left() - viewport.left() + px(4.))
            .top(lift.top() - viewport.top() - px(2.))
            .w((lift.size.width - px(8.)).max(px(0.)))
            .h(lift.size.height + px(2.))
            .rounded(px(8.))
            .bg(floating_fill())
            .border_1()
            .border_color(content(0.08))
            .shadow(vec![
                gpui::BoxShadow::new(px(0.), px(12.), black(if is_light() { 0.16 } else { 0.5 }))
                    .blur_radius(px(28.))
                    .spread_radius(px(-6.)),
            ]);
        Some(gpui::deferred(plate).with_priority(1).into_any())
    }

    /// Draw held rows independently of their list slots, which can scroll
    /// beyond the virtual list's overdraw during a gesture.
    fn tree_dragged_rows(
        &mut self,
        plate: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> Option<LiftedTreeRows> {
        let (kind, id) = self.tree_dragged_block()?;
        let rows = self.tree.drag_block_rows(kind, id)?;
        let measurements: Vec<_> = rows
            .clone()
            .map(|ix| self.tree.list.estimated_bounds_for_item(ix))
            .collect::<Option<_>>()?;
        let lift = self.tree_lift_bounds()?;
        let viewport = self.tree.viewport_bounds();
        let offset = lift.top() - measurements.first()?.top();
        let mut overlays = Vec::new();
        for (ix, bounds) in rows.clone().zip(&measurements) {
            let top = bounds.top() + offset;
            if top + bounds.size.height < viewport.top() || top > viewport.bottom() {
                continue;
            }
            let row = self.tree_row(self.tree.rows[ix].clone(), plate, cx);
            let overlay = div()
                .absolute()
                .top(top - viewport.top())
                .left_0()
                .w_full()
                .child(row);
            overlays.push(gpui::deferred(overlay).with_priority(2).into_any_element());
        }
        Some(LiftedTreeRows {
            start: rows.start,
            heights: measurements.iter().map(|row| row.size.height).collect(),
            overlays,
        })
    }

    fn tree_list_row(
        &mut self,
        ix: usize,
        plate: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(row) = self.tree.rows.get(ix).cloned() else {
            return div().into_any();
        };
        let owner = self.tree.owners.get(ix).cloned().flatten();
        let row = self.tree_row(row, plate, cx);
        let Some(owner) = owner else {
            return row;
        };
        let mut offset = self.project_rail_project_offset(&owner.project);
        let mut raised = self.project_rail_project_raised(&owner.project);
        if let Some(folder) = &owner.folder {
            let kind = RailDragKind::Folder(owner.project);
            offset += self.rail_item_offset(folder, &kind);
            raised |= self.rail_item_raised(folder, &kind);
        }
        if let Some((scope, session)) = owner.session {
            let kind = RailDragKind::Session(scope);
            let item = session.to_string();
            offset += self.rail_item_offset(&item, &kind);
            raised |= self.rail_item_raised(&item, &kind);
        }
        let row = if offset == 0. {
            row
        } else {
            // GPUI lays out each list item as a root, so a relative inset
            // on the item itself does not move it. Keep the slot at rest
            // and position its child inside that slot, like a title tab.
            div()
                .child(div().relative().top(px(offset)).child(row))
                .into_any()
        };
        if raised {
            gpui::deferred(row).with_priority(1).into_any()
        } else {
            row
        }
    }

    /// A node's rows while it opens or closes, clipped by its disclosure.
    fn tree_reveal(
        &mut self,
        key: String,
        rows: Vec<TreeRow>,
        plate: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows: Vec<_> = rows
            .into_iter()
            .map(|row| self.tree_row(row, plate, cx))
            .collect();
        self.tree
            .disclosures
            .get(&key)
            .and_then(|d| d.render(div().flex().flex_col().children(rows), Instant::now()))
            .map_or_else(|| div().into_any(), IntoElement::into_any_element)
    }

    fn tree_row(&mut self, row: TreeRow, plate: gpui::Hsla, cx: &mut Context<Self>) -> AnyElement {
        // A session row's spacing comes from its rail step's own padding, which the
        // rail is drawn through; a gap outside it would break the line between rows.
        let rail_row = matches!(row, TreeRow::Session { .. });
        let row = match row {
            TreeRow::Reveal { key, rows } => return self.tree_reveal(key, rows, plate, cx),
            TreeRow::Monos => self.monos.clone().map_or_else(
                || div().into_any(),
                |monos| {
                    monos.update(cx, |monos, cx| {
                        monos.tree_rail(tree_section_heading("Monos"), cx)
                    })
                },
            ),
            TreeRow::Header(ProjectEntry::Heading(name)) => self.tree_heading(name, cx),
            TreeRow::Header(ProjectEntry::Group(group)) => self.tree_group_header(group, cx),
            TreeRow::Header(ProjectEntry::Project(path)) => self.tree_project_row(path, cx),
            TreeRow::Section {
                project,
                name,
                count,
                open,
            } => self.tree_section_row(project, name, count, open, cx),
            TreeRow::Folder {
                project,
                id,
                count,
                open,
            } => self.tree_folder_row(project, id, count, open, cx),
            TreeRow::Worktree {
                project,
                path,
                branch,
                open,
                counts,
            } => self.tree_worktree_row(project, path, branch, open, counts, cx),
            TreeRow::Session {
                id,
                depth,
                last,
                branch,
                children,
                orchestration,
                ..
            } => {
                let row = self.tree_session_row(id, branch, children, orchestration, plate, cx);
                div()
                    .ml(px((f32::from(depth) - 1.).max(0.) * 12.))
                    .child(rails::step(last, row))
                    .into_any()
            }
            TreeRow::Orchestration { id, represented } => {
                self.orchestration_summaries.get(&id).map_or_else(
                    || div().into_any(),
                    |summary| {
                        div()
                            .ml(px(22.))
                            .child(self.agents_excluding(id, summary, &represented, cx))
                            .into_any()
                    },
                )
            }
            TreeRow::Archived(count) => {
                let open = self
                    .tree
                    .archived_open
                    .unwrap_or(self.workspace.preferences.show_archived);
                tree_node(
                    "tree-archived",
                    format!("Archived {count}"),
                    "archive",
                    open,
                    self.tree.openness(ARCHIVED_KEY, open, Instant::now()),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tree.archived_open = Some(!open);
                    this.tree_disclose(ARCHIVED_KEY.into(), !open, cx);
                    this.notify_sidebar(cx);
                }))
                .into_any()
            }
        };
        div()
            .px(px(6.))
            .when(!rail_row, |d| d.pb(px(2.)))
            .child(row)
            .into_any()
    }

    fn tree_heading(&mut self, name: &'static str, cx: &mut Context<Self>) -> AnyElement {
        tree_section_heading(name)
            .when(name == "Projects", |heading| {
                heading.child(self.sidebar_filter_button(cx)).child(
                    icon_button("tree-project-add", "Add a project or group", "plus").on_click(
                        cx.listener(|this, e: &gpui::ClickEvent, window, cx| {
                            this.open_menu(
                                MenuTarget::ProjectRailAdd,
                                Anchor::Point(e.position()),
                                window,
                                cx,
                            );
                        }),
                    ),
                )
            })
            .into_any()
    }

    fn tree_group_header(&mut self, id: String, cx: &mut Context<Self>) -> AnyElement {
        let Some(group) = self
            .workspace
            .project_rail_groups
            .iter()
            .find(|g| g.id == id)
        else {
            return div().into_any();
        };
        let name = group.name.clone();
        let appearance = super::super::project_rail::project_group_appearance(group);
        let open = self.tree_group_open(&id);
        let active = self.workspace.project_rail_group(&self.workspace.project)
            == Some(id.as_str())
            && !self
                .workspace
                .project_rail_pinned
                .contains(&self.workspace.project)
            && !self.app_view_open()
            && !self.mono_active(cx);
        let key = group_key(&id);
        let lifted = self.tree_dragged_block() == Some((&RailDragKind::Group, id.as_str()));
        let offset = if lifted {
            0.
        } else {
            self.rail_item_offset(&id, &RailDragKind::Group)
        };
        let raised = !lifted && self.rail_item_raised(&id, &RailDragKind::Group);
        let anchor = self.project_rail_group_anchor(&id);
        self.tree.track_anchor(&anchor);
        let drag = id.clone();
        let menu = id.clone();
        let options = id.clone();
        let selector = format!("tree-group-{id}");
        let row = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .group("tree-group")
            .role(Role::Button)
            .aria_label(name.clone())
            .aria_expanded(open)
            .aria_selected(active)
            .relative()
            .top(px(offset))
            .h(px(30.))
            .px(px(6.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .when(active, |d| d.bg(content(0.10)))
            .hover(|d| d.bg(content(if active { 0.10 } else { 0.05 })))
            .child(
                tree_chevron(self.tree.openness(&key, open, Instant::now()), 12.)
                    .text_color(content(0.42)),
            )
            .child(crate::project_settings::project_icon(&appearance, &id, 12.))
            .child(tree_heading_text(&name))
            .child(
                icon_button("tree-group-options", "Group options", "more-horizontal")
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_menu(
                            MenuTarget::ProjectRailGroup(options.clone()),
                            Anchor::Point(e.position()),
                            window,
                            cx,
                        );
                    }))
                    .invisible()
                    .group_hover("tree-group", |d| d.visible()),
            )
            .child(anchor.probe())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    this.start_project_rail_drag(drag.clone(), true, e, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(
                        MenuTarget::ProjectRailGroup(menu.clone()),
                        Anchor::Point(e.position),
                        window,
                        cx,
                    );
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.tree_take_project_drag_click() {
                    return;
                }
                if let Some(group) = this
                    .workspace
                    .project_rail_groups
                    .iter_mut()
                    .find(|g| g.id == id)
                {
                    group.collapsed = !group.collapsed;
                }
                if !this.tree_filtered() {
                    this.tree_disclose(key.clone(), !open, cx);
                }
                this.persist(cx);
                this.notify_sidebar(cx);
            }));
        if raised {
            gpui::deferred(row).with_priority(1).into_any()
        } else {
            row.into_any()
        }
    }

    fn tree_project_row(&mut self, path: String, cx: &mut Context<Self>) -> AnyElement {
        let name = self.project_label(&path);
        let opened = self.tree_project_open(&path);
        let active =
            self.workspace.project == path && !self.app_view_open() && !self.mono_active(cx);
        let counts = self.tree.counts.get(&path).copied().unwrap_or_default();
        let open_path = path.clone();
        let drag_path = path.clone();
        let menu_path = path.clone();
        let options_path = path.clone();
        let new_path = path.clone();
        let bounds = self.tree_project_anchor(&path);
        self.tree.track_anchor(&bounds);
        let row = div()
            .id(SharedString::from(format!("tree-project-{path}")))
            .debug_selector({
                let path = path.clone();
                move || format!("tree-project-{path}")
            })
            .role(Role::Button)
            .aria_label(rollup_label(&name, counts))
            .aria_expanded(opened)
            .aria_selected(active)
            .group("tree-project")
            .relative()
            .h(px(30.))
            .px(px(6.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(content(0.92))
            .when(active, |d| d.bg(content(0.10)))
            .hover(|d| d.bg(content(if active { 0.10 } else { 0.05 })))
            .child(
                tree_chevron(
                    self.tree
                        .openness(&project_key(&path), opened, Instant::now()),
                    12.,
                )
                .text_color(content(0.42)),
            )
            .child(self.tree_project_avatar(&path, counts, 16., false))
            .child(div().flex_1().min_w_0().truncate().child(name))
            .child(counts_row(counts))
            .child(
                icon_button("tree-project-new-session", "New session", "plus")
                    .debug_selector({
                        let path = path.clone();
                        move || format!("tree-project-new-session-{path}")
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.tree_new_session(new_path.clone(), window, cx);
                    }))
                    .invisible()
                    .group_hover("tree-project", |d| d.visible()),
            )
            .child(
                icon_button("tree-project-options", "Project options", "more-horizontal")
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(
                        cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_menu(
                                MenuTarget::Project {
                                    path: options_path.clone(),
                                    rail: true,
                                },
                                Anchor::Point(event.position()),
                                window,
                                cx,
                            );
                        }),
                    )
                    .invisible()
                    .group_hover("tree-project", |d| d.visible()),
            )
            .child(bounds.probe())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    this.start_project_rail_drag(drag_path.clone(), false, e, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    this.open_menu(
                        MenuTarget::Project {
                            path: menu_path.clone(),
                            rail: true,
                        },
                        Anchor::Point(e.position),
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.tree_take_project_drag_click() {
                    return;
                }
                this.tree.projects.insert(open_path.clone(), !opened);
                // A filter holds every project open; nothing moves then.
                if !this.tree_filtered() {
                    this.tree_disclose(project_key(&open_path), !opened, cx);
                }
                this.notify_sidebar(cx);
            }));
        row.into_any()
    }

    fn tree_project_avatar(&self, path: &str, counts: Counts, side: f32, dot: bool) -> AnyElement {
        div()
            .relative()
            .size(px(side + 6.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .when(counts.running > 0, |d| {
                d.border_1()
                    .border_color(agent_running_color().opacity(0.72))
            })
            .child(if path.starts_with("remote://") {
                icon("globe", side).text_color(content(0.55)).into_any()
            } else {
                self.project_mark(path, side)
            })
            .when_some(counts.color().filter(|_| dot), |d, color| {
                d.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .right_0()
                        .size(px(7.))
                        .rounded_full()
                        .bg(color),
                )
            })
            .into_any()
    }

    fn tree_section_row(
        &self,
        project: String,
        name: &'static str,
        count: usize,
        open: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let glyph = if name == "Pinned" { "pin" } else { "clock" };
        let key = section_key(&project, name);
        tree_node(
            SharedString::from(format!("tree-section-{project}-{name}")),
            name,
            glyph,
            open,
            self.tree.openness(&key, open, Instant::now()),
        )
        .ml(px(14.))
        .child(tree_caption(count.to_string(), content(0.42)))
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this.tree_filtered() {
                this.tree_disclose(key.clone(), !open, cx);
                this.update_groups_for(project.clone(), cx, |groups| {
                    let collapsed = if name == "Pinned" {
                        &mut groups.pinned_collapsed
                    } else {
                        &mut groups.reminders_collapsed
                    };
                    *collapsed = !*collapsed;
                });
            }
        }))
        .into_any()
    }

    fn tree_folder_row(
        &mut self,
        project: String,
        id: String,
        count: usize,
        open: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let groups = self.project_groups_for(&project);
        let Some(folder) = groups.folder(&id) else {
            return div().into_any();
        };
        let name = folder.name.clone();
        let accent = session_groups::folder_accent(folder);
        let rename_target = session_groups::RenameTarget::Folder(id.clone());
        if self
            .renaming
            .as_ref()
            .and_then(|rename| rename.folder_project.as_deref())
            == Some(project.as_str())
            && let Some(renaming) = self.rename_row(&rename_target, cx)
        {
            return div().ml(px(14.)).child(renaming).into_any();
        }
        let bounds = self.folder_bounds.entry(id.clone()).or_default().clone();
        self.tree.track_anchor(&bounds);
        let menu_id = id.clone();
        let drag_id = id.clone();
        let menu_project = project.clone();
        let drag_project = project.clone();
        let key = folder_key(&project, &id);
        let turn = self.tree.openness(&key, open, Instant::now());
        tree_node(
            SharedString::from(format!("tree-folder-{project}-{id}")),
            name,
            "folder",
            open,
            turn,
        )
        .ml(px(14.))
        .relative()
        .when_some(accent, |d, color| d.text_color(color))
        .when(
            self.sidebar_drop == Some(session_groups::ListDrop::Folder(id.clone())),
            |d| d.bg(content(0.14)),
        )
        .child(tree_caption(count.to_string(), content(0.42)))
        .child(bounds.probe())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                this.start_tree_folder_drag(drag_project.clone(), drag_id.clone(), e, window, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                this.tree.folder_project = Some(menu_project.clone());
                this.open_menu(
                    MenuTarget::Folder(menu_id.clone()),
                    Anchor::Point(e.position),
                    window,
                    cx,
                );
            }),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if this.tree_take_project_drag_click() {
                return;
            }
            this.sidebar_target = Some(rename_target.clone());
            this.tree.folder_project = Some(project.clone());
            window.focus(&this.sidebar_focus, cx);
            if !this.tree_filtered() {
                this.tree_disclose(key.clone(), !open, cx);
                this.update_groups_for(project.clone(), cx, |groups| groups.toggle_folder(&id));
            }
        }))
        .into_any()
    }

    fn tree_worktree_row(
        &mut self,
        project: String,
        path: String,
        branch: Option<String>,
        open: bool,
        counts: Counts,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.queue_tree_diff(&path, cx);
        let diff = self.tree.diffs.display(&path);
        let key = worktree_key(&path);
        let title = branch.clone().unwrap_or_else(|| project_name(&path));
        let new_session = crate::worktree_switcher::WorktreeFocus {
            path: path.clone(),
            branch,
        };
        let selector = format!("tree-worktree-{path}");
        let plus_selector = format!("tree-worktree-new-session-{path}");
        tree_node(
            SharedString::from(selector.clone()),
            title,
            "git-branch",
            open,
            self.tree.openness(&key, open, Instant::now()),
        )
        .debug_selector(move || selector.clone())
        .group("tree-worktree")
        .ml(px(14.))
        .when_some(diff, |d, diff| d.child(tree_caption(diff, content(0.42))))
        .when_some(counts.color(), |d, color| d.child(tree_dot(color)))
        .child(
            icon_button("tree-worktree-new-session", "New session", "plus")
                .debug_selector(move || plus_selector.clone())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.tree_new_session_in_worktree(
                        project.clone(),
                        new_session.clone(),
                        window,
                        cx,
                    );
                }))
                .invisible()
                .group_hover("tree-worktree", |d| d.visible()),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this.tree.worktrees.remove(&path) {
                this.tree.worktrees.insert(path.clone());
            }
            if !this.tree_filtered() {
                this.tree_disclose(key.clone(), !open, cx);
            }
            this.notify_sidebar(cx);
        }))
        .into_any()
    }

    fn tree_ask(&self, id: i64, cx: &App) -> Option<String> {
        let Activity::NeedsYou { .. } = self.agent_status(id).activity else {
            return None;
        };
        self.notification_chat(id, cx)
            .and_then(|view| {
                view.read(cx)
                    .pending_notification_inputs(cx)
                    .into_iter()
                    .next()
            })
            .map(|input| input.label)
    }

    fn tree_request_queue(&self, ids: &[i64], cx: &mut Context<Self>) -> AnyElement {
        let queue = div().id("tree-needs-you").mx(px(6.)).mb(px(4.));
        let color = needs_you_color(ids.len());
        if ids.is_empty() {
            return queue
                .debug_selector(|| "tree-needs-you-empty".into())
                .h(px(28.))
                .px(px(6.))
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(color)
                .child("Nothing needs you")
                .into_any();
        }
        let summary = div()
            .id("tree-next-request")
            .debug_selector(|| "tree-next-request".into())
            .role(Role::Button)
            .aria_label(format!("Open next session needing you, {}", ids.len()))
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(28.))
            .px(px(6.))
            .rounded(px(6.))
            .text_color(color)
            .text_size(px(12.))
            .hover(|d| d.bg(content(0.05)))
            .child(icon("hand", 14.).text_color(color))
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Needs you {}", ids.len())),
            )
            .child(tree_caption("Ctrl J", content(0.45)))
            .on_click(cx.listener(|this, _, window, cx| this.open_next_tree_request(window, cx)));
        let list = div()
            .id("tree-request-list")
            .debug_selector(|| "tree-request-list".into())
            .max_h(px(118.))
            .overflow_y_scroll()
            .children(ids.iter().map(|id| self.tree_request_item(*id, cx)));
        queue
            .rounded(px(6.))
            .bg(color.opacity(0.055))
            .child(summary)
            .children(self.tree.request_disclosure.render(list, Instant::now()))
            .into_any()
    }

    fn tree_request_item(&self, id: i64, cx: &mut Context<Self>) -> AnyElement {
        let title = self
            .session_summaries
            .get(&id)
            .map(|_| self.session_metadata(id, cx).display_title())
            .or_else(|| {
                self.notification_chat(id, cx)
                    .map(|view| view.read(cx).display_title().to_owned())
            })
            .unwrap_or_else(|| "Agent session".into());
        let project = self
            .session_summaries
            .get(&id)
            .map(|s| self.project_label(self.project_of(id, &s.cwd)));
        let ask = self
            .tree_ask(id, cx)
            .unwrap_or_else(|| "Waiting for input".into());
        div()
            .id(("tree-request-session", id as usize))
            .role(Role::Button)
            .aria_label(format!("{title}, {ask}"))
            .px(px(6.))
            .py_1()
            .rounded(px(6.))
            .hover(|d| d.bg(content(0.05)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .child(tree_dot(warning_color()))
                    .child(div().flex_1().min_w_0().truncate().child(title))
                    .children(project.map(|name| tree_caption(name, content(0.45)))),
            )
            .child(
                div()
                    .pl(px(12.))
                    .text_size(px(11.))
                    .text_color(content(0.60))
                    .truncate()
                    .child(ask),
            )
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_tree_session(id, window, cx)),
            )
            .into_any()
    }

    fn tree_session_row(
        &mut self,
        id: i64,
        branch: Option<String>,
        children: usize,
        orchestration: bool,
        plate: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(row) = self.rename_row(&session_groups::RenameTarget::Session(id), cx) {
            return row;
        }
        let selected = self.selected_sessions.contains(&id);
        let active =
            self.workspace.active == Some(id) && !self.app_view_open() && !self.mono_active(cx);
        let hover = self.session_hover == Some(id);
        let bounds = self.session_row_bounds.entry(id).or_default().clone();
        self.tree.track_anchor(&bounds);
        let meta = self.session_metadata(id, cx);
        let status = self.agent_status(id);
        let now = now_ms();
        let needs_you = status.needs_you();
        // A finished row eases between its unread look and the read one, and
        // keeps the unread look while it is the open session even though it
        // already counts as seen.
        let (look, badge) = self.tree_unread_look(id, &status.activity);
        let unread = look > 0.;
        let shown_activity = match status.activity {
            Activity::Done { at, outcome, .. } => Activity::Done {
                at,
                outcome,
                unread,
            },
            ref activity => activity.clone(),
        };
        let read_activity = match status.activity {
            Activity::Done { at, outcome, .. } => Activity::Done {
                at,
                outcome,
                unread: false,
            },
            ref activity => activity.clone(),
        };
        let subline = if needs_you {
            Some(
                self.tree_ask(id, cx)
                    .unwrap_or_else(|| "Waiting for input".into()),
            )
        } else if unread {
            self.progress_summary(id).map(str::to_owned)
        } else {
            None
        };
        let trailing = match shown_activity {
            Activity::Done { at, .. } => {
                let color = mix(
                    crate::session_mark::activity_color(&read_activity),
                    crate::session_mark::activity_color(&shown_activity),
                    look,
                );
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            crate::session_mark::activity_glyph(&shown_activity, 12.)
                                .filter(|_| look >= 0.5)
                                .unwrap_or_else(|| icon("check", 12.)),
                        )
                        .child(tree_caption(monocode_core::time::relative(at, now), color))
                        .into_any(),
                )
            }
            Activity::NeedsYou { .. } => None,
            Activity::Idle => self
                .reminders
                .iter()
                .find(|reminder| reminder.session_id == id)
                .map(|r| {
                    tree_caption(session_groups::reminder_caption(r.due_at), warning_color())
                        .into_any()
                }),
            _ => status.label(now).map(|label| {
                tree_caption(label, crate::session_mark::activity_color(&status.activity))
                    .into_any()
            }),
        };
        let edge_fill = if active || selected {
            plate.blend(content(0.10))
        } else if hover {
            plate.blend(content(0.05))
        } else {
            plate
        };
        let avatar = crate::session_mark::provider_activity_icon(
            &meta.summary.provider,
            &shown_activity,
            crate::session_mark::ActivityIconSurface::Sidebar,
            edge_fill,
            cx.reduce_motion(),
            badge,
        );
        let mono = self
            .monos
            .as_ref()
            .and_then(|m| m.read(cx).tree_launch_mono(id))
            .map(|m| {
                let mono_id = m.id.clone();
                div()
                    .debug_selector(move || format!("tree-session-mono-{id}-{mono_id}"))
                    .flex_shrink_0()
                    .child(crate::monos::avatar::avatar(
                        &m.mascot, m.color, "idle", 10., true,
                    ))
            });
        let title = meta.display_title();
        let expanded = self.tree.sessions.contains(&id);
        let compact = !needs_you && !unread;
        let content_row = div()
            .h(px(24.))
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(5.))
            .child(avatar)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .font_weight(if look >= 0.5 {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(content(
                        if matches!(shown_activity, Activity::Done { .. }) {
                            0.58 + (0.92 - 0.58) * look
                        } else {
                            0.92
                        },
                    ))
                    .child(title.clone()),
            )
            .children(mono)
            .child(div().flex_1())
            .children(branch.map(|branch| tree_caption(branch, content(0.45))))
            .children(self.shell_task_chip(id))
            .children(if hover {
                Some(self.session_archive_button(id, &meta, cx))
            } else {
                trailing
            })
            .when(children > 0, |d| {
                d.child(
                    div()
                        .id(("tree-child-rail", id as usize))
                        .role(Role::Button)
                        .aria_expanded(expanded)
                        .aria_label(if orchestration {
                            "Orchestrated agents"
                        } else {
                            "Started sessions"
                        })
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap_px()
                        .rounded(px(6.))
                        .px_1()
                        .hover(|d| d.bg(content(0.08)))
                        .child(tree_chevron(
                            self.tree
                                .openness(&children_key(id), expanded, Instant::now()),
                            10.,
                        ))
                        .child(tree_caption(format!("{children}"), content(0.56)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            let open = !this.tree.sessions.remove(&id);
                            if open {
                                this.tree.sessions.insert(id);
                            }
                            this.tree_disclose(children_key(id), open, cx);
                            this.notify_sidebar(cx);
                        })),
                )
            });
        div()
            .id(("tree-session", id as usize))
            .debug_selector(move || format!("sidebar-session-{id}"))
            .role(Role::Button)
            .aria_label(title)
            .aria_selected(active || selected)
            .relative()
            .rounded(px(6.))
            .px(px(6.))
            .flex()
            .flex_col()
            .when(active || selected, |d| d.bg(content(0.10)))
            .hover(|d| d.bg(content(if active || selected { 0.10 } else { 0.05 })))
            .when(
                self.sidebar_drop == Some(session_groups::ListDrop::Session(id)),
                |d| d.bg(content(0.14)),
            )
            .child(content_row)
            .when(!compact, |d| {
                // The summary under an unread row grows and fades with its look.
                let summary = if needs_you { 1. } else { look };
                d.child(
                    div()
                        .debug_selector(move || format!("tree-session-summary-{id}"))
                        .pl(px(21.))
                        .h(px(17. * summary))
                        .opacity(summary)
                        .overflow_hidden()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(if needs_you {
                            warning_color()
                        } else {
                            content(0.55)
                        })
                        .child(subline.unwrap_or_default()),
                )
            })
            .child(bounds.probe())
            .on_hover(cx.listener(move |this, entered, window, cx| {
                if !this.tree_rail_held()
                    && !this.pane_drag.as_ref().is_some_and(|drag| drag.active)
                {
                    this.hover_session(id, bounds.clone(), *entered, compact, window, cx);
                }
            }))
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                if !std::mem::take(&mut this.skip_session_click) {
                    this.select_sidebar_session(id, e.modifiers(), window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.skip_session_click = false;
                    this.tab_drag = None;
                    this.start_tree_session_drag(id, event, window, cx);
                    this.pane_drag = Some(PaneDrag {
                        source: DragSource::Session(id),
                        start: event.position,
                        position: event.position,
                        active: false,
                        over: None,
                        insert: None,
                    });
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.context_selection = !this.selected_sessions.contains(&id);
                    if this.context_selection {
                        this.selected_sessions.clear();
                        this.selected_sessions.insert(id);
                    }
                    this.open_menu(
                        MenuTarget::Session(id),
                        Anchor::Point(event.position),
                        window,
                        cx,
                    );
                }),
            )
            .into_any()
    }

    pub(in crate::shell) fn tree_status_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // Opening another session demotes a lingering row even while the tree is
        // collapsed, so it does not replay when the tree comes back.
        self.tree_settle_unread(cx);
        let catalog = self.tree_catalog(cx);
        let requests = catalog.requests.len();
        let request_color = needs_you_color(requests);
        let monos = self
            .monos
            .clone()
            .map(|monos| monos.update(cx, |monos, cx| monos.project_navigation_rail(true, cx)));
        let entries = self.tree_project_entries(true);
        let navigation = self.tree_status_navigation(cx);
        div()
            .id("tree-status-bar")
            .debug_selector(|| "tree-status-bar".into())
            .w(px(COLLAPSED_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(shell_fill())
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .px(px(5.))
            .pt(px(4.))
            .border_r_1()
            .border_color(content(0.08))
            .child(
                icon_button("expand-project-tree", "Expand project tree", "sidebar").on_click(
                    cx.listener(|this, _, window, cx| this.toggle_primary_sidebar(window, cx)),
                ),
            )
            .children(navigation)
            .child(div().w_full().h_px().my_1().bg(content(0.08)))
            .child(
                div()
                    .id("tree-compact-needs-you")
                    .role(Role::Button)
                    .aria_label(format!("Needs you {requests}"))
                    .text_size(px(11.))
                    .text_color(request_color)
                    .rounded(px(6.))
                    .h(px(28.))
                    .px(px(3.))
                    .flex()
                    .items_center()
                    .gap_px()
                    .hover(|d| d.bg(content(0.05)))
                    .child(icon("hand", 13.).text_color(request_color))
                    .child(requests.to_string())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_next_tree_request(window, cx)),
                    ),
            )
            .child(
                div()
                    .id("tree-status-projects")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(2.))
                    .children(monos)
                    .children(entries.into_iter().map(|item| {
                        match item {
                            ProjectEntry::Heading(_) => {
                                div().h_px().w(px(20.)).my_1().bg(content(0.10)).into_any()
                            }
                            ProjectEntry::Group(id) => div()
                                .h(px(2.))
                                .w(px(16.))
                                .my_1()
                                .rounded_full()
                                .bg(self.tree_group_color(&id))
                                .into_any(),
                            ProjectEntry::Project(path) => {
                                let counts = catalog.counts.get(&path).copied().unwrap_or_default();
                                let label = rollup_label(&self.project_label(&path), counts);
                                let tooltip = label.clone();
                                div()
                                    .id(SharedString::from(format!("tree-status-project-{path}")))
                                    .role(Role::Button)
                                    .aria_label(label)
                                    .size(px(30.))
                                    .rounded(px(6.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .hover(|d| d.bg(content(0.05)))
                                    .child(self.tree_project_avatar(&path, counts, 16., true))
                                    .tooltip(move |_, cx| {
                                        cx.new(|_| project_rail::ProjectTooltip(tooltip.clone()))
                                            .into()
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.reveal_tree_project(path.clone(), cx);
                                        this.set_peek(true, cx);
                                    }))
                                    .into_any()
                            }
                        }
                    })),
            )
            .child(
                // Focused reached Settings through its project header, which Tree drops.
                icon_button("tree-status-settings", "Settings", "settings")
                    .debug_selector(|| "tree-status-settings".into())
                    .flex_shrink_0()
                    .mb(px(6.))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(crate::settings::Section::General, window, cx)
                    })),
            )
            .into_any()
    }

    fn tree_group_color(&self, id: &str) -> gpui::Hsla {
        self.workspace
            .project_rail_groups
            .iter()
            .find(|group| group.id == id)
            .and_then(|group| {
                group
                    .custom_color
                    .as_deref()
                    .and_then(|c| u32::from_str_radix(c.trim_start_matches('#'), 16).ok())
                    .or_else(|| {
                        group.color_index.and_then(|ix| {
                            monocode_core::project_appearance::COLORS.get(ix).copied()
                        })
                    })
            })
            .map_or(content(0.22), |color| rgb(color).into())
    }

    fn tree_status_navigation(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let workspace = &self.workspace;
        [
            ("Search", "search", workspace.sidebar_search_visible),
            ("Today", "clock", workspace.sidebar_today_visible),
            ("New session", "square-plus", true),
            ("Inbox", "inbox", workspace.sidebar_inbox_visible),
            (
                "Notes",
                "note",
                workspace.preferences.notes && workspace.sidebar_notes_visible,
            ),
            ("Automations", "zap", workspace.sidebar_automations_visible),
            (
                "Contributors",
                "chart-square",
                workspace.sidebar_contributors_visible,
            ),
        ]
        .into_iter()
        .filter(|(_, _, visible)| *visible)
        .map(|(label, glyph, _)| {
            let inbox_unread = label == "Inbox"
                && self
                    .inbox
                    .as_ref()
                    .is_some_and(|inbox| inbox.read(cx).unread_count() > 0);
            let count = match label {
                "Today" => self
                    .today
                    .as_ref()
                    .map_or(0, |today| today.read(cx).open_count()),
                "Notes" => self.note_count.unwrap_or(0),
                _ => 0,
            };
            icon_button(label, label, glyph)
                .relative()
                .when(inbox_unread, |d| {
                    d.child(
                        div()
                            .absolute()
                            .right_0()
                            .top_0()
                            .child(tree_dot(accent_color())),
                    )
                })
                .when(count > 0, |d| {
                    d.child(
                        div()
                            .absolute()
                            .right_0()
                            .top_0()
                            .bg(shell_fill())
                            .rounded_full()
                            .text_size(px(9.))
                            .px(px(2.))
                            .child(count.to_string()),
                    )
                })
                .on_click(cx.listener(move |this, _, window, cx| match label {
                    "Search" => this.focus_search(window, cx),
                    "Today" => this.open_today(window, cx),
                    "New session" => this.new_chat(cx),
                    "Inbox" => this.open_inbox(window, cx),
                    "Notes" => this.open_notes(window, cx),
                    "Automations" => this.open_automations(window, cx),
                    "Contributors" => this.open_contributors(None, None, window, cx),
                    _ => (),
                }))
                .into_any()
        })
        .collect()
    }
}

pub(super) fn needs_you_color(count: usize) -> gpui::Hsla {
    if count == 0 {
        content(0.4)
    } else {
        warning_color()
    }
}

fn tree_section_heading(name: &str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .min_h(px(26.))
        .pl(px(6.))
        .gap_1()
        .child(tree_heading_text(name))
}

fn tree_heading_text(name: &str) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(content(0.45))
        .child(name.to_uppercase())
}

fn tree_caption(text: impl Into<SharedString>, color: gpui::Hsla) -> gpui::Div {
    div()
        .min_w_0()
        .max_w(px(146.))
        .truncate()
        .text_size(px(11.))
        .text_color(color)
        .child(text.into())
}

fn tree_dot(color: gpui::Hsla) -> gpui::Div {
    div().size(px(6.)).rounded_full().bg(color).flex_shrink_0()
}

/// A disclosure chevron turned by how open its node is drawn: right when shut,
/// down when open, on the disclosure's curve in between.
fn tree_chevron(openness: f32, side: f32) -> gpui::Svg {
    icon("chevron-down", side).with_transformation(gpui::Transformation::rotate(gpui::radians(
        -std::f32::consts::FRAC_PI_2 * (1. - openness),
    )))
}

/// `from` eased towards `to` by `t`.
fn mix(from: gpui::Hsla, to: gpui::Hsla, t: f32) -> gpui::Hsla {
    let t = t.clamp(0., 1.);
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    // A grey ink has no hue of its own: keep the colour's all the way.
    let (h0, h1) = if from.s == 0. {
        (to.h, to.h)
    } else if to.s == 0. {
        (from.h, from.h)
    } else {
        (from.h, to.h)
    };
    gpui::hsla(
        lerp(h0, h1),
        lerp(from.s, to.s),
        lerp(from.l, to.l),
        lerp(from.a, to.a),
    )
}

fn tree_node(
    id: impl Into<gpui::ElementId>,
    title: impl Into<SharedString>,
    glyph: &'static str,
    open: bool,
    openness: f32,
) -> gpui::Stateful<gpui::Div> {
    let title = title.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(title.clone())
        .aria_expanded(open)
        .h(px(25.))
        .px(px(5.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(5.))
        .text_color(content(0.69))
        .text_size(px(12.))
        .hover(|d| d.bg(content(0.05)))
        .child(tree_chevron(openness, 10.).text_color(content(0.36)))
        .child(icon(glyph, 12.))
        .child(div().flex_1().min_w_0().truncate().child(title))
}

fn counts_row(counts: Counts) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(6.))
        .children(
            [
                (counts.needs_you, warning_color()),
                (counts.running, agent_running_color()),
                (counts.unread, agent_unread_color()),
            ]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, color)| {
                div()
                    .flex()
                    .gap_1()
                    .items_center()
                    .child(tree_dot(color))
                    .child(tree_caption(count.to_string(), content(0.56)))
            }),
        )
}

fn rollup_label(name: &str, counts: Counts) -> String {
    format!(
        "{name}, {} needs you, {} running, {} unread",
        counts.needs_you, counts.running, counts.unread
    )
}
